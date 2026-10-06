//! Os vídeos gravados pela rede (janela e daemon em máquinas diferentes): um servidor HTTP mínimo, só
//! de leitura, com suporte a `Range` (o player precisa pular para o meio do arquivo sem baixá-lo todo).
//!
//! **Autorização por URL assinada.** O GStreamer não sabe responder a um desafio, então o canal de
//! controle (já autenticado) entrega à janela uma URL de um arquivo só, válida por pouco tempo:
//! `/f/<arquivo>?exp=<unix>&sig=<HMAC-SHA256(token, "f|arquivo|exp")>`. O token nunca aparece na URL; a
//! assinatura não serve para outro arquivo nem depois de `exp`. O servidor só entrega arquivos de
//! dentro da pasta de gravações (sem `..`, sem caminho absoluto, sem sair por link simbólico). O
//! tráfego é HTTP puro, como o resto do canal (veja `ipc::auth`).

use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Quanto tempo uma URL assinada vale (o player abre o arquivo e segue pedindo trechos dele).
pub const URL_TTL_SECS: u64 = 6 * 3600;
const MAX_CONNECTIONS: usize = 32;
const CHUNK: usize = 256 * 1024;
const HEADER_LIMIT: u64 = 16 * 1024;

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Um caminho relativo seguro: sem `..`, sem raiz, sem NUL, não vazio.
pub fn safe_relative(file: &str) -> Option<PathBuf> {
    if file.is_empty() || file.contains('\0') {
        return None;
    }
    let p = Path::new(file);
    p.components()
        .all(|c| matches!(c, Component::Normal(_)))
        .then(|| p.to_path_buf())
}

fn message(file: &str, exp: u64) -> String {
    format!("f|{file}|{exp}")
}

fn sign(token: &str, file: &str, exp: u64) -> String {
    let mut m = HmacSha256::new_from_slice(token.as_bytes()).expect("HMAC aceita qualquer chave");
    m.update(message(file, exp).as_bytes());
    m.finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn verify(token: &str, file: &str, exp: u64, sig: &str) -> bool {
    if sig.len() != 64 || !sig.is_ascii() {
        return false;
    }
    let Ok(bytes) = (0..sig.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&sig[i..i + 2], 16))
        .collect::<Result<Vec<u8>, _>>()
    else {
        return false;
    };
    let mut m = HmacSha256::new_from_slice(token.as_bytes()).expect("HMAC aceita qualquer chave");
    m.update(message(file, exp).as_bytes());
    m.verify_slice(&bytes).is_ok()
}

fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// O que o daemon sabe para entregar URLs: o token, a porta HTTP e a pasta.
#[derive(Debug, Clone)]
pub struct FileServe {
    token: Arc<str>,
    pub port: u16,
    dir: PathBuf,
}

impl FileServe {
    /// A parte `caminho?exp=…&sig=…` da URL de `file` (relativo à pasta de gravações). Erro claro se o
    /// caminho não é seguro ou o arquivo não existe.
    pub fn signed_path(&self, file: &str) -> Result<String, String> {
        let rel = safe_relative(file).ok_or("nome de arquivo inválido")?;
        resolve(&self.dir, &rel).ok_or("arquivo não encontrado no daemon")?;
        let exp = now() + URL_TTL_SECS;
        let sig = sign(&self.token, file, exp);
        Ok(format!("/f/{}?exp={exp}&sig={sig}", percent_encode(file)))
    }
}

/// O arquivo de verdade, se existe, é um arquivo e está **dentro** de `dir` (links simbólicos
/// resolvidos).
fn resolve(dir: &Path, rel: &Path) -> Option<PathBuf> {
    let root = dir.canonicalize().ok()?;
    let full = root.join(rel).canonicalize().ok()?;
    (full.starts_with(&root) && full.is_file()).then_some(full)
}

/// Sobe o servidor HTTP de arquivos em `addr`. Devolve o que o canal de controle usa para assinar
/// URLs e a thread corre até `stop` (o daemon a fecha ao sair).
pub fn serve(
    addr: SocketAddr,
    token: &str,
    dir: PathBuf,
    stop: Arc<AtomicBool>,
) -> std::io::Result<FileServe> {
    let listener = TcpListener::bind(addr)?;
    listener.set_nonblocking(true)?;
    let port = listener.local_addr()?.port();
    let serve = FileServe {
        token: Arc::from(token),
        port,
        dir,
    };
    let state = serve.clone();
    let open = Arc::new(AtomicUsize::new(0));
    std::thread::Builder::new()
        .name("rrv-files".into())
        .spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        if open.load(Ordering::Relaxed) >= MAX_CONNECTIONS {
                            continue;
                        }
                        open.fetch_add(1, Ordering::Relaxed);
                        let (state, open2) = (state.clone(), open.clone());
                        let spawned = std::thread::Builder::new()
                            .name("rrv-files-conn".into())
                            .spawn(move || {
                                let _ = handle(stream, &state);
                                open2.fetch_sub(1, Ordering::Relaxed);
                            });
                        if spawned.is_err() {
                            open.fetch_sub(1, Ordering::Relaxed);
                        }
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(50)),
                }
            }
        })?;
    Ok(serve)
}

/// Os pedidos de um cliente na mesma conexão (o player reutiliza), até ele fechar.
fn handle(stream: TcpStream, state: &FileServe) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    stream.set_write_timeout(Some(Duration::from_secs(30)))?;
    let _ = stream.set_nodelay(true);
    let mut writer = stream.try_clone()?;
    let mut reader = BufReader::new(stream);
    loop {
        let mut line = String::new();
        if reader.by_ref().take(HEADER_LIMIT).read_line(&mut line)? == 0 {
            return Ok(());
        }
        let mut headers = Vec::new();
        loop {
            let mut h = String::new();
            if reader.by_ref().take(HEADER_LIMIT).read_line(&mut h)? == 0 {
                return Ok(());
            }
            if h == "\r\n" || h == "\n" {
                break;
            }
            if headers.len() > 64 {
                return respond(
                    &mut writer,
                    431,
                    "Request Header Fields Too Large",
                    &[],
                    None,
                );
            }
            headers.push(h);
        }
        let keep_alive = headers.iter().any(|h| {
            h.to_ascii_lowercase().starts_with("connection:")
                && h.to_ascii_lowercase().contains("keep-alive")
        });
        let range = headers.iter().find_map(|h| {
            h.to_ascii_lowercase()
                .strip_prefix("range:")
                .map(|v| v.trim().to_string())
        });
        let done = answer(
            &mut writer,
            state,
            line.trim_end(),
            range.as_deref(),
            keep_alive,
        )?;
        if !keep_alive || !done {
            return Ok(());
        }
    }
}

/// O que um pedido pede: `(arquivo, exp, sig)` de `/f/<arquivo>?exp=&sig=`.
pub fn parse_target(target: &str) -> Option<(String, u64, String)> {
    let rest = target.strip_prefix("/f/")?;
    let (path, query) = rest.split_once('?')?;
    let file = percent_decode(path)?;
    let mut exp = None;
    let mut sig = None;
    for kv in query.split('&') {
        match kv.split_once('=') {
            Some(("exp", v)) => exp = v.parse().ok(),
            Some(("sig", v)) => sig = Some(v.to_string()),
            _ => {}
        }
    }
    Some((file, exp?, sig?))
}

/// `bytes=a-b`, `bytes=a-` ou `bytes=-n` num arquivo de `len` bytes → `[início, fim]` inclusivos.
pub fn parse_range(value: &str, len: u64) -> Option<(u64, u64)> {
    let spec = value.trim().strip_prefix("bytes=")?;
    if spec.contains(',') || len == 0 {
        return None; // vários trechos: não suportado
    }
    let (a, b) = spec.split_once('-')?;
    let (start, end) = match (a.trim(), b.trim()) {
        ("", n) => {
            let n: u64 = n.parse().ok()?;
            (len.saturating_sub(n), len - 1)
        }
        (s, "") => (s.parse().ok()?, len - 1),
        (s, e) => (s.parse().ok()?, e.parse::<u64>().ok()?.min(len - 1)),
    };
    (start <= end && start < len).then_some((start, end))
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("mp4") => "video/mp4",
        Some("mkv") => "video/x-matroska",
        _ => "application/octet-stream",
    }
}

/// Responde a um pedido. `Ok(true)` = a resposta foi inteira (a conexão pode seguir).
fn answer(
    w: &mut TcpStream,
    state: &FileServe,
    request_line: &str,
    range: Option<&str>,
    keep_alive: bool,
) -> std::io::Result<bool> {
    let mut parts = request_line.split_whitespace();
    let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    if method != "GET" && method != "HEAD" {
        respond(
            w,
            405,
            "Method Not Allowed",
            &[("Allow", "GET, HEAD".into())],
            None,
        )?;
        return Ok(false);
    }
    let Some((file, exp, sig)) = parse_target(target) else {
        respond(w, 400, "Bad Request", &[], None)?;
        return Ok(false);
    };
    // 403 igual para assinatura errada, vencida ou de outro arquivo: não diz qual.
    if exp < now() || !verify(&state.token, &file, exp, &sig) {
        respond(w, 403, "Forbidden", &[], None)?;
        return Ok(false);
    }
    let Some(full) = safe_relative(&file).and_then(|rel| resolve(&state.dir, &rel)) else {
        respond(w, 404, "Not Found", &[], None)?;
        return Ok(false);
    };
    let mut f = std::fs::File::open(&full)?;
    let len = f.metadata()?.len();
    let conn = (
        "Connection",
        if keep_alive { "keep-alive" } else { "close" }.to_string(),
    );
    let base = [
        ("Content-Type", content_type(&full).to_string()),
        ("Accept-Ranges", "bytes".to_string()),
        conn,
    ];
    let (status, reason, start, end) = match range {
        None => (200, "OK", 0, len.saturating_sub(1)),
        Some(r) => match parse_range(r, len) {
            Some((s, e)) => (206, "Partial Content", s, e),
            None => {
                respond(
                    w,
                    416,
                    "Range Not Satisfiable",
                    &[("Content-Range", format!("bytes */{len}"))],
                    None,
                )?;
                return Ok(false);
            }
        },
    };
    let body_len = if len == 0 { 0 } else { end - start + 1 };
    let mut headers = base.to_vec();
    headers.push(("Content-Length", body_len.to_string()));
    if status == 206 {
        headers.push(("Content-Range", format!("bytes {start}-{end}/{len}")));
    }
    respond(w, status, reason, &headers, Some(()))?;
    if method == "HEAD" || body_len == 0 {
        return Ok(true);
    }
    f.seek(SeekFrom::Start(start))?;
    let mut left = body_len;
    let mut buf = vec![0u8; CHUNK];
    while left > 0 {
        let n = f.read(&mut buf[..CHUNK.min(left as usize)])?;
        if n == 0 {
            return Ok(false); // o arquivo encolheu no meio: fecha
        }
        w.write_all(&buf[..n])?;
        left -= n as u64;
    }
    Ok(true)
}

/// O cabeçalho de uma resposta. Com `body = Some(())` quem chama escreve o corpo em seguida.
fn respond(
    w: &mut TcpStream,
    status: u16,
    reason: &str,
    headers: &[(&str, String)],
    body: Option<()>,
) -> std::io::Result<()> {
    let mut head = format!("HTTP/1.1 {status} {reason}\r\n");
    for (k, v) in headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    if body.is_none() {
        head.push_str("Content-Length: 0\r\nConnection: close\r\n");
    }
    head.push_str("\r\n");
    w.write_all(head.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_relative_paths_are_safe() {
        assert!(safe_relative("cam/a.mkv").is_some());
        assert!(safe_relative("exports/clip.mp4").is_some());
        for bad in ["", "../x", "a/../../x", "/etc/passwd", "a\0b", "./", ".."] {
            assert!(safe_relative(bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn a_signature_is_bound_to_its_file_its_expiry_and_the_token() {
        let sig = sign("token-0123456789abcdef", "cam/a.mkv", 100);
        assert!(verify("token-0123456789abcdef", "cam/a.mkv", 100, &sig));
        assert!(
            !verify("token-0123456789abcdef", "cam/b.mkv", 100, &sig),
            "outro arquivo"
        );
        assert!(
            !verify("token-0123456789abcdef", "cam/a.mkv", 101, &sig),
            "outra validade"
        );
        assert!(!verify("outro-token-0123456789", "cam/a.mkv", 100, &sig));
        assert!(!verify("token-0123456789abcdef", "cam/a.mkv", 100, "zz"));
        assert!(!verify(
            "token-0123456789abcdef",
            "cam/a.mkv",
            100,
            &"0".repeat(64)
        ));
    }

    #[test]
    fn percent_coding_round_trips_names_with_spaces_and_accents() {
        for name in ["cam/Portão da frente.mkv", "a+b&c=d.mp4", "x/y z/ç.mkv"] {
            let enc = percent_encode(name);
            assert!(
                !enc.contains(' ') && !enc.contains('&') && !enc.contains('+'),
                "{enc}"
            );
            assert_eq!(percent_decode(&enc).as_deref(), Some(name));
        }
        assert_eq!(percent_decode("%zz"), None);
        assert_eq!(percent_decode("%4"), None);
    }

    #[test]
    fn the_target_is_parsed_into_file_expiry_and_signature() {
        let (f, e, s) = parse_target("/f/cam/a%20b.mkv?exp=42&sig=abc").unwrap();
        assert_eq!((f.as_str(), e, s.as_str()), ("cam/a b.mkv", 42, "abc"));
        assert!(parse_target("/x/a?exp=1&sig=a").is_none());
        assert!(parse_target("/f/a?sig=a").is_none());
        assert!(parse_target("/f/a?exp=1").is_none());
    }

    #[test]
    fn ranges_cover_the_usual_forms_and_refuse_the_rest() {
        assert_eq!(parse_range("bytes=0-99", 1000), Some((0, 99)));
        assert_eq!(parse_range("bytes=500-", 1000), Some((500, 999)));
        assert_eq!(parse_range("bytes=-100", 1000), Some((900, 999)));
        assert_eq!(
            parse_range("bytes=900-5000", 1000),
            Some((900, 999)),
            "fim além do arquivo"
        );
        assert_eq!(
            parse_range("bytes=1000-", 1000),
            None,
            "começa depois do fim"
        );
        assert_eq!(parse_range("bytes=5-2", 1000), None);
        assert_eq!(parse_range("bytes=0-1,5-9", 1000), None, "vários trechos");
        assert_eq!(parse_range("items=0-1", 1000), None);
        assert_eq!(parse_range("bytes=0-1", 0), None);
    }
}
