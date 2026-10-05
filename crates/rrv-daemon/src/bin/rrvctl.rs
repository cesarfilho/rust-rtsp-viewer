//! `rrvctl`: opera o `rrv-daemon` pelo socket (a mesma porta que a janela usa).
//!
//!   rrvctl status
//!   rrvctl record "Portão"      # liga/desliga a gravação (nome ou índice)
//!   rrvctl enable 2 / disable 2
//!   rrvctl zones "Portão"
//!   rrvctl events               # acompanha os eventos até Ctrl+C

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand};
use rrv_core::ipc::client::IpcClient;
use rrv_core::ipc::protocol::{CameraInfo, Request, Response};

#[derive(Parser)]
#[command(name = "rrvctl", version, about = "Opera o rrv-daemon pelo socket")]
struct Cli {
    /// Socket do daemon (padrão: `$RRV_SOCKET`, senão `$XDG_RUNTIME_DIR/rrv/rrv.sock`)
    #[arg(long, env = "RRV_SOCKET", global = true)]
    socket: Option<PathBuf>,

    /// Imprime JSON em vez de texto
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Estado de todas as câmeras
    Status,
    /// Liga ou desliga a gravação de uma câmera
    Record { camera: String },
    /// Liga uma câmera
    Enable { camera: String },
    /// Desliga uma câmera
    Disable { camera: String },
    /// Zonas de movimento de uma câmera
    Zones { camera: String },
    /// Acompanha os eventos (movimento, gravação, online/offline)
    Events,
    /// Exporta um clipe (.mp4, sem reencode) para a pasta exports/ das gravações
    Export {
        /// Nome da câmera
        camera: String,
        /// Início, `AAAA-MM-DD HH:MM:SS` (hora local) ou `-30m` / `-2h` (atrás de agora)
        #[arg(allow_hyphen_values = true)]
        from: String,
        /// Fim, no mesmo formato; padrão: agora
        #[arg(allow_hyphen_values = true)]
        to: Option<String>,
    },
    /// Gravações e eventos das últimas horas (do histórico do daemon)
    History {
        /// Nome da câmera (padrão: todas)
        camera: Option<String>,
        /// Quantas horas olhar para trás
        #[arg(long, default_value_t = 24)]
        hours: u32,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("rrvctl: {e}");
            ExitCode::FAILURE
        }
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// `-30m`, `-2h` (atrás de agora) ou `AAAA-MM-DD HH:MM:SS` (hora local) → Unix ms.
fn parse_time(text: &str, now: i64) -> Result<i64, String> {
    let t = text.trim();
    if let Some(rest) = t.strip_prefix('-') {
        let (num, unit) = rest.split_at(rest.len().saturating_sub(1));
        let n: i64 = num
            .parse()
            .map_err(|_| format!("tempo inválido: {text:?}"))?;
        let ms = match unit {
            "s" => 1_000,
            "m" => 60_000,
            "h" => 3_600_000,
            _ => return Err(format!("unidade inválida em {text:?} (use s, m ou h)")),
        };
        return Ok(now - n * ms);
    }
    use chrono::TimeZone;
    let naive = chrono::NaiveDateTime::parse_from_str(t, "%Y-%m-%d %H:%M:%S")
        .map_err(|_| format!("tempo inválido: {text:?} (use AAAA-MM-DD HH:MM:SS ou -30m)"))?;
    chrono::Local
        .from_local_datetime(&naive)
        .single()
        .map(|d| d.timestamp_millis())
        .ok_or_else(|| format!("horário ambíguo: {text:?}"))
}

/// `HH:MM:SS` local de um instante Unix em ms.
fn clock(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("%d/%m %H:%M:%S")
                .to_string()
        })
        .unwrap_or_else(|| "?".into())
}

fn cameras(c: &mut IpcClient) -> Result<Vec<CameraInfo>, String> {
    match c.request(&Request::Status)? {
        Response::Status { cameras } => Ok(cameras),
        Response::Error { message } => Err(message),
        other => Err(format!("resposta inesperada: {other:?}")),
    }
}

/// Aceita o índice ou o nome (sem diferenciar maiúsculas).
fn resolve(c: &mut IpcClient, who: &str) -> Result<usize, String> {
    let all = cameras(c)?;
    if let Ok(i) = who.parse::<usize>()
        && i < all.len()
    {
        return Ok(i);
    }
    all.iter()
        .find(|cam| cam.name.eq_ignore_ascii_case(who))
        .map(|cam| cam.index)
        .ok_or_else(|| {
            let names: Vec<_> = all.iter().map(|c| c.name.as_str()).collect();
            format!("câmera '{who}' não encontrada (há: {})", names.join(", "))
        })
}

fn expect_ok(r: Response) -> Result<(), String> {
    match r {
        Response::Ok => Ok(()),
        Response::Error { message } => Err(message),
        other => Err(format!("resposta inesperada: {other:?}")),
    }
}

fn run(cli: &Cli) -> Result<(), String> {
    let socket = cli
        .socket
        .clone()
        .unwrap_or_else(rrv_core::ipc::default_socket_path);
    let mut c = IpcClient::connect(&socket)?;
    match &cli.command {
        Command::Status => {
            let all = cameras(&mut c)?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&all).unwrap());
            } else {
                println!(
                    "{:<3} {:<24} {:<13} {:<6} gravando",
                    "#", "câmera", "estado", "stream"
                );
                for cam in all {
                    let decoder = match (&cam.decoder, cam.decoder_hw) {
                        (Some(d), true) => format!("{d} (GPU)"),
                        (Some(d), false) => format!("{d} (CPU)"),
                        (None, _) => "—".to_string(),
                    };
                    println!(
                        "{:<3} {:<24} {:<13} {:<6} {:<10} {:>4.0} {:>6} {:<16} {}{}",
                        cam.index,
                        cam.name,
                        cam.status,
                        cam.stream,
                        if cam.width > 0 {
                            format!("{}x{}", cam.width, cam.height)
                        } else {
                            "—".into()
                        },
                        cam.fps,
                        cam.bitrate_kbps,
                        decoder,
                        if cam.recording { "sim" } else { "não" },
                        if cam.motion { "  (movimento)" } else { "" }
                    );
                }
            }
            Ok(())
        }
        Command::Record { camera } => {
            let i = resolve(&mut c, camera)?;
            match c.request(&Request::ToggleRecording { camera: i })? {
                Response::Recording { recording, .. } => {
                    println!(
                        "{}",
                        if recording {
                            "gravando"
                        } else {
                            "parou de gravar"
                        }
                    );
                    Ok(())
                }
                Response::Error { message } => Err(message),
                other => Err(format!("resposta inesperada: {other:?}")),
            }
        }
        Command::Enable { camera } | Command::Disable { camera } => {
            let enabled = matches!(cli.command, Command::Enable { .. });
            let i = resolve(&mut c, camera)?;
            expect_ok(c.request(&Request::SetCameraEnabled { camera: i, enabled })?)
        }
        Command::Zones { camera } => {
            let i = resolve(&mut c, camera)?;
            match c.request(&Request::GetZones { camera: i })? {
                Response::Zones { zones, .. } => {
                    println!("{}", serde_json::to_string_pretty(&zones).unwrap());
                    Ok(())
                }
                Response::Error { message } => Err(message),
                other => Err(format!("resposta inesperada: {other:?}")),
            }
        }
        Command::Export { camera, from, to } => {
            let now = now_ms();
            let from_ms = parse_time(from, now)?;
            let to_ms = match to {
                Some(t) => parse_time(t, now)?,
                None => now,
            };
            match c.request(&Request::ExportClip {
                camera: camera.clone(),
                from_ms,
                to_ms,
            })? {
                Response::Exported { file, bytes } => {
                    println!("{file}  ({} KiB)", bytes / 1024);
                    Ok(())
                }
                Response::Error { message } => Err(message),
                other => Err(format!("resposta inesperada: {other:?}")),
            }
        }
        Command::History { camera, hours } => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_millis() as i64);
            let req = Request::History {
                camera: camera.clone(),
                from_ms: now - *hours as i64 * 3_600_000,
                to_ms: now,
            };
            match c.request(&req)? {
                Response::History {
                    segments,
                    events,
                    truncated,
                } => {
                    if cli.json {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&(&segments, &events)).unwrap()
                        );
                        return Ok(());
                    }
                    println!("# gravações ({})", segments.len());
                    for s in &segments {
                        let end = s.ts_end.map_or("gravando".to_string(), |e| {
                            format!("{:>5}s", (e - s.ts_start) / 1000)
                        });
                        println!(
                            "{}  {:<16} {} {:>8} KiB  {}{}",
                            clock(s.ts_start),
                            s.camera,
                            end,
                            s.bytes / 1024,
                            s.mode,
                            if s.has_motion { "  movimento" } else { "" }
                        );
                    }
                    println!("# eventos ({})", events.len());
                    for e in &events {
                        println!("{}  {:<16} {}", clock(e.ts), e.camera, e.kind);
                    }
                    if truncated {
                        println!("(resposta cortada no limite; reduza --hours)");
                    }
                    Ok(())
                }
                Response::Error { message } => Err(message),
                other => Err(format!("resposta inesperada: {other:?}")),
            }
        }
        Command::Events => {
            c.subscribe()?;
            loop {
                if let Some(ev) = c.next_event(Duration::from_secs(60))? {
                    if cli.json {
                        println!("{}", serde_json::to_string(&ev).unwrap());
                    } else {
                        println!(
                            "[{}] {}{}",
                            ev.name,
                            ev.kind.label(),
                            ev.detail.map(|d| format!(": {d}")).unwrap_or_default()
                        );
                    }
                }
            }
        }
    }
}
