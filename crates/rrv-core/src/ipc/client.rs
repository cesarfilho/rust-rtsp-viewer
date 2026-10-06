//! O lado da janela (e do `rrvctl`): conecta ao daemon, faz pedidos e recebe os
//! eventos. Síncrono e simples; a janela o usa numa thread própria (2.5.7).

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use super::auth;
use super::conn::Conn;
use super::protocol::{
    PROTOCOL_VERSION, Request, Response, ServerMessage, WireEvent, decode_line, encode_line,
};

/// Por que uma conexão falhou. A janela mostra mensagens diferentes e age de
/// forma diferente em cada caso (spec `ux-daemon.md`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectError {
    /// Nada ouvindo no socket (daemon parado ou ainda subindo).
    NotRunning(String),
    /// O socket existe mas é de outro usuário.
    PermissionDenied(String),
    /// O daemon fala outra versão do protocolo.
    Incompatible(String),
    Other(String),
}

impl std::fmt::Display for ConnectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (Self::NotRunning(m)
        | Self::PermissionDenied(m)
        | Self::Incompatible(m)
        | Self::Other(m)) = self;
        f.write_str(m)
    }
}

/// O prefixo de um endereço TCP no lugar do caminho do socket: `tcp://192.168.1.10:7878`.
pub const TCP_PREFIX: &str = "tcp://";

/// Quanto esperar o TCP conectar (um IP que não responde não pode prender a janela).
const TCP_CONNECT_TIMEOUT: Duration = Duration::from_secs(4);

/// O endereço `host:porta` de um alvo `tcp://host:porta`, ou `None` se for um caminho de socket Unix.
pub fn tcp_address(target: &Path) -> Option<&str> {
    target.to_str()?.strip_prefix(TCP_PREFIX)
}

pub struct IpcClient {
    reader: BufReader<Conn>,
    writer: Conn,
    /// Eventos que chegaram enquanto se esperava uma resposta.
    events: VecDeque<WireEvent>,
    /// Quem fala do outro lado (`rrv-daemon 0.8.0`).
    pub server: String,
}

impl IpcClient {
    /// Conecta e faz o `Hello`. Erro claro se o daemon não está rodando ou se
    /// fala outra versão do protocolo.
    pub fn connect(path: &Path) -> Result<Self, String> {
        Self::connect_detailed(path).map_err(|e| e.to_string())
    }

    /// Como [`IpcClient::connect`], dizendo *por que* falhou.
    pub fn connect_detailed(path: &Path) -> Result<Self, ConnectError> {
        Self::connect_target(path, None)
    }

    /// Conecta a um socket Unix (`/caminho/rrv.sock`) ou a um daemon pela rede (`tcp://host:porta`, que
    /// exige o `token` do daemon). Diz *por que* falhou.
    pub fn connect_target(target: &Path, token: Option<&str>) -> Result<Self, ConnectError> {
        let mut client = if let Some(addr) = tcp_address(target) {
            Self::open_tcp(addr, token)?
        } else {
            Self::open_unix(target)?
        };
        client.hello()?;
        Ok(client)
    }

    fn open_tcp(addr: &str, token: Option<&str>) -> Result<Self, ConnectError> {
        let sock = addr
            .to_socket_addrs()
            .map_err(|e| ConnectError::Other(format!("endereço '{addr}' inválido: {e}")))?
            .next()
            .ok_or_else(|| ConnectError::Other(format!("não achei o endereço '{addr}'")))?;
        let stream = TcpStream::connect_timeout(&sock, TCP_CONNECT_TIMEOUT).map_err(|e| {
            ConnectError::NotRunning(format!(
                "não consegui falar com o daemon em {addr} ({e}); ele está rodando e escutando nessa porta?"
            ))
        })?;
        let _ = stream.set_nodelay(true);
        let conn = Conn::Tcp(stream);
        let writer = conn
            .try_clone()
            .map_err(|e| ConnectError::Other(e.to_string()))?;
        let mut client = Self {
            reader: BufReader::new(conn),
            writer,
            events: VecDeque::new(),
            server: String::new(),
        };
        // 1. O daemon abre com um desafio; sem token não há como responder.
        let nonce = match client.read_first(Duration::from_secs(5))? {
            ServerMessage::Challenge { nonce } => nonce,
            other => {
                return Err(ConnectError::Other(format!(
                    "o servidor em {addr} não é um rrv-daemon (esperava um desafio, veio {other:?})"
                )));
            }
        };
        let Some(token) = token else {
            return Err(ConnectError::PermissionDenied(format!(
                "o daemon em {addr} exige um token (RRV_TOKEN, ou o segredo rrv_token no chaveiro)"
            )));
        };
        // 2. A resposta prova que temos o token sem enviá-lo.
        match client
            .request(&Request::Auth {
                response: auth::respond(token, &nonce),
            })
            .map_err(ConnectError::Other)?
        {
            Response::Ok => Ok(client),
            Response::Error { .. } => Err(ConnectError::PermissionDenied(format!(
                "o daemon em {addr} recusou o token"
            ))),
            other => Err(ConnectError::Other(format!(
                "resposta inesperada à autenticação: {other:?}"
            ))),
        }
    }

    /// A primeira mensagem do servidor, com prazo.
    fn read_first(&mut self, timeout: Duration) -> Result<ServerMessage, ConnectError> {
        self.reader
            .get_ref()
            .set_read_timeout(Some(timeout))
            .map_err(|e| ConnectError::Other(e.to_string()))?;
        match self.read_message().map_err(ConnectError::Other)? {
            Some(m) => Ok(m),
            None => Err(ConnectError::Other(
                "o servidor não respondeu a tempo".into(),
            )),
        }
    }

    fn open_unix(path: &Path) -> Result<Self, ConnectError> {
        super::check_socket_path(path).map_err(ConnectError::Other)?;
        let stream = UnixStream::connect(path).map_err(|e| {
            let msg = format!(
                "não consegui falar com o daemon em {} ({e}); ele está rodando?",
                path.display()
            );
            if e.kind() == ErrorKind::PermissionDenied {
                ConnectError::PermissionDenied(msg)
            } else {
                ConnectError::NotRunning(msg)
            }
        })?;
        let conn = Conn::Unix(stream);
        let writer = conn
            .try_clone()
            .map_err(|e| ConnectError::Other(e.to_string()))?;
        Ok(Self {
            reader: BufReader::new(conn),
            writer,
            events: VecDeque::new(),
            server: String::new(),
        })
    }

    /// O `Hello` (versão do protocolo), depois de aberta e autenticada a conexão.
    fn hello(&mut self) -> Result<(), ConnectError> {
        let client = self;
        match client
            .request(&Request::Hello {
                protocol: PROTOCOL_VERSION,
            })
            .map_err(ConnectError::Other)?
        {
            Response::Hello { server, .. } => client.server = server,
            Response::Error { message } if message.contains("versão do protocolo") => {
                return Err(ConnectError::Incompatible(message));
            }
            Response::Error { message } => return Err(ConnectError::Other(message)),
            other => {
                return Err(ConnectError::Other(format!(
                    "resposta inesperada ao Hello: {other:?}"
                )));
            }
        }
        Ok(())
    }

    /// Faz um pedido e devolve a resposta. Eventos que chegarem no meio ficam
    /// na fila de [`IpcClient::next_event`].
    pub fn request(&mut self, request: &Request) -> Result<Response, String> {
        self.request_timeout(request, Duration::from_secs(10))
    }

    /// Como [`IpcClient::request`], com o prazo que o chamador escolhe (o
    /// heartbeat da janela usa um curto).
    pub fn request_timeout(
        &mut self,
        request: &Request,
        timeout: Duration,
    ) -> Result<Response, String> {
        self.reader
            .get_ref()
            .set_read_timeout(Some(timeout))
            .map_err(|e| e.to_string())?;
        self.writer
            .write_all(encode_line(request).as_bytes())
            .map_err(|e| format!("falha ao enviar: {e}"))?;
        loop {
            match self.read_message()? {
                Some(ServerMessage::Response(r)) => return Ok(r),
                Some(ServerMessage::Event(e)) => self.events.push_back(e),
                Some(ServerMessage::Challenge { .. }) => {
                    return Err("o daemon mandou um desafio fora de hora".into());
                }
                None => return Err("o daemon não respondeu a tempo".into()),
            }
        }
    }

    /// Passa a receber eventos.
    pub fn subscribe(&mut self) -> Result<(), String> {
        match self.request(&Request::Subscribe)? {
            Response::Subscribed => Ok(()),
            Response::Error { message } => Err(message),
            other => Err(format!("resposta inesperada ao Subscribe: {other:?}")),
        }
    }

    /// O próximo evento, esperando até `timeout`. `Ok(None)` = nenhum no prazo.
    pub fn next_event(&mut self, timeout: Duration) -> Result<Option<WireEvent>, String> {
        if let Some(e) = self.events.pop_front() {
            return Ok(Some(e));
        }
        self.reader
            .get_ref()
            .set_read_timeout(Some(timeout.max(Duration::from_millis(1))))
            .map_err(|e| e.to_string())?;
        loop {
            match self.read_message()? {
                Some(ServerMessage::Event(e)) => return Ok(Some(e)),
                // uma resposta sem pedido pendente não deveria existir; ignora
                Some(ServerMessage::Response(_) | ServerMessage::Challenge { .. }) => continue,
                None => return Ok(None),
            }
        }
    }

    /// Lê uma mensagem. `Ok(None)` = estourou o prazo de leitura.
    fn read_message(&mut self) -> Result<Option<ServerMessage>, String> {
        let mut line = String::new();
        match self.reader.read_line(&mut line) {
            Ok(0) => Err("o daemon fechou a conexão".into()),
            Ok(_) => decode_line(&line).map(Some),
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => Ok(None),
            Err(e) => Err(format!("falha ao ler: {e}")),
        }
    }
}
