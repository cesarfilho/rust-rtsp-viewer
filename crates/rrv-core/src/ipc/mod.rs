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
pub mod files;
pub mod handler;
pub mod link;
pub mod protocol;
pub mod server;

use std::path::PathBuf;

/// Onde fica o socket: `RRV_SOCKET`; senão o primeiro que existir entre
/// `$XDG_RUNTIME_DIR/rrv/rrv.sock` (daemon nativo) e [`docker_socket_path`] (o do compose.yaml);
/// sem nenhum, o primeiro. No contêiner, `RRV_SOCKET=/run/rrv/rrv.sock`.
pub fn default_socket_path() -> PathBuf {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("rrv")
        .join("rrv.sock");
    pick_socket(
        std::env::var_os("RRV_SOCKET").map(PathBuf::from),
        [runtime, docker_socket_path()],
        |p| p.exists(),
    )
}

/// O socket do daemon do Docker visto do host: `~/.local/state/rust-rtsp-viewer/run/rrv.sock`.
/// Fica no disco, não em `$XDG_RUNTIME_DIR`: o contêiner sobe no boot, antes do login, quando
/// `/run/user/<uid>` ainda não existe, e o Docker criava a pasta como root (o daemon não abria
/// o socket e reiniciava sem parar).
pub fn docker_socket_path() -> PathBuf {
    crate::infrastructure::view_state::state_dir()
        .join("run")
        .join("rrv.sock")
}

fn pick_socket(
    forced: Option<PathBuf>,
    candidates: [PathBuf; 2],
    exists: impl Fn(&std::path::Path) -> bool,
) -> PathBuf {
    if let Some(p) = forced {
        return p;
    }
    let [first, second] = candidates;
    if !exists(&first) && exists(&second) {
        second
    } else {
        first
    }
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

#[cfg(test)]
mod socket_tests {
    use super::pick_socket;
    use std::path::PathBuf;

    #[test]
    fn the_socket_is_the_forced_one_else_the_first_that_exists() {
        let (a, b) = (
            PathBuf::from("/run/u/rrv/rrv.sock"),
            PathBuf::from("/h/run/rrv.sock"),
        );
        let only = |x: &'static str| move |p: &std::path::Path| p == std::path::Path::new(x);
        let forced = Some(PathBuf::from("/x.sock"));
        assert_eq!(
            pick_socket(forced, [a.clone(), b.clone()], |_| true),
            PathBuf::from("/x.sock")
        );
        assert_eq!(
            pick_socket(None, [a.clone(), b.clone()], only("/h/run/rrv.sock")),
            b
        );
        assert_eq!(
            pick_socket(None, [a.clone(), b.clone()], |_| true),
            a,
            "os dois: o nativo"
        );
        assert_eq!(
            pick_socket(None, [a.clone(), b], |_| false),
            a,
            "nenhum: o nativo"
        );
    }
}
