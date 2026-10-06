//! Utilitários de teste compartilhados (feature `testing`): uma câmera HLS ao
//! vivo simulada só com GStreamer, um diretório temporário e verificações de
//! arquivo. Não faz parte da API do motor.
//!
//! A fonte é em tempo real de propósito: um arquivo é decodificado mais rápido
//! que o relógio e entrega quadros em rajadas, o que torna movimento e gravação
//! não determinísticos (não é como uma câmera se comporta).

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use gstreamer as gst;
use gstreamer::prelude::*;

pub struct TempDir(pub PathBuf);
impl TempDir {
    pub fn new(tag: &str) -> Self {
        let d = std::env::temp_dir().join(format!("rrv-hl-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        Self(d)
    }
    pub fn path(&self, n: &str) -> PathBuf {
        self.0.join(n)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Uma câmera HLS ao vivo: `videotestsrc` em tempo real → `hlssink2` gravando
/// segmentos de 1 s, servidos por um HTTP mínimo em 127.0.0.1.
pub struct LiveCamera {
    pipeline: gst::Pipeline,
    src: gst::Element,
    stop: Arc<AtomicBool>,
    port: u16,
}

impl LiveCamera {
    pub fn start(dir: &Path) -> Self {
        Self::launch(
            dir,
            "videotestsrc name=src is-live=true pattern=ball \
             ! video/x-raw,width=320,height=180,framerate=30/1 \
             ! videoconvert ! x264enc tune=zerolatency key-int-max=30 bitrate=400",
        )
    }

    /// Uma cena de verdade: a foto `jpeg` parada, em 1280×720 com barras (a proporção de uma
    /// câmera), e um relógio no canto que muda a cada quadro — é ele que "mexe" (`set_moving`).
    /// O codificador usa quantizador constante: com taxa variável, cada keyframe (a cada 1 s) refaz a
    /// imagem um pouco diferente e uma cena parada pareceria em movimento de segundo em segundo.
    /// Serve para rodar um detector de objetos de verdade sobre pessoas e veículos reais.
    pub fn start_scene(dir: &Path, jpeg: &Path) -> Self {
        Self::launch(
            dir,
            &format!(
                "filesrc location=\"{}\" ! jpegdec ! imagefreeze is-live=true \
                 ! videoscale add-borders=true ! videoconvert \
                 ! video/x-raw,width=1280,height=720,framerate=10/1 \
                 ! timeoverlay name=src time-mode=running-time halignment=left valignment=top \
                   font-desc=\"Sans 40\" \
                 ! videoconvert \
                 ! x264enc tune=zerolatency key-int-max=10 pass=quant quantizer=18",
                jpeg.display()
            ),
        )
    }

    fn launch(dir: &Path, source: &str) -> Self {
        gst::init().unwrap();
        let hls = dir.join("hls");
        std::fs::create_dir_all(&hls).unwrap();
        let desc = format!(
            "{source} \
             ! h264parse \
             ! hlssink2 location=\"{d}/seg%05d.ts\" playlist-location=\"{d}/live.m3u8\" \
               target-duration=1 max-files=10 playlist-length=6",
            d = hls.display()
        );
        let pipeline = gst::parse::launch(&desc)
            .unwrap()
            .downcast::<gst::Pipeline>()
            .unwrap();
        let src = pipeline.by_name("src").unwrap();
        pipeline.set_state(gst::State::Playing).unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        std::thread::spawn(move || {
            while !flag.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((conn, _)) => {
                        let hls = hls.clone();
                        std::thread::spawn(move || serve(conn, &hls));
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(20)),
                }
            }
        });
        let cam = Self {
            pipeline,
            src,
            stop,
            port,
        };
        // A câmera só "existe" quando já publicou segmentos suficientes.
        let playlist = dir.join("hls/live.m3u8");
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            let ready = std::fs::read_to_string(&playlist)
                .map(|t| t.matches(".ts").count() >= 3)
                .unwrap_or(false);
            if ready {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        cam
    }

    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/live.m3u8", self.port)
    }

    /// A bola se mexe, ou a imagem fica parada.
    pub fn set_moving(&self, moving: bool) {
        if self.src.find_property("pattern").is_some() {
            self.src
                .set_property_from_str("pattern", if moving { "ball" } else { "black" });
        } else {
            // a cena (`start_scene`): o relógio some e sobra a foto parada
            self.src.set_property("silent", !moving);
        }
    }
}

impl Drop for LiveCamera {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

/// HTTP/1.0 mínimo: serve arquivos de `root`.
fn serve(mut conn: TcpStream, root: &Path) {
    let _ = conn.set_read_timeout(Some(Duration::from_secs(5)));
    let mut buf = [0u8; 2048];
    let n = conn.read(&mut buf).unwrap_or(0);
    let req = String::from_utf8_lossy(&buf[..n]);
    let path = req
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .unwrap_or("/")
        .trim_start_matches('/')
        .split('?')
        .next()
        .unwrap_or("")
        .to_string();
    if path.contains("..") {
        let _ = conn.write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n");
        return;
    }
    match std::fs::read(root.join(&path)) {
        Ok(body) => {
            let ctype = if path.ends_with(".m3u8") {
                "application/vnd.apple.mpegurl"
            } else {
                "video/mp2t"
            };
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = conn.write_all(head.as_bytes());
            let _ = conn.write_all(&body);
        }
        Err(_) => {
            let _ = conn.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
        }
    }
}

pub fn mkv_files(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<_> = std::fs::read_dir(dir)
        .map(|d| d.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    v.retain(|p| p.extension().is_some_and(|x| x == "mkv"));
    v.sort();
    v
}

pub fn is_playable(path: &Path) -> bool {
    let desc = format!(
        "filesrc location=\"{}\" ! decodebin ! fakesink sync=false",
        path.display()
    );
    let p = gst::parse::launch(&desc).unwrap();
    if p.set_state(gst::State::Playing).is_err() {
        return false;
    }
    let ok = p
        .bus()
        .and_then(|b| {
            b.timed_pop_filtered(
                gst::ClockTime::from_seconds(10),
                &[gst::MessageType::Eos, gst::MessageType::Error],
            )
        })
        .is_some_and(|m| matches!(m.view(), gst::MessageView::Eos(_)));
    let _ = p.set_state(gst::State::Null);
    ok
}
