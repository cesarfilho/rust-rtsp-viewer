//! Teste de estresse (tarefa 2.8 do plano): 16 câmeras sintéticas sob uma
//! tempestade de reconexões, procurando vazamento de threads, descritores de
//! arquivo e memória.
//!
//! Fica `#[ignore]`: demora. Rodar com
//!
//! ```text
//! cargo test --release --test stress -- --ignored --nocapture
//! RRV_STRESS_SECS=3600 cargo test --release --test stress -- --ignored --nocapture   # critério de 1 h
//! ```

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use gstreamer as gst;
use gstreamer::prelude::*;
use rust_rtsp_viewer::config::CameraConfig;
use rust_rtsp_viewer::ui::bridge::GStreamerBridge;

const CAMERAS: usize = 16;

fn make_clip(path: &Path) {
    gst::init().unwrap();
    let desc = format!(
        "videotestsrc num-buffers=300 pattern=ball ! video/x-raw,width=640,height=360,framerate=30/1 \
         ! videoconvert ! x264enc tune=zerolatency key-int-max=15 ! h264parse \
         ! matroskamux ! filesink location=\"{}\"",
        path.display()
    );
    let pipeline = gst::parse::launch(&desc).unwrap();
    pipeline.set_state(gst::State::Playing).unwrap();
    let msg = pipeline.bus().unwrap().timed_pop_filtered(
        gst::ClockTime::from_seconds(30),
        &[gst::MessageType::Eos, gst::MessageType::Error],
    );
    pipeline.set_state(gst::State::Null).unwrap();
    assert!(matches!(
        msg.as_ref().map(|m| m.view()),
        Some(gst::MessageView::Eos(_))
    ));
}

/// Gerador pseudoaleatório mínimo (xorshift), para não puxar dependência.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn between(&mut self, lo: u64, hi: u64) -> u64 {
        lo + self.next() % (hi - lo + 1)
    }
}

#[derive(Clone, Copy, Debug)]
struct Sample {
    at_secs: u64,
    threads: u64,
    fds: u64,
    rss_kb: u64,
}

fn proc_status(key: &str) -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .unwrap()
        .lines()
        .find(|l| l.starts_with(key))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

fn sample(start: Instant) -> Sample {
    Sample {
        at_secs: start.elapsed().as_secs(),
        threads: proc_status("Threads:"),
        fds: std::fs::read_dir("/proc/self/fd")
            .map(|d| d.count() as u64)
            .unwrap_or(0),
        rss_kb: proc_status("VmRSS:"),
    }
}

fn camera(path: &Path) -> CameraConfig {
    toml::from_str(&format!("url = {:?}", path.to_string_lossy())).unwrap()
}

#[test]
#[ignore = "demorado: rode com --ignored (RRV_STRESS_SECS ajusta a duração)"]
fn sixteen_cameras_survive_a_reconnect_storm_without_leaking() {
    let secs: u64 = std::env::var("RRV_STRESS_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60);

    let dir: PathBuf = std::env::temp_dir().join(format!("rrv-stress-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let clip = dir.join("clip.mkv");
    make_clip(&clip);

    let stop = Arc::new(AtomicBool::new(false));
    let cycles = Arc::new(AtomicU64::new(0));
    let failures = Arc::new(AtomicU64::new(0));
    let recordings = Arc::new(AtomicU64::new(0));

    let workers: Vec<_> = (0..CAMERAS)
        .map(|i| {
            let (stop, cycles, failures, recordings) = (
                stop.clone(),
                cycles.clone(),
                failures.clone(),
                recordings.clone(),
            );
            let cfg = camera(&clip);
            let rec_dir = dir.join(format!("rec-{i}"));
            std::thread::spawn(move || {
                let mut bridge = GStreamerBridge::new(640, 360).unwrap();
                bridge.recording_config.dir = rec_dir;
                // Metade das câmeras também roda o ramo de detecção.
                bridge.detect_enabled = i % 2 == 0;
                let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ (i as u64 + 1));
                while !stop.load(Ordering::Relaxed) {
                    if bridge.start_from_config(&cfg).is_err() {
                        failures.fetch_add(1, Ordering::Relaxed);
                    }
                    std::thread::sleep(Duration::from_millis(rng.between(150, 900)));
                    bridge.poll_bus();
                    // Uma em cada 4 reconexões liga a gravação no meio.
                    if i % 4 == 0 && rng.between(0, 3) == 0 {
                        if bridge.start_recording().is_ok() {
                            recordings.fetch_add(1, Ordering::Relaxed);
                        }
                        std::thread::sleep(Duration::from_millis(rng.between(100, 500)));
                    }
                    // A reconexão: `stop` finaliza a gravação se houver.
                    bridge.stop();
                    cycles.fetch_add(1, Ordering::Relaxed);
                }
                bridge.stop();
            })
        })
        .collect();

    let start = Instant::now();
    let mut samples = vec![sample(start)];
    while start.elapsed() < Duration::from_secs(secs) {
        std::thread::sleep(Duration::from_secs(5.min(secs.max(1))));
        let s = sample(start);
        println!(
            "t={:>4}s threads={:<4} fds={:<4} rss={:>7} kB cycles={} rec={} fail={}",
            s.at_secs,
            s.threads,
            s.fds,
            s.rss_kb,
            cycles.load(Ordering::Relaxed),
            recordings.load(Ordering::Relaxed),
            failures.load(Ordering::Relaxed)
        );
        samples.push(s);
    }

    stop.store(true, Ordering::Relaxed);
    for w in workers {
        w.join().expect("a worker thread panicked");
    }
    std::thread::sleep(Duration::from_secs(1));
    let end = sample(start);
    println!(
        "fim: threads={} fds={} rss={} kB, {} ciclos",
        end.threads,
        end.fds,
        end.rss_kb,
        cycles.load(Ordering::Relaxed)
    );
    let _ = std::fs::remove_dir_all(&dir);

    // Aquecimento: o primeiro terço da corrida fica de fora (pools do
    // GStreamer, alocador e caches se estabilizam). Do restante compara a
    // primeira metade com a segunda. As threads oscilam com a fase de
    // montagem/desmontagem dos pipelines, então a tolerância é proporcional.
    let warm: Vec<_> = samples.iter().skip(samples.len() / 3).copied().collect();
    let (early, late) = warm.split_at((warm.len() / 2).max(1));
    let peak = |xs: &[Sample], f: fn(&Sample) -> u64| xs.iter().map(f).max().unwrap_or(0);
    let mean = |xs: &[Sample], f: fn(&Sample) -> u64| {
        xs.iter().map(f).sum::<u64>() / xs.len().max(1) as u64
    };

    assert!(
        cycles.load(Ordering::Relaxed) > 50,
        "a tempestade quase não rodou"
    );
    let (t0, t1) = (peak(early, |s| s.threads), peak(late, |s| s.threads));
    assert!(t1 <= t0 + t0 / 4 + 16, "threads crescendo: {t0} → {t1}");
    let (f0, f1) = (peak(early, |s| s.fds), peak(late, |s| s.fds));
    assert!(f1 <= f0 + 16, "descritores crescendo: {f0} → {f1}");
    let (m0, m1) = (mean(early, |s| s.rss_kb), mean(late, |s| s.rss_kb));
    println!("RSS média: {m0} kB (1ª metade) → {m1} kB (2ª metade)");
    assert!(
        m1 <= m0 + m0 / 5,
        "memória crescendo mais de 20%: {m0} kB → {m1} kB"
    );
    // Depois de parar tudo, as threads dos pipelines têm de ter ido embora.
    assert!(
        end.threads <= samples[0].threads + 8,
        "threads sobraram após parar tudo: {} (início: {})",
        end.threads,
        samples[0].threads
    );
}
