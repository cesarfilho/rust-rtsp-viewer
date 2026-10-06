//! O canal entre a janela e o daemon: socket Unix local **ou TCP pela LAN** (com token, veja [`auth`]),
//! uma mensagem JSON por linha, versionado (ADR 0010).
//!
//! - [`protocol`]: os tipos e o formato do fio.
//! - [`handler`]: o que cada pedido faz no motor (puro, sem socket).
//! - [`server`]: o socket do daemon.
//! - [`client`]: o lado da janela (e do `rrvctl`).
//! - [`link`]: a conexão da janela mantida numa thread (heartbeat, backoff).

pub mod auth;
pub mod client;
pub mod conn;
pub mod handler;
pub mod link;
pub mod protocol;
pub mod server;

use std::path::PathBuf;

/// Onde fica o socket: `RRV_SOCKET`, senão `$XDG_RUNTIME_DIR/rrv/rrv.sock`,
/// senão `/tmp/rrv/rrv.sock`. No Docker, `RRV_SOCKET=/run/rrv/rrv.sock` e esse
/// diretório é um volume compartilhado com o host.
pub fn default_socket_path() -> PathBuf {
    if let Some(p) = std::env::var_os("RRV_SOCKET") {
        return PathBuf::from(p);
    }
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join("rrv").join("rrv.sock")
}

/// O token do canal pela rede para quem se conecta: a variável `RRV_TOKEN`, senão o segredo `rrv_token`
/// (arquivo em `$RRV_SECRETS_DIR` ou chaveiro do sistema). `None` se não houver.
pub fn client_token() -> Option<String> {
    std::env::var("RRV_TOKEN")
        .ok()
        .filter(|t| !t.is_empty())
        .or_else(|| crate::secrets::lookup("rrv_token"))
}

/// O limite de `sun_path` dos sockets Unix (108 no Linux, contando o NUL).
pub const MAX_SOCKET_PATH_BYTES: usize = 107;

/// Erro claro quando o caminho não cabe: sem isso o sistema responde só
/// "path must be shorter than SUN_LEN".
pub fn check_socket_path(path: &std::path::Path) -> Result<(), String> {
    use std::os::unix::ffi::OsStrExt;
    let n = path.as_os_str().as_bytes().len();
    if n > MAX_SOCKET_PATH_BYTES {
        return Err(format!(
            "o caminho do socket tem {n} bytes e o limite do Unix é {MAX_SOCKET_PATH_BYTES}: \
             use um mais curto (RRV_SOCKET ou --socket): {}",
            path.display()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_socket_path_that_is_too_long_is_rejected_with_a_clear_message() {
        assert!(check_socket_path(std::path::Path::new("/run/user/1000/rrv/rrv.sock")).is_ok());
        let long = format!("/tmp/{}/rrv.sock", "x".repeat(120));
        let err = check_socket_path(std::path::Path::new(&long)).unwrap_err();
        assert!(err.contains("limite do Unix"), "{err}");
    }
}
