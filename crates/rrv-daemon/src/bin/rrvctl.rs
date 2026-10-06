//! `rrvctl`: opera o `rrv-daemon` pelo socket (a mesma porta que a janela usa).
//!
//!   rrvctl status
//!   rrvctl record "Portão"      # liga/desliga a gravação (nome ou índice)
//!   rrvctl enable 2 / disable 2
//!   rrvctl zones "Portão"
//!   rrvctl events               # acompanha os eventos até Ctrl+C
//!   rrvctl discover             # câmeras ONVIF na rede (não precisa do daemon)
//!   rrvctl secret set cam_portao_password   # guarda a senha no chaveiro do sistema

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand};
use rrv_core::ipc::client::IpcClient;
use rrv_core::ipc::protocol::{CameraInfo, Request, Response};

#[derive(Parser)]
#[command(name = "rrvctl", version, about = "Opera o rrv-daemon pelo socket")]
struct Cli {
    /// Socket do daemon, ou `tcp://host:porta` para um daemon em outra máquina (token: `RRV_TOKEN` ou o
    /// segredo `rrv_token` do chaveiro). Padrão: `$RRV_SOCKET`, senão `$XDG_RUNTIME_DIR/rrv/rrv.sock`
    #[arg(long, env = "RRV_SOCKET", global = true)]
    socket: Option<PathBuf>,

    /// Imprime JSON em vez de texto
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum SecretAction {
    /// Guarda a senha de NOME. Sem terminal, lê a senha da entrada padrão (`printf %s "$S" | rrvctl secret set NOME`)
    Set { name: String },
    /// Diz se NOME existe (nunca imprime o valor)
    Check { name: String },
    /// Apaga NOME
    Delete { name: String },
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
    /// Senhas no chaveiro do sistema (Secret Service), que a janela e o daemon leem por `${NOME}`
    Secret {
        #[command(subcommand)]
        action: SecretAction,
    },
    /// Procura câmeras ONVIF na rede (não precisa do daemon). Com `--user` (e a senha na variável de
    /// ambiente `--password-env`) pergunta a cada câmera os streams e imprime o trecho do `config.toml`;
    /// a senha nunca é impressa nem vai para o trecho (ele traz `${SEGREDO}`).
    Discover {
        /// Usuário da câmera
        #[arg(long)]
        user: Option<String>,
        /// Variável de ambiente que guarda a senha (padrão: ONVIF_PASSWORD)
        #[arg(long, default_value = "ONVIF_PASSWORD")]
        password_env: String,
        /// Quantos segundos esperar as respostas
        #[arg(long, default_value_t = 4)]
        wait: u64,
        /// Guarda a senha no chaveiro com o nome que o trecho do config usa (`${cam_…_password}`)
        #[arg(long)]
        store_secret: bool,
    },
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
        /// Só os eventos com este rótulo (ex.: `person`, `car`); as gravações não são filtradas
        #[arg(long)]
        label: Option<String>,
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

fn discover(
    user: Option<&str>,
    password_env: &str,
    wait: u64,
    store_secret: bool,
    json: bool,
) -> Result<(), String> {
    use rrv_core::onvif;
    let found = onvif::discover(Duration::from_secs(wait.clamp(1, 30)));
    if found.is_empty() {
        println!("nenhuma câmera ONVIF respondeu (ONVIF ligado na câmera? mesma rede?)");
        return Ok(());
    }
    let password = user.map(|_| std::env::var(password_env).unwrap_or_default());
    let mut report = Vec::new();
    for f in &found {
        let label = format!(
            "{}  {}  {}",
            f.ip,
            f.name.as_deref().unwrap_or("?"),
            f.hardware.as_deref().unwrap_or("")
        );
        let (Some(u), Some(p)) = (user, password.as_deref()) else {
            report.push((label, None, f.xaddr.clone()));
            continue;
        };
        if p.is_empty() {
            return Err(format!(
                "a variável {password_env} está vazia ou não existe"
            ));
        }
        let outcome = onvif::inspect(f, u, p).and_then(|i| {
            let snippet = onvif::config_snippet(f, u, &i);
            if store_secret {
                let name = onvif::secret_name_for(f, &i);
                rrv_core::secrets::keyring::store(&name, p)?;
                eprintln!("senha guardada no chaveiro como `{name}`");
            }
            Ok(snippet)
        });
        report.push((label, Some(outcome), f.xaddr.clone()));
    }
    if json {
        let v: Vec<_> = report
            .iter()
            .map(|(l, o, x)| {
                serde_json::json!({
                    "camera": l,
                    "xaddr": x,
                    "config": o.as_ref().and_then(|r| r.as_ref().ok()),
                    "error": o.as_ref().and_then(|r| r.as_ref().err()),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&v).unwrap());
        return Ok(());
    }
    println!("# {} câmera(s) ONVIF", report.len());
    for (label, outcome, xaddr) in report {
        println!("\n{label}\n  {xaddr}");
        match outcome {
            None => {}
            Some(Ok(snippet)) => println!("\n{snippet}"),
            Some(Err(e)) => println!("  não consegui ler os streams: {e}"),
        }
    }
    if user.is_none() {
        println!(
            "\nPara ver os streams e gerar o trecho do config.toml:\n  ONVIF_PASSWORD=… rrvctl discover --user admin"
        );
    }
    Ok(())
}

/// A senha de um terminal sem eco, ou de toda a entrada padrão quando não é um terminal.
fn read_password(prompt: &str) -> Result<String, String> {
    use std::io::{IsTerminal, Read, Write};
    let stdin = std::io::stdin();
    let mut line = String::new();
    if stdin.is_terminal() {
        eprint!("{prompt}");
        let _ = std::io::stderr().flush();
        let _ = std::process::Command::new("stty").arg("-echo").status();
        let r = stdin.read_line(&mut line);
        let _ = std::process::Command::new("stty").arg("echo").status();
        eprintln!();
        r.map_err(|e| e.to_string())?;
    } else {
        stdin
            .lock()
            .read_to_string(&mut line)
            .map_err(|e| e.to_string())?;
    }
    Ok(line.trim_end_matches(['\n', '\r']).to_string())
}

fn secret(action: &SecretAction) -> Result<(), String> {
    use rrv_core::secrets::keyring;
    match action {
        SecretAction::Set { name } => {
            let value = read_password(&format!("Senha para {name}: "))?;
            keyring::store(name, &value)?;
            println!("guardada no chaveiro como `{name}`; use ${{{name}}} na URL da câmera");
        }
        SecretAction::Check { name } => {
            if keyring::exists(name) {
                println!("`{name}` existe no chaveiro");
            } else {
                return Err(format!("`{name}` não existe no chaveiro"));
            }
        }
        SecretAction::Delete { name } => {
            keyring::clear(name)?;
            println!("`{name}` apagada do chaveiro");
        }
    }
    Ok(())
}

fn run(cli: &Cli) -> Result<(), String> {
    if let Command::Discover {
        user,
        password_env,
        wait,
        store_secret,
    } = &cli.command
    {
        return discover(
            user.as_deref(),
            password_env,
            *wait,
            *store_secret,
            cli.json,
        );
    }
    if let Command::Secret { action } = &cli.command {
        return secret(action);
    }
    let socket = cli
        .socket
        .clone()
        .unwrap_or_else(rrv_core::ipc::default_socket_path);
    // `--socket tcp://host:porta` fala com um daemon em outra máquina (token em RRV_TOKEN ou no chaveiro).
    let token = rrv_core::ipc::client_token();
    let mut c = IpcClient::connect_target(&socket, token.as_deref()).map_err(|e| e.to_string())?;
    match &cli.command {
        Command::Discover { .. } | Command::Secret { .. } => {
            unreachable!("tratado antes de conectar")
        }
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
        Command::History {
            camera,
            hours,
            label,
        } => {
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
                    mut events,
                    truncated,
                } => {
                    if let Some(l) = label {
                        events.retain(|e| e.label.eq_ignore_ascii_case(l));
                    }
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
                        let what = match (e.label.as_str(), e.score) {
                            ("", _) => String::new(),
                            (l, Some(s)) => format!("  {l} {:.0}%", s * 100.0),
                            (l, None) => format!("  {l}"),
                        };
                        let zone = e
                            .zone
                            .as_deref()
                            .map_or(String::new(), |z| format!("  [{z}]"));
                        println!("{}  {:<16} {}{what}{zone}", clock(e.ts), e.camera, e.kind);
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
