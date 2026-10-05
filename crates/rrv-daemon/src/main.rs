//! `rrv-daemon`: o motor de vídeo sem janela (ADR 0010).
//!
//! Captura, reconecta, grava e detecta movimento com a janela fechada. Roda em
//! Docker (`docker stop` envia SIGTERM) ou como serviço de usuário. Ao receber
//! SIGTERM/SIGINT/SIGHUP finaliza as gravações em curso antes de sair: um
//! processo encerrado sem isso deixa um arquivo vazio e ilegível.

use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use clap::Parser;
use rrv_core::engine::{Engine, EngineEvent, EngineSettings, TICK_MS};
use rrv_core::startup::{load_config, merge_global_camera_defaults};

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
}

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
    let (config, warnings) = load_config(&cli.config, "rrv-daemon")?;
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
    let notify = config
        .notifications
        .map(|n| n.into_config())
        .unwrap_or_default();
    let logs = config.logs.unwrap_or_default();
    let view = config.view.unwrap_or_default();
    let zones = rrv_core::infrastructure::zone_state::load();

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
    log::info!(
        "rrv-daemon {}: {} câmera(s), gravação em {}",
        env!("CARGO_PKG_VERSION"),
        engine.camera_count(),
        recording.dir.display()
    );

    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    ctrlc::set_handler(move || flag.store(true, Ordering::SeqCst))
        .map_err(|e| format!("could not install the signal handler: {e}"))?;

    let period = Duration::from_millis(TICK_MS);
    let mut tick = 0u64;
    while !stop.load(Ordering::SeqCst) {
        let started = Instant::now();
        for event in engine.step(tick) {
            log_event(&engine, &event);
        }
        tick += 1;
        std::thread::sleep(period.saturating_sub(started.elapsed()));
    }

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
