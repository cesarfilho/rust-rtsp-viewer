//! Os vídeos pela rede: o servidor HTTP do daemon (com `Range`) e as URLs assinadas pelo canal de controle.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::thread;
use std::time::Duration;

use rrv_core::config::{CameraConfig, LogsConfigFile};
use rrv_core::domain::motion::MotionConfig;
use rrv_core::domain::notify::NotifyConfig;
use rrv_core::domain::recording::RecordingConfig;
use rrv_core::engine::{Engine, EngineSettings};
use rrv_core::infrastructure::zone_state::ZonesFile;
use rrv_core::ipc::client::IpcClient;
use rrv_core::ipc::handler::Host;
use rrv_core::ipc::protocol::{Request, Response};
use rrv_core::ipc::server::IpcServer;
use rrv_core::testing::TempDir;

const TOKEN: &str = "token-de-teste-0123456789";

fn engine(tmp: &TempDir) -> Engine {
    let cam: CameraConfig =
        toml::from_str("url = \"rtsp://127.0.0.1:9/0\"\nname = \"cam0\"").unwrap();
    Engine::new(EngineSettings {
        cameras: &[cam],
        recording: &RecordingConfig::default(),
        motion: MotionConfig::default(),
        notify: NotifyConfig::default(),
        logs: &LogsConfigFile {
            dir: Some(tmp.path("logs")),
            ..LogsConfigFile::default()
        },
        pause_hidden: true,
        stagger: Duration::from_millis(100),
        zones: &ZonesFile::default(),
    })
}

/// 10 000 bytes previsíveis: o byte `i` vale `i % 251`.
fn sample() -> Vec<u8> {
    (0..10_000u32).map(|i| (i % 251) as u8).collect()
}

/// Um HTTP/1.1 cru: `(status, cabeçalhos em minúsculas, corpo)`.
fn http(addr: SocketAddr, request: &str) -> (u16, String, Vec<u8>) {
    let mut s = TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    s.write_all(request.as_bytes()).unwrap();
    let mut raw = Vec::new();
    let _ = s.read_to_end(&mut raw);
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("sem cabeçalho");
    let head = String::from_utf8_lossy(&raw[..split]).to_ascii_lowercase();
    let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
    (status, head, raw[split + 4..].to_vec())
}

fn get(addr: SocketAddr, path: &str, range: Option<&str>) -> (u16, String, Vec<u8>) {
    let r = range.map(|r| format!("Range: {r}\r\n")).unwrap_or_default();
    http(
        addr,
        &format!("GET {path} HTTP/1.1\r\nHost: x\r\n{r}Connection: close\r\n\r\n"),
    )
}

#[test]
fn a_signed_url_serves_the_whole_file_and_byte_ranges_and_nothing_else() {
    let tmp = TempDir::new("files");
    let rec = tmp.path("rec");
    std::fs::create_dir_all(rec.join("cam")).unwrap();
    std::fs::write(rec.join("cam/Portão a.mkv"), sample()).unwrap();
    std::fs::write(tmp.path("segredo.txt"), b"fora da pasta").unwrap();
    // um link simbólico para fora da pasta de gravações
    std::os::unix::fs::symlink(tmp.path("segredo.txt"), rec.join("cam/fuga.mkv")).unwrap();

    let mut engine = engine(&tmp);
    let mut zones = ZonesFile::default();
    let server = IpcServer::bind(&tmp.path("rrv/rrv.sock")).unwrap();
    let tcp = server
        .listen_tcp("127.0.0.1:0".parse().unwrap(), TOKEN)
        .unwrap();
    let files = server
        .serve_files("127.0.0.1:0".parse().unwrap(), TOKEN, rec)
        .unwrap();
    let client = thread::spawn(move || {
        let mut c =
            IpcClient::connect_target(format!("tcp://{tcp}").as_ref(), Some(TOKEN)).unwrap();
        let mut ask = |f: &str| c.request(&Request::FileUrl { file: f.into() }).unwrap();
        let ok = ask("cam/Portão a.mkv");
        let bad: Vec<Response> = [
            "../segredo.txt",
            "/etc/passwd",
            "cam/nao-existe.mkv",
            "cam/fuga.mkv",
            "",
        ]
        .iter()
        .map(|f| ask(f))
        .collect();
        (ok, bad)
    });
    while !client.is_finished() {
        server.poll(&mut Host {
            engine: &mut engine,
            zones_file: &mut zones,
            persist: false,
            history: None,
            recordings: None,
        });
        thread::sleep(Duration::from_millis(10));
    }
    let (ok, bad) = client.join().unwrap();

    // pedir uma URL de fora da pasta, ou do que não existe, é erro (inclusive o link para fora)
    for (i, r) in bad.iter().enumerate() {
        assert!(matches!(r, Response::Error { .. }), "caso {i}: {r:?}");
    }
    let Response::FileUrl { port, path } = ok else {
        panic!("{ok:?}");
    };
    assert_eq!(port, files.port());
    let data = sample();

    // o arquivo todo
    let (st, head, body) = get(files, &path, None);
    assert_eq!(st, 200);
    assert!(
        head.contains("accept-ranges: bytes") && head.contains("content-length: 10000"),
        "{head}"
    );
    assert_eq!(body, data);

    // um trecho do meio (o que o player faz ao pular)
    let (st, head, body) = get(files, &path, Some("bytes=1000-1999"));
    assert_eq!(st, 206);
    assert!(
        head.contains("content-range: bytes 1000-1999/10000"),
        "{head}"
    );
    assert_eq!(body, &data[1000..2000]);
    // do meio até o fim, e o rabo
    assert_eq!(get(files, &path, Some("bytes=9990-")).2, &data[9990..]);
    assert_eq!(get(files, &path, Some("bytes=-10")).2, &data[9990..]);
    // além do fim
    assert_eq!(get(files, &path, Some("bytes=20000-")).0, 416);

    // HEAD não traz corpo
    let (st, head, body) = http(
        files,
        &format!("HEAD {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n"),
    );
    assert_eq!((st, body.len()), (200, 0));
    assert!(head.contains("content-length: 10000"));

    // sem assinatura, com assinatura adulterada, de outro arquivo: 403, e nunca o conteúdo
    let (file_part, query) = path.split_once('?').unwrap();
    assert_eq!(get(files, file_part, None).0, 400, "sem exp/sig");
    let tampered = path.replace("sig=", "sig=00");
    assert_eq!(get(files, &tampered, None).0, 403);
    let other_file = format!("{}?{query}", file_part.replace("a.mkv", "b.mkv"));
    assert_eq!(
        get(files, &other_file, None).0,
        403,
        "a assinatura é de um arquivo só"
    );
    let traversal = format!("/f/..%2Fsegredo.txt?{query}");
    assert_eq!(get(files, &traversal, None).0, 403);
    // métodos que não são leitura
    let (st, _, _) = http(
        files,
        &format!("POST {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n"),
    );
    assert_eq!(st, 405);
}

#[test]
fn a_daemon_that_serves_no_files_says_so() {
    let tmp = TempDir::new("files-off");
    let mut engine = engine(&tmp);
    let mut zones = ZonesFile::default();
    let server = IpcServer::bind(&tmp.path("rrv/rrv.sock")).unwrap();
    let sock = tmp.path("rrv/rrv.sock");
    let client = thread::spawn(move || {
        let mut c = IpcClient::connect(&sock).unwrap();
        c.request(&Request::FileUrl {
            file: "a.mkv".into(),
        })
        .unwrap()
    });
    while !client.is_finished() {
        server.poll(&mut Host {
            engine: &mut engine,
            zones_file: &mut zones,
            persist: false,
            history: None,
            recordings: None,
        });
        thread::sleep(Duration::from_millis(10));
    }
    let r = client.join().unwrap();
    assert!(matches!(r, Response::Error { message } if message.contains("não serve arquivos")));
}
