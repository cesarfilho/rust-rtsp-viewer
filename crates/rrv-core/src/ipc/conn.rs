//! Uma conexão do canal: socket Unix (local) ou TCP (pela LAN). O resto do código lê e escreve nela
//! sem saber qual é.

use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream};
use std::os::unix::net::UnixStream;
use std::time::Duration;

#[derive(Debug)]
pub enum Conn {
    Unix(UnixStream),
    Tcp(TcpStream),
}

impl Conn {
    pub fn try_clone(&self) -> std::io::Result<Self> {
        Ok(match self {
            Self::Unix(s) => Self::Unix(s.try_clone()?),
            Self::Tcp(s) => Self::Tcp(s.try_clone()?),
        })
    }

    pub fn set_read_timeout(&self, d: Option<Duration>) -> std::io::Result<()> {
        match self {
            Self::Unix(s) => s.set_read_timeout(d),
            Self::Tcp(s) => s.set_read_timeout(d),
        }
    }

    pub fn set_write_timeout(&self, d: Option<Duration>) -> std::io::Result<()> {
        match self {
            Self::Unix(s) => s.set_write_timeout(d),
            Self::Tcp(s) => s.set_write_timeout(d),
        }
    }

    pub fn set_nonblocking(&self, nb: bool) -> std::io::Result<()> {
        match self {
            Self::Unix(s) => s.set_nonblocking(nb),
            Self::Tcp(s) => s.set_nonblocking(nb),
        }
    }

    pub fn shutdown(&self) {
        let _ = match self {
            Self::Unix(s) => s.shutdown(Shutdown::Both),
            Self::Tcp(s) => s.shutdown(Shutdown::Both),
        };
    }

    /// Quem está do outro lado, para o log (`ip:porta`); vazio no socket Unix.
    pub fn peer(&self) -> String {
        match self {
            Self::Unix(_) => String::new(),
            Self::Tcp(s) => s.peer_addr().map(|a| a.to_string()).unwrap_or_default(),
        }
    }
}

impl Read for Conn {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Unix(s) => s.read(buf),
            Self::Tcp(s) => s.read(buf),
        }
    }
}

impl Write for Conn {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Self::Unix(s) => s.write(buf),
            Self::Tcp(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Self::Unix(s) => s.flush(),
            Self::Tcp(s) => s.flush(),
        }
    }
}

// `&Conn` também lê/escreve (como `&UnixStream`): o servidor usa a mesma conexão em duas threads.
impl Read for &Conn {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Conn::Unix(s) => (&*s).read(buf),
            Conn::Tcp(s) => (&*s).read(buf),
        }
    }
}

impl Write for &Conn {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Conn::Unix(s) => (&*s).write(buf),
            Conn::Tcp(s) => (&*s).write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
