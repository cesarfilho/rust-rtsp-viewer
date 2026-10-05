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
    let db = rrv_core::infrastructure::view_state::state_dir().join("history.db");
    match rrv_core::infrastructure::store::StoreHandle::spawn(db.clone()) {
        Ok(store) => {
            log::info!("histórico em {}", db.display());
            engine.set_store(store);
        }
        // Sem histórico o daemon continua gravando: o vídeo vem antes do índice.
        Err(e) => log::warn!("sem histórico ({}): {e}", db.display()),
    }
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

    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    ctrlc::set_handler(move || flag.store(true, Ordering::SeqCst))
        .map_err(|e| format!("could not install the signal handler: {e}"))?;

    let period = Duration::from_millis(TICK_MS);
    let mut tick = 0u64;
    while !stop.load(Ordering::SeqCst) {
        let started = Instant::now();
        // Pedidos da janela/rrvctl primeiro, depois o tick do motor.
        server.poll(&mut Host {
            engine: &mut engine,
            zones_file: &mut zones,
            persist: true,
        });
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

    let _ = std::fs::remove_file(&cli.health_file);
    log::info!("sinal de parada recebido: finalizando gravações");
    engine.shutdown();
    log::info!("rrv-daemon encerrado");
    Ok(())
}

/// O host do motor entrega os eventos: aqui só registra no log. O webhook/MQTT
/// de saída (plano 2.5.11) entra neste ponto, com o `notification` que a
/// política do motor já decidiu.
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
