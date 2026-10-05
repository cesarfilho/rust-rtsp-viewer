//! O socket do daemon.
//!
//! Uma thread aceita conexões e cada conexão tem a sua. As threads de conexão
//! **não tocam no motor**: repassam os pedidos ao laço principal por um canal e
//! esperam a resposta, porque o `Engine` é de uma thread só. O laço principal
//! chama [`IpcServer::poll`] a cada tick e [`IpcServer::publish`] com os
//! eventos novos.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::handler::{self, Host};
use super::protocol::{Request, Response, ServerMessage, WireEvent, decode_line, encode_line};

/// Uma linha maior que isso não é do protocolo: fecha a conexão.
const MAX_LINE: u64 = 1 << 20;
/// Quanto uma conexão espera o laço principal responder.
const REPLY_TIMEOUT: Duration = Duration::from_secs(5);
/// Um cliente que não lê não pode prender o servidor.
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

struct Pending {
    request: Request,
    reply: Sender<Response>,
}

type Subscribers = Arc<Mutex<Vec<Sender<WireEvent>>>>;

pub struct IpcServer {
    path: PathBuf,
    pending: Receiver<Pending>,
    subscribers: Subscribers,
    stop: Arc<AtomicBool>,
}

impl IpcServer {
    /// Cria o socket em `path` (diretório `0700`, socket `0600`: só o dono
    /// fala com o daemon). Recusa se já há um daemon respondendo ali; remove um
    /// arquivo de socket velho de um daemon que morreu sem limpar.
    pub fn bind(path: &Path) -> std::io::Result<Self> {
        super::check_socket_path(path)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
            // Só endurece um diretório que é nosso (criado para o socket).
            if dir.file_name().is_some_and(|n| n == "rrv") {
                let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
            }
        }
        if path.exists() {
            if UnixStream::connect(path).is_ok() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AddrInUse,
                    format!("já há um daemon respondendo em {}", path.display()),
                ));
            }
            std::fs::remove_file(path)?;
        }
        let listener = UnixListener::bind(path)?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;

        let (tx, pending) = mpsc::channel();
        let subscribers: Subscribers = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        {
            let (tx, subs, stop) = (tx, subscribers.clone(), stop.clone());
            std::thread::Builder::new()
                .name("rrv-ipc-accept".into())
                .spawn(move || {
                    while !stop.load(Ordering::Relaxed) {
                        match listener.accept() {
                            Ok((stream, _)) => {
                                let (tx, subs) = (tx.clone(), subs.clone());
                                let _ = std::thread::Builder::new()
                                    .name("rrv-ipc-conn".into())
                                    .spawn(move || connection(stream, tx, subs));
                            }
                            Err(_) => std::thread::sleep(Duration::from_millis(50)),
                        }
                    }
                })?;
        }
        Ok(Self {
            path: path.to_path_buf(),
            pending,
            subscribers,
            stop,
        })
    }

    /// Executa os pedidos que chegaram desde o último tick. Retorna quantos.
    pub fn poll(&self, host: &mut Host<'_>) -> usize {
        let mut n = 0;
        while let Ok(p) = self.pending.try_recv() {
            let response = handler::apply(host, &p.request);
            let _ = p.reply.send(response);
            n += 1;
        }
        n
    }

    /// Entrega eventos aos assinantes; quem desconectou sai da lista.
    pub fn publish(&self, events: &[WireEvent]) {
        if events.is_empty() {
            return;
        }
        let mut subs = self.subscribers.lock().unwrap_or_else(|e| e.into_inner());
        subs.retain(|tx| events.iter().all(|ev| tx.send(ev.clone()).is_ok()));
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = std::fs::remove_file(&self.path);
    }
}

fn send(writer: &Mutex<UnixStream>, message: &ServerMessage) -> std::io::Result<()> {
    let line = encode_line(message);
    writer
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .write_all(line.as_bytes())
}

/// Lê uma linha de no máximo `MAX_LINE` bytes. `None` = conexão fechada ou
/// linha grande demais.
fn read_line(reader: &mut BufReader<UnixStream>) -> Option<String> {
    let mut buf = String::new();
    match reader.by_ref().take(MAX_LINE).read_line(&mut buf) {
        Ok(0) | Err(_) => None,
        Ok(_) if !buf.ends_with('\n') => None,
        Ok(_) => Some(buf),
    }
}

fn connection(stream: UnixStream, pending: Sender<Pending>, subscribers: Subscribers) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_write_timeout(Some(WRITE_TIMEOUT));
    let Ok(write_half) = stream.try_clone() else {
        return;
    };
    let writer = Arc::new(Mutex::new(write_half));
    let mut reader = BufReader::new(stream);

    // 1. A primeira mensagem tem de ser Hello com a versão certa.
    let Some(first) = read_line(&mut reader) else {
        return;
    };
    let hello = match decode_line::<Request>(&first) {
        Ok(Request::Hello { protocol }) => handler::hello(protocol),
        Ok(_) => Response::Error {
            message: "a primeira mensagem tem de ser Hello".into(),
        },
        Err(message) => Response::Error { message },
    };
    let accepted = matches!(hello, Response::Hello { .. });
    if send(&writer, &ServerMessage::Response(hello)).is_err() || !accepted {
        return;
    }

    // 2. Pedidos.
    while let Some(line) = read_line(&mut reader) {
        let request = match decode_line::<Request>(&line) {
            Ok(r) => r,
            Err(message) => {
                if send(
                    &writer,
                    &ServerMessage::Response(Response::Error { message }),
                )
                .is_err()
                {
                    return;
                }
                continue;
            }
        };
        let response = match request {
            Request::Hello { .. } => Response::Error {
                message: "Hello só vale na primeira mensagem".into(),
            },
            Request::Subscribe => {
                let (tx, rx) = mpsc::channel::<WireEvent>();
                subscribers
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(tx);
                let w = writer.clone();
                let _ = std::thread::Builder::new()
                    .name("rrv-ipc-events".into())
                    .spawn(move || {
                        for ev in rx {
                            if send(&w, &ServerMessage::Event(ev)).is_err() {
                                break;
                            }
                        }
                    });
                Response::Subscribed
            }
            other => {
                let (reply, wait) = mpsc::channel();
                if pending
                    .send(Pending {
                        request: other,
                        reply,
                    })
                    .is_err()
                {
                    return; // o daemon está encerrando
                }
                wait.recv_timeout(REPLY_TIMEOUT).unwrap_or(Response::Error {
                    message: "o daemon está ocupado, tente de novo".into(),
                })
            }
        };
        if send(&writer, &ServerMessage::Response(response)).is_err() {
            return;
        }
    }
}
