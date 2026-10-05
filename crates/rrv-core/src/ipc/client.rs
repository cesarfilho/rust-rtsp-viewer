//! O lado da janela (e do `rrvctl`): conecta ao daemon, faz pedidos e recebe os
//! eventos. Síncrono e simples; a janela o usa numa thread própria (2.5.7).

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

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

pub struct IpcClient {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
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
        let writer = stream
            .try_clone()
            .map_err(|e| ConnectError::Other(e.to_string()))?;
        let mut client = Self {
            reader: BufReader::new(stream),
            writer,
            events: VecDeque::new(),
            server: String::new(),
        };
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
        Ok(client)
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
                Some(ServerMessage::Response(_)) => continue,
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
