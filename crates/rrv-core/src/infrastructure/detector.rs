//! Runs a YOLO11 ONNX model (feature `detect`). The maths around it — letterbox, decoding, NMS —
//! is `domain::detect`; this file only owns the ONNX Runtime session.
//!
//! The runtime library is loaded at run time (`ort` feature `load-dynamic`): point `ORT_DYLIB_PATH`
//! at a `libonnxruntime.so` — the CPU build, or the CUDA one for the GPU. Nothing links against it,
//! so the same binary runs with either.

use std::path::Path;

use ort::ep;
use ort::session::Session;
use ort::value::Tensor;

use crate::domain::detect::{self, COCO_LABELS, Detection};

/// Where to run the network.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Cpu,
    /// CUDA; `load` fails if the library cannot use it, so the caller decides on the CPU fallback.
    Cuda,
}

pub struct Detector {
    session: Session,
    input_name: String,
    size: u32,
    classes: usize,
}

fn err<E: std::fmt::Display>(what: &str) -> impl FnOnce(E) -> String + '_ {
    move |e| format!("{what}: {e}")
}

impl Detector {
    /// Loads `model` (an ONNX file exported at a fixed square size, e.g. 320 or 640).
    pub fn load(model: &Path, backend: Backend) -> Result<Self, String> {
        let mut builder = Session::builder().map_err(err("sessão ONNX"))?;
        if backend == Backend::Cuda {
            builder = builder
                .with_execution_providers([ep::CUDA::default().build().error_on_failure()])
                .map_err(err("CUDA"))?;
        }
        let session = builder
            .commit_from_file(model)
            .map_err(err("carregando o modelo"))?;

        let input = session.inputs().first().ok_or("modelo sem entrada")?;
        let dims = input
            .dtype()
            .tensor_shape()
            .ok_or("a entrada não é um tensor")?;
        let size = match dims.as_ref() {
            [_, 3, h, w] if h == w && *h > 0 => *h as u32,
            other => return Err(format!("entrada {other:?}: preciso de 1×3×N×N fixo")),
        };
        let output = session.outputs().first().ok_or("modelo sem saída")?;
        let out_dims = output
            .dtype()
            .tensor_shape()
            .ok_or("a saída não é um tensor")?;
        let classes = match out_dims.as_ref() {
            [_, rows, _] if *rows > 4 => (*rows - 4) as usize,
            other => return Err(format!("saída {other:?}: preciso de 1×(4+classes)×N")),
        };
        if classes != COCO_LABELS.len() {
            return Err(format!(
                "o modelo tem {classes} classes, esperava {} (COCO)",
                COCO_LABELS.len()
            ));
        }
        Ok(Self {
            input_name: input.name().to_string(),
            session,
            size,
            classes,
        })
    }

    /// Side of the square the model expects.
    pub fn input_size(&self) -> u32 {
        self.size
    }

    /// Detects objects in an RGBA picture; boxes come back normalised to it.
    pub fn detect_rgba(
        &mut self,
        rgba: &[u8],
        width: u32,
        height: u32,
        min_score: f32,
        iou: f32,
    ) -> Result<Vec<Detection>, String> {
        let (input, lb) = detect::preprocess_rgba(rgba, width, height, self.size)
            .ok_or("quadro incoerente com o tamanho")?;
        let n = self.size as usize;
        let tensor = Tensor::from_array(([1usize, 3, n, n], input)).map_err(err("tensor"))?;
        let outputs = self
            .session
            .run(ort::inputs![self.input_name.as_str() => tensor])
            .map_err(err("inferência"))?;
        let (_, data) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(err("saída"))?;
        Ok(detect::postprocess(data, self.classes, &lb, min_score, iou))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Gabarito: o que a própria Ultralytics detecta em `tests/data/bus.jpg` (810×1080) com o mesmo
    /// modelo exportado, conf 0,25 e IoU 0,45 — `(classe, score, [x, y, w, h] normalizado)`.
    const REFERENCE: [(&str, f32, [f32; 4]); 4] = [
        ("bus", 0.939, [0.015, 0.211, 0.972, 0.469]),
        ("person", 0.902, [0.060, 0.368, 0.240, 0.469]),
        ("person", 0.849, [0.828, 0.364, 0.172, 0.451]),
        ("person", 0.833, [0.275, 0.376, 0.151, 0.421]),
    ];

    fn model_path() -> Option<std::path::PathBuf> {
        let path = std::env::var_os("RRV_TEST_MODEL")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../../models/yolo11n-640.onnx")
            });
        (path.exists() && std::env::var_os("ORT_DYLIB_PATH").is_some()).then_some(path)
    }

    fn iou(a: [f32; 4], d: &Detection) -> f32 {
        let b = Detection {
            class: 0,
            score: 0.0,
            x: a[0],
            y: a[1],
            w: a[2],
            h: a[3],
        };
        b.iou(d)
    }

    #[test]
    fn finds_what_ultralytics_finds_in_the_reference_picture() {
        let Some(model) = model_path() else {
            eprintln!("sem modelo ou sem ORT_DYLIB_PATH (scripts/fetch-model.sh): teste pulado");
            return;
        };
        let img = image::open(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/bus.jpg"))
            .unwrap()
            .to_rgba8();
        let (w, h) = img.dimensions();
        assert_eq!((w, h), (810, 1080));

        let backend = if std::env::var_os("RRV_TEST_CUDA").is_some() {
            Backend::Cuda
        } else {
            Backend::Cpu
        };
        let mut det = Detector::load(&model, backend).unwrap();
        assert_eq!(det.input_size(), 640);
        let found = det.detect_rgba(img.as_raw(), w, h, 0.25, 0.45).unwrap();

        for (label, score, boxed) in REFERENCE {
            let best = found
                .iter()
                .filter(|d| d.label() == label)
                .map(|d| (iou(boxed, d), d))
                .max_by(|a, b| a.0.total_cmp(&b.0));
            let (i, d) = best.unwrap_or_else(|| panic!("não achou {label}: {found:?}"));
            assert!(i >= 0.85, "{label}: IoU {i:.2} com {d:?}");
            assert!(
                (d.score - score).abs() < 0.05,
                "{label}: score {} contra {score}",
                d.score
            );
        }
    }

    #[test]
    fn a_picture_with_nothing_in_it_detects_nothing() {
        let Some(model) = model_path() else {
            eprintln!("sem modelo ou sem ORT_DYLIB_PATH: teste pulado");
            return;
        };
        let mut det = Detector::load(&model, Backend::Cpu).unwrap();
        let grey = vec![128u8; 320 * 240 * 4];
        assert!(
            det.detect_rgba(&grey, 320, 240, 0.25, 0.45)
                .unwrap()
                .is_empty()
        );
        // tamanho incoerente com os bytes: erro, não pânico
        assert!(det.detect_rgba(&grey, 10, 10, 0.25, 0.45).is_err());
    }

    #[test]
    fn a_missing_model_is_an_error_not_a_panic() {
        if std::env::var_os("ORT_DYLIB_PATH").is_none() {
            return;
        }
        assert!(Detector::load(Path::new("/nao/existe.onnx"), Backend::Cpu).is_err());
    }
}
