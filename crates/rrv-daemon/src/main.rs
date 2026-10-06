//! `rrv-daemon`: o motor de vídeo sem janela (ADR 0010).
//!
//! Captura, reconecta, grava e detecta movimento com a janela fechada. Roda em
//! Docker (`docker stop` envia SIGTERM) ou como serviço de usuário. Ao receber
//! SIGTERM/SIGINT/SIGHUP finaliza as gravações em curso antes de sair: um
//! processo encerrado sem isso deixa um arquivo vazio e ilegível.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use clap::Parser;
use rrv_core::engine::{Engine, EngineEvent, EngineSettings, TICK_MS};
use rrv_core::infrastructure::store::StoreCmd;
use rrv_core::ipc::handler::Host;
use rrv_core::ipc::protocol::WireEvent;
use rrv_core::ipc::server::IpcServer;
use rrv_core::startup::{Secrets, load_config_with, merge_global_camera_defaults};

#[derive(Parser)]
#[command(
    name = "rrv-daemon",
    version,
    about = "Motor do rust-rtsp-viewer sem janela: grava e detecta com a janela fechada"
)]
struct Cli {
    /// Caminho do arquivo de configuração
    #[arg(default_value = "config.toml", env = "RRV_CONFIG")]
    config: String,

    /// Valida o arquivo de configuração e sai (0 = usável, 1 = erros)
    #[arg(long)]
    check: bool,

    /// Healthcheck: sai com 0 se o daemon em execução bateu o coração há pouco
    /// (para o `HEALTHCHECK` do Docker)
    #[arg(long)]
    health: bool,

    /// Arquivo de batimento do daemon
    #[arg(long, env = "RRV_HEALTH_FILE", default_value = DEFAULT_HEALTH_FILE)]
    health_file: PathBuf,

    /// Socket Unix da janela e do `rrvctl` (padrão: `$RRV_SOCKET`, senão
    /// `$XDG_RUNTIME_DIR/rrv/rrv.sock`)
    #[arg(long, env = "RRV_SOCKET")]
    socket: Option<PathBuf>,
}

const DEFAULT_HEALTH_FILE: &str = "/tmp/rrv-daemon.health";
/// O batimento é gravado a cada tick de 1 s; mais velho que isso = laço travado.
const HEALTH_MAX_AGE: Duration = Duration::from_secs(15);

fn main() -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let cli = Cli::parse();
    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("rrv-daemon: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &Cli) -> Result<(), String> {
    if cli.health {
        return check_health(&cli.health_file);
    }
    let (config, warnings) = load_config_with(
        &cli.config,
        "rrv-daemon",
        if cli.check {
            Secrets::Lenient
        } else {
            Secrets::Strict
        },
    )?;
    let mut cameras = config.cameras.clone().unwrap_or_default();
    merge_global_camera_defaults(&mut cameras, &config);
    if cli.check {
        println!(
            "{}: ok — {} câmera(s), {warnings} aviso(s)",
            cli.config,
            cameras.len()
        );
        return Ok(());
    }
    if cameras.is_empty() {
        return Err(format!("{}: nenhuma câmera configurada", cli.config));
    }
    gstreamer::init().map_err(|e| format!("GStreamer init failed: {e}"))?;

    let recording = config
        .recording
        .map(|r| r.into_config())
        .unwrap_or_default();
    let motion = config.motion.map(|m| m.into_config()).unwrap_or_default();
    let mut notify = config
        .notifications
        .map(|n| n.into_config())
        .unwrap_or_default();
    // O webhook é o aviso com a janela fechada. Configurá-lo liga a política de
    // aviso do motor (tipos de evento e cooldown de `[notifications]`); o
    // `enabled` de `[notifications]` continua valendo só para o desktop da janela.
    let webhook = config
        .webhook
        .map(|w| w.into_config())
        .transpose()?
        .flatten()
        .map(rrv_core::webhook::Webhook::spawn);
    if let Some(w) = &webhook {
        notify.enabled = true;
        log::info!("webhook ligado: {}", w.target());
    }
    let retention = config
        .retention
        .map(|r| r.into_config())
        .transpose()?
        .unwrap_or_default();
    let mqtt = config
        .mqtt
        .map(|m| m.into_config())
        .transpose()?
        .filter(|m| m.enabled);
    let detect = config
        .detect
        .map(|d| d.into_config())
        .transpose()?
        .unwrap_or_default();
    let daemon_cfg = config.daemon.clone().unwrap_or_default();
    let logs = config.logs.unwrap_or_default();
    let view = config.view.unwrap_or_default();
    let mut zones = rrv_core::infrastructure::zone_state::load();

    let mut engine = Engine::new(EngineSettings {
        cameras: &cameras,
        recording: &recording,
        motion,
        notify,
        logs: &logs,
        pause_hidden: view.pause_hidden(),
        stagger: Duration::from_millis(view.stagger_ms()),
        zones: &zones,
    });
    engine.set_headless();
    start_detection(&mut engine, detect);
    let db = rrv_core::infrastructure::view_state::state_dir().join("history.db");
    match rrv_core::infrastructure::store::StoreHandle::spawn(db.clone()) {
        Ok(store) => {
            log::info!("histórico em {}", db.display());
            engine.set_store(store);
        }
        // Sem histórico o daemon continua gravando: o vídeo vem antes do índice.
        Err(e) => log::warn!("sem histórico ({}): {e}", db.display()),
    }
    // Conexão só de leitura do laço principal (a thread do banco é a que escreve; o
    // WAL deixa as duas conviverem).
    let history = engine
        .store
        .as_ref()
        .and_then(|_| rrv_core::infrastructure::store::Store::open(&db).ok());
    log::info!(
        "rrv-daemon {}: {} câmera(s), gravação em {}",
        env!("CARGO_PKG_VERSION"),
        engine.camera_count(),
        recording.dir.display()
    );

    // O canal da janela e do `rrvctl`. Falhar aqui (por exemplo, outro daemon
    // já respondendo no socket) é fatal: dois daemons gravariam as mesmas câmeras.
    let socket = cli
        .socket
        .clone()
        .unwrap_or_else(rrv_core::ipc::default_socket_path);
    let server = IpcServer::bind(&socket)
        .map_err(|e| format!("não consegui abrir o socket {}: {e}", socket.display()))?;
    log::info!("socket de controle em {}", server.path().display());
    // Pela rede (a janela em outra máquina): só se `[daemon] listen` pedir, e sempre com token.
    if let Some(listen) = &daemon_cfg.listen {
        let addr: std::net::SocketAddr = listen
            .parse()
            .map_err(|e| format!("[daemon] listen '{listen}' inválido: {e}"))?;
        let token = daemon_cfg.token.as_deref().unwrap_or("");
        let bound = server
            .listen_tcp(addr, token)
            .map_err(|e| format!("não consegui escutar em {listen}: {e}"))?;
        log::info!(
            "canal pela rede em {bound} (autenticado por token; o tráfego NÃO é criptografado: só numa LAN de confiança)"
        );
    }

    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    ctrlc::set_handler(move || flag.store(true, Ordering::SeqCst))
        .map_err(|e| format!("could not install the signal handler: {e}"))?;

    let mqtt = mqtt.and_then(|cfg| {
        let target = rrv_core::mqtt::safe_target(&format!("mqtt://{}:{}", cfg.host, cfg.port));
        match rrv_core::mqtt::MqttPublisher::spawn(cfg, &engine.names) {
            Ok(p) => {
                log::info!("MQTT ligado: {target}");
                Some(p)
            }
            Err(e) => {
                log::warn!("MQTT desligado: {e}");
                None
            }
        }
    });

    let period = Duration::from_millis(TICK_MS);
    let mut tick = 0u64;
    let mut disk_watch = rrv_core::domain::retention::DiskWatch::default();
    while !stop.load(Ordering::SeqCst) {
        let started = Instant::now();
        // Pedidos da janela/rrvctl primeiro, depois o tick do motor.
        server.poll(&mut Host {
            engine: &mut engine,
            zones_file: &mut zones,
            persist: true,
            history: history.as_ref(),
            recordings: Some(recording.dir.as_path()),
        });
        // A cada minuto (e na partida): apaga o que passou da idade ou do limite de disco.
        if tick.is_multiple_of(RETENTION_EVERY_TICKS)
            && let Some(store) = &engine.store
        {
            store.send(StoreCmd::Retention {
                cfg: retention,
                dir: recording.dir.clone(),
            });
        }
        if tick.is_multiple_of(RETENTION_EVERY_TICKS)
            && let Some(usage) = rrv_core::infrastructure::disk::usage(&recording.dir)
            && let Some(t) = disk_watch.update(usage)
        {
            announce_disk(t, usage, &server, webhook.as_ref(), &engine);
        }
        let events = engine.step(tick);
        let now = unix_secs();
        let wire: Vec<WireEvent> = events
            .iter()
            .map(|e| {
                let name = engine.names.get(e.camera).map_or("?", String::as_str);
                WireEvent::from_engine(e, name, now)
            })
            .collect();
        server.publish(&wire);
        if let Some(w) = &webhook {
            for ev in &wire {
                w.send(ev);
            }
        }
        if let Some(m) = &mqtt {
            for ev in &wire {
                m.event(ev);
            }
            for i in 0..engine.camera_count() {
                m.update(i, &mqtt_state(&engine, i));
            }
        }
        for event in &events {
            log_event(&engine, event);
        }
        // Uma vez por segundo, o batimento que o healthcheck lê.
        if tick.is_multiple_of(10) {
            write_heartbeat(&cli.health_file, &engine);
        }
        tick += 1;
        std::thread::sleep(period.saturating_sub(started.elapsed()));
    }

    if let Some(m) = mqtt {
        m.shutdown();
    }
    let _ = std::fs::remove_file(&cli.health_file);
    log::info!("sinal de parada recebido: finalizando gravações");
    engine.shutdown();
    log::info!("rrv-daemon encerrado");
    Ok(())
}

/// O host do motor entrega os eventos: aqui só registra no log. O webhook/MQTT
/// de saída (plano 2.5.11) entra neste ponto, com o `notification` que a
/// política do motor já decidiu.
/// Avisa que o disco das gravações ficou quase cheio (ou voltou ao normal): log, webhook,
/// janela (toast) e o histórico. Uma vez por mudança de estado.
fn announce_disk(
    t: rrv_core::domain::retention::DiskTransition,
    usage: rrv_core::domain::retention::DiskUsage,
    server: &IpcServer,
    webhook: Option<&rrv_core::webhook::Webhook>,
    engine: &Engine,
) {
    use rrv_core::domain::retention::DiskTransition::{BecameLow, Recovered};
    let gib = |b: u64| b as f64 / (1u64 << 30) as f64;
    let free = usage.total.saturating_sub(usage.used);
    let (detail, notification) = match t {
        BecameLow { free_percent } => {
            let d = format!("{free_percent}% livre ({:.1} GiB)", gib(free));
            log::warn!("disco das gravações quase cheio: {d}");
            (
                d.clone(),
                Some((
                    "Disco quase cheio".to_string(),
                    format!("Restam {d}. A retenção apaga o mais antigo; aumente o espaço."),
                )),
            )
        }
        Recovered { free_percent } => {
            log::info!("disco das gravações voltou ao normal: {free_percent}% livre");
            (format!("{free_percent}% livre"), None)
        }
    };
    let wire = WireEvent {
        camera: 0,
        name: "Disco".into(),
        kind: rrv_core::domain::timeline::EventType::DiskLow,
        detail: Some(detail.clone()),
        notification,
        unix_secs: unix_secs(),
    };
    server.publish(std::slice::from_ref(&wire));
    if let Some(w) = webhook {
        w.send(&wire);
    }
    if let Some(store) = &engine.store {
        store.send(StoreCmd::Event {
            camera: "Disco".into(),
            ts: chrono::Utc::now().timestamp_millis(),
            kind: "disk_low".into(),
            label: detail,
            score: None,
            bbox: None,
            zone: None,
        });
    }
}

/// A retenção roda a cada minuto.
const RETENTION_EVERY_TICKS: u64 = 600;

fn log_event(engine: &Engine, event: &EngineEvent) {
    let name = engine.names.get(event.camera).map_or("?", String::as_str);
    match &event.detail {
        Some(d) => log::info!("[{name}] {}: {d}", event.kind.label()),
        None => log::info!("[{name}] {}", event.kind.label()),
    }
    if let Some((title, body)) = &event.notification {
        log::info!("[{name}] notificação: {title} — {body}");
    }
}

fn unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// `<segundos unix> <câmeras> <ao vivo>`: o healthcheck só olha a hora; o resto
/// é para quem abrir o arquivo.
fn write_heartbeat(path: &Path, engine: &Engine) {
    use rrv_core::domain::camera_status::CameraStatus;
    let live = engine
        .status
        .iter()
        .filter(|s| matches!(s, CameraStatus::Live | CameraStatus::Recording))
        .count();
    let line = format!("{} {} {live}\n", unix_secs(), engine.camera_count());
    // Grava num temporário e renomeia: o healthcheck nunca lê pela metade.
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, line).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

/// Saudável = o laço do daemon bateu o coração há menos de [`HEALTH_MAX_AGE`].
/// Uma câmera fora do ar não torna o contêiner doente: reiniciá-lo não a
/// consertaria, e perderia as gravações das outras.
fn check_health(path: &Path) -> Result<(), String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("sem batimento em {}: {e}", path.display()))?;
    let beat: u64 = text
        .split_whitespace()
        .next()
        .and_then(|t| t.parse().ok())
        .ok_or_else(|| format!("batimento ilegível em {}", path.display()))?;
    let age = unix_secs().saturating_sub(beat);
    if age > HEALTH_MAX_AGE.as_secs() {
        return Err(format!("o laço do daemon parou há {age} s"));
    }
    Ok(())
}

/// Liga a detecção de objetos quando `[detect] enabled` e o binário tem a feature `detect`.
/// Qualquer falha (modelo ausente, biblioteca não encontrada, CUDA indisponível sem recuo)
/// desliga só a detecção: o vídeo vem antes.
#[cfg(feature = "detect")]
fn start_detection(engine: &mut Engine, cfg: rrv_core::domain::detect::DetectConfig) {
    use rrv_core::domain::detect::BackendChoice;
    use rrv_core::engine::inference::InferenceWorker;
    use rrv_core::infrastructure::detector::{Backend, Detector};

    if !cfg.enabled {
        return;
    }
    let load = |backend| Detector::load(&cfg.model, backend);
    let detector = match cfg.backend {
        BackendChoice::Cpu => load(Backend::Cpu),
        BackendChoice::Cuda => load(Backend::Cuda),
        BackendChoice::Auto => load(Backend::Cuda).or_else(|e| {
            log::info!("detecção: CUDA indisponível ({e}); usando a CPU");
            load(Backend::Cpu)
        }),
    };
    match detector {
        Ok(d) => {
            log::info!(
                "detecção ligada: {} ({} px)",
                cfg.model.display(),
                d.input_size()
            );
            let d = d.with_thresholds(cfg.min_score, cfg.iou);
            engine.set_detect_policy(cfg.labels.clone(), cfg.cooldown_secs);
            engine.set_inference(InferenceWorker::start(d, engine.camera_count().max(2)));
        }
        Err(e) => log::warn!("detecção desligada: {e}"),
    }
}

#[cfg(not(feature = "detect"))]
fn start_detection(_engine: &mut Engine, cfg: rrv_core::domain::detect::DetectConfig) {
    if cfg.enabled {
        log::warn!("[detect] enabled, mas este binário foi feito sem a feature `detect`");
    }
}

/// O que o MQTT publica de uma câmera, lido do motor.
fn mqtt_state(engine: &Engine, i: usize) -> rrv_core::mqtt::CameraState {
    use rrv_core::domain::camera_status::CameraStatus;
    let status = &engine.status[i];
    let mut objects: Vec<String> = engine.detections[i]
        .iter()
        .map(|d| d.label().to_string())
        .collect();
    objects.sort();
    objects.dedup();
    rrv_core::mqtt::CameraState {
        status: rrv_core::ipc::handler::status_name(status).into(),
        online: matches!(status, CameraStatus::Live | CameraStatus::Recording),
        recording: *status == CameraStatus::Recording,
        motion: engine.motion_active[i],
        objects,
    }
}
