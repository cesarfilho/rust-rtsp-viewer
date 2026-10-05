//! A conexão da janela com o daemon, mantida numa thread própria (spec
//! `ux-daemon.md`).
//!
//! A UI não toca no socket: manda [`LinkCommand`]s e recebe [`LinkEvent`]s por
//! canais, e os desenha no tick. A thread conecta, assina os eventos, consulta o
//! estado a cada segundo (o *heartbeat*: sem resposta em [`HEARTBEAT_TIMEOUT`]
//! o daemon é dado como perdido) e reconecta com espera crescente.
//!
//! Não há troca automática para o motor local ao perder o daemon: isso é uma
//! decisão da pessoa (o daemon pode ainda estar gravando).

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::client::{ConnectError, IpcClient};
use super::protocol::{CameraInfo, Request, Response, WireEvent};

/// De quanto em quanto tempo o estado é consultado.
pub const HEARTBEAT_EVERY: Duration = Duration::from_secs(1);
/// Sem resposta a um pedido por esse tempo, o daemon está perdido.
pub const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(5);
/// Espera entre tentativas de reconexão: 1, 2, 4, 8 e depois 15 s.
pub const BACKOFF_SECS: [u64; 5] = [1, 2, 4, 8, 15];

/// O que a thread conta à UI.
#[derive(Debug, Clone, PartialEq)]
pub enum LinkEvent {
    /// Tentando conectar (primeira vez ou depois de "Reconectar agora").
    Connecting,
    /// Conectado e com a assinatura de eventos ativa.
    Connected {
        server: String,
        cameras: Vec<CameraInfo>,
    },
    /// Estado atualizado (a cada [`HEARTBEAT_EVERY`]).
    Status(Vec<CameraInfo>),
    /// Um evento do motor, em tempo real.
    Event(WireEvent),
    /// Perdeu o contato; vai tentar de novo em `retry_in`.
    Lost { reason: String, retry_in: Duration },
    /// O daemon fala outra versão do protocolo. Não tenta de novo sozinha.
    Incompatible { message: String },
    /// O socket existe mas é de outro usuário. Não tenta de novo sozinha.
    NoPermission { message: String },
    /// Resposta a um [`LinkCommand::Request`].
    Reply {
        token: u64,
        result: Result<Response, String>,
    },
}

/// O que a UI pede à thread.
#[derive(Debug)]
pub enum LinkCommand {
    /// Faz um pedido ao daemon; a resposta volta como [`LinkEvent::Reply`].
    Request {
        token: u64,
        request: Request,
    },
    /// "Reconectar agora": tenta já, sem esperar o backoff.
    Reconnect,
    Shutdown,
}

pub struct DaemonLink {
    pub events: Receiver<LinkEvent>,
    commands: Sender<LinkCommand>,
    thread: Option<JoinHandle<()>>,
}

impl DaemonLink {
    pub fn spawn(socket: PathBuf) -> Self {
        let (ev_tx, events) = mpsc::channel();
        let (commands, cmd_rx) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("rrv-daemon-link".into())
            .spawn(move || run(socket, ev_tx, cmd_rx))
            .ok();
        Self {
            events,
            commands,
            thread,
        }
    }

    pub fn send(&self, command: LinkCommand) {
        let _ = self.commands.send(command);
    }

    /// Pede um pedido ao daemon. Devolve o token que identifica a resposta.
    pub fn request(&self, token: u64, request: Request) {
        self.send(LinkCommand::Request { token, request });
    }

    pub fn reconnect_now(&self) {
        self.send(LinkCommand::Reconnect);
    }
}

impl Drop for DaemonLink {
    fn drop(&mut self) {
        let _ = self.commands.send(LinkCommand::Shutdown);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

enum Next {
    Retry,
    Stop,
}

fn run(socket: PathBuf, ev: Sender<LinkEvent>, cmds: Receiver<LinkCommand>) {
    let mut attempt = 0usize;
    // Comandos que chegaram enquanto não havia conexão: a resposta é um erro
    // claro, não um silêncio.
    let refuse = |cmd: LinkCommand, ev: &Sender<LinkEvent>| {
        if let LinkCommand::Request { token, .. } = cmd {
            let _ = ev.send(LinkEvent::Reply {
                token,
                result: Err("sem conexão com o daemon".into()),
            });
        }
    };
    loop {
        let _ = ev.send(LinkEvent::Connecting);
        match IpcClient::connect_detailed(&socket) {
            Ok(mut client) => {
                attempt = 0;
                match serve(&mut client, &ev, &cmds) {
                    Next::Stop => return,
                    Next::Retry => {}
                }
            }
            Err(ConnectError::Incompatible(message)) => {
                let _ = ev.send(LinkEvent::Incompatible { message });
                if !wait_for_reconnect(&cmds, &ev, &refuse) {
                    return;
                }
                continue;
            }
            Err(ConnectError::PermissionDenied(message)) => {
                let _ = ev.send(LinkEvent::NoPermission { message });
                if !wait_for_reconnect(&cmds, &ev, &refuse) {
                    return;
                }
                continue;
            }
            Err(e) => {
                let retry_in =
                    Duration::from_secs(BACKOFF_SECS[attempt.min(BACKOFF_SECS.len() - 1)]);
                attempt += 1;
                let _ = ev.send(LinkEvent::Lost {
                    reason: e.to_string(),
                    retry_in,
                });
                if !sleep_or_reconnect(retry_in, &cmds, &ev, &refuse) {
                    return;
                }
                continue;
            }
        }
        // `serve` voltou com Retry: perdeu a conexão que tinha.
        let retry_in = Duration::from_secs(BACKOFF_SECS[0]);
        if !sleep_or_reconnect(retry_in, &cmds, &ev, &refuse) {
            return;
        }
    }
}

/// Espera até `d`, ou até a UI pedir "Reconectar agora" (`true`) ou encerrar
/// (`false`). Pedidos nesse meio-tempo são recusados com erro claro.
fn sleep_or_reconnect(
    d: Duration,
    cmds: &Receiver<LinkCommand>,
    ev: &Sender<LinkEvent>,
    refuse: &dyn Fn(LinkCommand, &Sender<LinkEvent>),
) -> bool {
    let until = Instant::now() + d;
    loop {
        let left = until.saturating_duration_since(Instant::now());
        match cmds.recv_timeout(left) {
            Ok(LinkCommand::Reconnect) | Err(RecvTimeoutError::Timeout) => return true,
            Ok(LinkCommand::Shutdown) | Err(RecvTimeoutError::Disconnected) => return false,
            Ok(other) => refuse(other, ev),
        }
    }
}

/// Espera só o "Reconectar agora" (sem backoff): para `Incompatible` e
/// `NoPermission`, onde tentar de novo sozinha não adianta.
fn wait_for_reconnect(
    cmds: &Receiver<LinkCommand>,
    ev: &Sender<LinkEvent>,
    refuse: &dyn Fn(LinkCommand, &Sender<LinkEvent>),
) -> bool {
    loop {
        match cmds.recv() {
            Ok(LinkCommand::Reconnect) => return true,
            Ok(LinkCommand::Shutdown) | Err(_) => return false,
            Ok(other) => refuse(other, ev),
        }
    }
}

/// Com a conexão de pé: assina, anuncia `Connected` e mantém o heartbeat até
/// perdê-la.
fn serve(client: &mut IpcClient, ev: &Sender<LinkEvent>, cmds: &Receiver<LinkCommand>) -> Next {
    if let Err(e) = client.subscribe() {
        let _ = ev.send(LinkEvent::Lost {
            reason: e,
            retry_in: Duration::from_secs(BACKOFF_SECS[0]),
        });
        return Next::Retry;
    }
    let cameras = match client.request_timeout(&Request::Status, HEARTBEAT_TIMEOUT) {
        Ok(Response::Status { cameras }) => cameras,
        Ok(other) => {
            let _ = ev.send(LinkEvent::Lost {
                reason: format!("resposta inesperada: {other:?}"),
                retry_in: Duration::from_secs(BACKOFF_SECS[0]),
            });
            return Next::Retry;
        }
        Err(e) => {
            let _ = ev.send(LinkEvent::Lost {
                reason: e,
                retry_in: Duration::from_secs(BACKOFF_SECS[0]),
            });
            return Next::Retry;
        }
    };
    let _ = ev.send(LinkEvent::Connected {
        server: client.server.clone(),
        cameras,
    });

    let mut next_beat = Instant::now() + HEARTBEAT_EVERY;
    loop {
        // Pedidos da UI.
        loop {
            match cmds.try_recv() {
                Ok(LinkCommand::Request { token, request }) => {
                    let result = client.request_timeout(&request, HEARTBEAT_TIMEOUT);
                    let lost = result.is_err();
                    let _ = ev.send(LinkEvent::Reply {
                        token,
                        result: result.clone(),
                    });
                    if lost {
                        let _ = ev.send(LinkEvent::Lost {
                            reason: result.err().unwrap_or_default(),
                            retry_in: Duration::from_secs(BACKOFF_SECS[0]),
                        });
                        return Next::Retry;
                    }
                }
                Ok(LinkCommand::Reconnect) => {}
                Ok(LinkCommand::Shutdown) | Err(mpsc::TryRecvError::Disconnected) => {
                    return Next::Stop;
                }
                Err(mpsc::TryRecvError::Empty) => break,
            }
        }
        // Eventos em tempo real (espera um pouco, para o laço não girar a seco).
        match client.next_event(Duration::from_millis(100)) {
            // Sem `continue`: com eventos chegando sem parar, o heartbeat abaixo
            // ainda precisa rodar.
            Ok(Some(e)) => {
                let _ = ev.send(LinkEvent::Event(e));
            }
            Ok(None) => {}
            Err(e) => {
                let _ = ev.send(LinkEvent::Lost {
                    reason: e,
                    retry_in: Duration::from_secs(BACKOFF_SECS[0]),
                });
                return Next::Retry;
            }
        }
        // Heartbeat.
        if Instant::now() >= next_beat {
            match client.request_timeout(&Request::Status, HEARTBEAT_TIMEOUT) {
                Ok(Response::Status { cameras }) => {
                    let _ = ev.send(LinkEvent::Status(cameras));
                }
                Ok(_) => {}
                Err(e) => {
                    let _ = ev.send(LinkEvent::Lost {
                        reason: e,
                        retry_in: Duration::from_secs(BACKOFF_SECS[0]),
                    });
                    return Next::Retry;
                }
            }
            next_beat = Instant::now() + HEARTBEAT_EVERY;
        }
    }
}
