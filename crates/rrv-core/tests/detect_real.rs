//! A detecção com o **modelo de verdade** (plano C4): uma câmera HLS ao vivo mostra uma foto real
//! (ônibus e pessoas) com um relógio no canto que "mexe"; o motor roda o YOLO11 e o histórico tem
//! de ganhar eventos `detection` com os rótulos certos.
//!
//! Precisa de `--features detect`, do modelo (`scripts/fetch-model.sh`) e de `ORT_DYLIB_PATH`; sem eles
//! o teste avisa e passa. `RRV_TEST_MODEL` troca o modelo, `RRV_TEST_CUDA=1` roda na GPU.
#![cfg(feature = "detect")]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rrv_core::config::{CameraConfig, LogsConfigFile};
use rrv_core::domain::motion::MotionConfig;
use rrv_core::domain::notify::NotifyConfig;
use rrv_core::domain::recording::RecordingConfig;
use rrv_core::engine::inference::InferenceWorker;
use rrv_core::engine::{Engine, EngineSettings, TICK_MS};
use rrv_core::infrastructure::detector::{Backend, Detector};
use rrv_core::infrastructure::store::{Store, StoreHandle};
use rrv_core::infrastructure::zone_state::ZonesFile;
use rrv_core::testing::{LiveCamera, TempDir};

fn model() -> Option<PathBuf> {
    let path = std::env::var_os("RRV_TEST_MODEL")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../models/yolo11n-640.onnx")
        });
    (path.exists() && std::env::var_os("ORT_DYLIB_PATH").is_some()).then_some(path)
}

fn engine(url: &str, rec: &Path) -> Engine {
    let cam: CameraConfig = toml::from_str(&format!("url = {url:?}\nname = \"cam\"")).unwrap();
    let recording = RecordingConfig {
        dir: rec.to_path_buf(),
        ..RecordingConfig::default()
    };
    let logs = LogsConfigFile {
        dir: Some(rec.join("logs")),
        ..LogsConfigFile::default()
    };
    let mut e = Engine::new(EngineSettings {
        cameras: &[cam],
        recording: &recording,
        motion: MotionConfig {
            sample_stride: 2,
            contour_area: 0.002,
            threshold: 20,
            ..MotionConfig::default()
        },
        notify: NotifyConfig::default(),
        logs: &logs,
        pause_hidden: true,
        stagger: Duration::from_millis(100),
        zones: &ZonesFile::default(),
    });
    e.set_headless();
    e
}

fn run(e: &mut Engine, secs: u64, mut done: impl FnMut(&Engine) -> bool) -> bool {
    let end = Instant::now() + Duration::from_secs(secs);
    let mut tick = 0;
    while Instant::now() < end {
        let _ = e.step(tick);
        tick += 1;
        if done(e) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(TICK_MS));
    }
    false
}

#[test]
fn the_real_model_finds_the_bus_and_the_people_through_the_whole_engine() {
    let Some(model) = model() else {
        eprintln!("sem modelo ou sem ORT_DYLIB_PATH (scripts/fetch-model.sh): teste pulado");
        return;
    };
    let jpeg = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/bus.jpg");
    let tmp = TempDir::new("detect-real");
    let cam = LiveCamera::start_scene(&tmp.0, &jpeg);
    let (rec, db) = (tmp.path("rec"), tmp.path("history.db"));

    let backend = if std::env::var_os("RRV_TEST_CUDA").is_some() {
        Backend::Cuda
    } else {
        Backend::Cpu
    };
    let detector = Detector::load(&model, backend).unwrap();
    let mut e = engine(&cam.url(), &rec);
    e.set_store(StoreHandle::spawn(db.clone()).unwrap());
    e.set_detect_policy(Vec::new(), 5);
    e.set_inference(InferenceWorker::start(detector, 2));

    let seen = run(&mut e, 90, |e| {
        e.detections[0].iter().any(|d| d.label() == "bus")
            && e.detections[0].iter().any(|d| d.label() == "person")
    });
    let stats = e.inference.as_ref().unwrap().stats();
    let found: Vec<String> = e.detections[0]
        .iter()
        .map(|d| format!("{} {:.0}%", d.label(), d.score * 100.0))
        .collect();
    assert!(
        seen,
        "o modelo real não achou ônibus e pessoa: viu {found:?}; inferências {stats:?}"
    );
    eprintln!("viu {found:?}; inferências {stats:?}");

    // a cena para. O HLS tem alguns segundos de atraso, então primeiro espera o contador
    // parar de subir por 6 s seguidos; depois, 8 s depois ele continua igual (ocioso de verdade)
    cam.set_moving(false);
    let mut last = e.inference.as_ref().unwrap().stats().inferred;
    let mut still_since = Instant::now();
    let deadline = Instant::now() + Duration::from_secs(60);
    while still_since.elapsed() < Duration::from_secs(6) && Instant::now() < deadline {
        run(&mut e, 1, |_| false);
        let now = e.inference.as_ref().unwrap().stats().inferred;
        if now != last {
            (last, still_since) = (now, Instant::now());
        }
    }
    let settled = e.inference.as_ref().unwrap().stats().inferred;
    run(&mut e, 8, |_| false);
    let after = e.inference.as_ref().unwrap().stats();
    e.shutdown();
    drop(e);
    assert_eq!(
        after.inferred, settled,
        "o modelo continuou rodando com a cena parada: {after:?}"
    );
    assert_eq!(after.errors, 0);

    // e o histórico tem os eventos, com rótulo, confiança e caixa
    let events = Store::open(&db)
        .unwrap()
        .events_between(Some("cam"), 0, i64::MAX)
        .unwrap();
    let detections: Vec<_> = events.iter().filter(|x| x.kind == "detection").collect();
    for label in ["bus", "person"] {
        let d = detections
            .iter()
            .find(|d| d.label == label)
            .unwrap_or_else(|| panic!("sem {label} no histórico: {detections:?}"));
        assert!(d.score.is_some_and(|s| s > 0.25), "{d:?}");
        let [x, y, w, h] = d.bbox.expect("caixa");
        assert!(
            (0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y) && w > 0.0 && h > 0.0,
            "{d:?}"
        );
    }
    eprintln!("tempo de inferência médio: {:.0} ms", after.avg_ms);
}
