//! Object detection, the pure half: everything around the neural network that needs no runtime.
//!
//! A YOLO11 ONNX model takes a `1×3×N×N` float tensor (RGB, 0–1, the picture *letterboxed* into an
//! N×N square on grey 114) and returns `1×(4+80)×M`: for each of the `M` candidate boxes, its centre
//! and size in that square's pixels, then one score per COCO class. This module prepares the input,
//! turns the output back into boxes of the *original* picture and applies non-maximum suppression.
//! Running the network itself lives in `infrastructure::detector` (feature `detect`).

/// The 80 COCO classes, in the order YOLO's scores come out.
pub const COCO_LABELS: [&str; 80] = [
    "person",
    "bicycle",
    "car",
    "motorcycle",
    "airplane",
    "bus",
    "train",
    "truck",
    "boat",
    "traffic light",
    "fire hydrant",
    "stop sign",
    "parking meter",
    "bench",
    "bird",
    "cat",
    "dog",
    "horse",
    "sheep",
    "cow",
    "elephant",
    "bear",
    "zebra",
    "giraffe",
    "backpack",
    "umbrella",
    "handbag",
    "tie",
    "suitcase",
    "frisbee",
    "skis",
    "snowboard",
    "sports ball",
    "kite",
    "baseball bat",
    "baseball glove",
    "skateboard",
    "surfboard",
    "tennis racket",
    "bottle",
    "wine glass",
    "cup",
    "fork",
    "knife",
    "spoon",
    "bowl",
    "banana",
    "apple",
    "sandwich",
    "orange",
    "broccoli",
    "carrot",
    "hot dog",
    "pizza",
    "donut",
    "cake",
    "chair",
    "couch",
    "potted plant",
    "bed",
    "dining table",
    "toilet",
    "tv",
    "laptop",
    "mouse",
    "remote",
    "keyboard",
    "cell phone",
    "microwave",
    "oven",
    "toaster",
    "sink",
    "refrigerator",
    "book",
    "clock",
    "vase",
    "scissors",
    "teddy bear",
    "hair drier",
    "toothbrush",
];

/// The class id of an English COCO name (case-insensitive).
pub fn class_id(name: &str) -> Option<usize> {
    COCO_LABELS
        .iter()
        .position(|l| l.eq_ignore_ascii_case(name.trim()))
}

/// What the person reads for a class: the Portuguese name for the common ones (while the interface is in
/// Portuguese), otherwise the English COCO name.
pub fn label_pt(label: &str) -> &str {
    if crate::i18n::get() == crate::i18n::Lang::En {
        return label;
    }
    match label {
        "person" => "pessoa",
        "bicycle" => "bicicleta",
        "car" => "carro",
        "motorcycle" => "moto",
        "bus" => "ônibus",
        "truck" => "caminhão",
        "bird" => "pássaro",
        "cat" => "gato",
        "dog" => "cachorro",
        "backpack" => "mochila",
        "handbag" => "bolsa",
        "suitcase" => "mala",
        other => other,
    }
}

/// Grey the model was trained to see around a letterboxed picture.
const PAD_VALUE: f32 = 114.0 / 255.0;

/// How a picture was fitted into the network's square, to map boxes back.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Letterbox {
    /// Side of the network input.
    pub size: u32,
    pub scale: f32,
    /// Offset of the picture inside the square, in square pixels.
    pub pad_x: f32,
    pub pad_y: f32,
    /// The original picture.
    pub src_w: u32,
    pub src_h: u32,
}

impl Letterbox {
    /// Fits `src_w × src_h` into `size × size` keeping the aspect ratio, centred.
    pub fn new(src_w: u32, src_h: u32, size: u32) -> Option<Self> {
        if src_w == 0 || src_h == 0 || size == 0 {
            return None;
        }
        let scale = (size as f32 / src_w as f32).min(size as f32 / src_h as f32);
        Some(Self {
            size,
            scale,
            pad_x: (size as f32 - src_w as f32 * scale) / 2.0,
            pad_y: (size as f32 - src_h as f32 * scale) / 2.0,
            src_w,
            src_h,
        })
    }
}

/// A detected object, normalised to the original picture (`0..=1`, top-left origin).
#[derive(Debug, Clone, PartialEq)]
pub struct Detection {
    pub class: usize,
    pub score: f32,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Detection {
    pub fn label(&self) -> &'static str {
        COCO_LABELS.get(self.class).copied().unwrap_or("?")
    }

    /// Intersection over union with another box.
    pub fn iou(&self, other: &Self) -> f32 {
        let x1 = self.x.max(other.x);
        let y1 = self.y.max(other.y);
        let x2 = (self.x + self.w).min(other.x + other.w);
        let y2 = (self.y + self.h).min(other.y + other.h);
        let inter = (x2 - x1).max(0.0) * (y2 - y1).max(0.0);
        let union = self.w * self.h + other.w * other.h - inter;
        if union <= 0.0 { 0.0 } else { inter / union }
    }
}

/// `[detect]` as written in the TOML.
#[derive(Debug, serde::Deserialize, Clone, Default)]
pub struct DetectFile {
    /// Run object detection on the frames where motion is seen (needs `[motion] enabled`). Default: false.
    pub enabled: Option<bool>,
    /// Path of the ONNX model (`scripts/fetch-model.sh`). Default: `models/yolo11n-640.onnx`.
    pub model: Option<String>,
    /// `"cpu"`, `"cuda"` or `"auto"` (CUDA, falling back to the CPU). Default: `"auto"`.
    pub backend: Option<String>,
    /// Lowest score kept (0.05–0.99). Default: 0.25.
    pub min_score: Option<f32>,
    /// NMS overlap threshold (0.1–0.9). Default: 0.45.
    pub iou: Option<f32>,
    /// Which COCO classes raise events, e.g. `["person", "car"]` (English names). Default: all.
    pub labels: Option<Vec<String>>,
    /// Minimum seconds between two events of one class on one camera (5–3600). Default: 30.
    pub cooldown_secs: Option<u64>,
    /// Record a clip while a wanted class is seen (pre-roll and post-roll from `[recording]`),
    /// instead of on every motion. Default: false.
    pub record: Option<bool>,
    /// Save a JPEG of the whole picture, with the boxes drawn, on every detection event
    /// (`<recording dir>/snapshots/<camera>/<date>/`). Default: false.
    pub snapshot: Option<bool>,
    /// Only these cameras (by `label`, else `name`; case ignored) go to the model. Default: all.
    /// A camera whose scene never stops moving (water, trees, traffic) would keep it busy.
    pub cameras: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendChoice {
    Cpu,
    Cuda,
    Auto,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DetectConfig {
    pub enabled: bool,
    pub model: std::path::PathBuf,
    pub backend: BackendChoice,
    pub min_score: f32,
    pub iou: f32,
    /// Class ids that raise events; empty = every class.
    pub labels: Vec<usize>,
    pub cooldown_secs: u64,
    /// Record while a wanted class is seen.
    pub record: bool,
    /// A JPEG per detection event.
    pub snapshot: bool,
    /// Camera names the model looks at; empty = every camera.
    pub cameras: Vec<String>,
}

impl Default for DetectConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            model: "models/yolo11n-640.onnx".into(),
            backend: BackendChoice::Auto,
            min_score: 0.25,
            iou: 0.45,
            labels: Vec::new(),
            cooldown_secs: 30,
            record: false,
            snapshot: false,
            cameras: Vec::new(),
        }
    }
}

impl DetectFile {
    pub fn into_config(self) -> Result<DetectConfig, String> {
        let d = DetectConfig::default();
        let backend = match self
            .backend
            .as_deref()
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            None | Some("auto") => BackendChoice::Auto,
            Some("cpu") => BackendChoice::Cpu,
            Some("cuda") => BackendChoice::Cuda,
            Some(other) => return Err(format!("backend = \"{other}\": use cpu, cuda ou auto")),
        };
        let min_score = self.min_score.unwrap_or(d.min_score);
        if !(0.05..=0.99).contains(&min_score) {
            return Err(format!("min_score = {min_score} fora de 0.05–0.99"));
        }
        let iou = self.iou.unwrap_or(d.iou);
        if !(0.1..=0.9).contains(&iou) {
            return Err(format!("iou = {iou} fora de 0.1–0.9"));
        }
        let mut labels = Vec::new();
        for name in self.labels.iter().flatten() {
            let id = class_id(name).ok_or_else(|| {
                format!("labels: \"{name}\" não é uma classe COCO (exemplos: person, car, dog)")
            })?;
            if !labels.contains(&id) {
                labels.push(id);
            }
        }
        let cooldown_secs = self.cooldown_secs.unwrap_or(d.cooldown_secs);
        if !(5..=3600).contains(&cooldown_secs) {
            return Err(format!("cooldown_secs = {cooldown_secs} fora de 5–3600"));
        }
        Ok(DetectConfig {
            labels,
            cooldown_secs,
            enabled: self.enabled.unwrap_or(false),
            model: self.model.map(Into::into).unwrap_or(d.model),
            backend,
            min_score,
            iou,
            record: self.record.unwrap_or(false),
            snapshot: self.snapshot.unwrap_or(false),
            cameras: self.cameras.unwrap_or_default(),
        })
    }
}

/// A rectangle of the picture, normalised (`0..=1`, top-left origin).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Region {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Region {
    pub const FULL: Self = Self {
        x: 0.0,
        y: 0.0,
        w: 1.0,
        h: 1.0,
    };

    /// The smallest rectangle holding every vertex of the **active** zones, or `None` when no zone
    /// restricts detection (then the whole frame is looked at).
    pub fn of_zones(zones: &crate::domain::zones::ZoneConfig) -> Option<Self> {
        let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for z in zones.zones.iter().filter(|z| z.is_active()) {
            for v in &z.vertices {
                x0 = x0.min(v.x);
                y0 = y0.min(v.y);
                x1 = x1.max(v.x);
                y1 = y1.max(v.y);
            }
        }
        if x0 > x1 || y0 > y1 {
            return None;
        }
        let (x0, y0, x1, y1) = (
            x0.clamp(0.0, 1.0) as f32,
            y0.clamp(0.0, 1.0) as f32,
            x1.clamp(0.0, 1.0) as f32,
            y1.clamp(0.0, 1.0) as f32,
        );
        (x1 > x0 && y1 > y0).then_some(Self {
            x: x0,
            y: y0,
            w: x1 - x0,
            h: y1 - y0,
        })
    }

    /// Cuts the region out of an RGBA picture. `None` for a size/bytes mismatch or a region that
    /// leaves less than one pixel.
    pub fn crop_rgba(&self, rgba: &[u8], width: u32, height: u32) -> Option<(Vec<u8>, u32, u32)> {
        let (w, h) = (width as usize, height as usize);
        if w == 0 || h == 0 || rgba.len() != w * h * 4 {
            return None;
        }
        let x0 = ((self.x * w as f32).floor() as usize).min(w - 1);
        let y0 = ((self.y * h as f32).floor() as usize).min(h - 1);
        let x1 = (((self.x + self.w) * w as f32).ceil() as usize).clamp(x0 + 1, w);
        let y1 = (((self.y + self.h) * h as f32).ceil() as usize).clamp(y0 + 1, h);
        let (cw, ch) = (x1 - x0, y1 - y0);
        let mut out = Vec::with_capacity(cw * ch * 4);
        for row in y0..y1 {
            out.extend_from_slice(&rgba[(row * w + x0) * 4..(row * w + x1) * 4]);
        }
        Some((out, cw as u32, ch as u32))
    }

    /// Where the crop actually starts, in the picture's own terms (the crop is cut on whole pixels).
    pub fn snapped(&self, width: u32, height: u32) -> Self {
        let (w, h) = (width as f32, height as f32);
        let x0 = (self.x * w).floor().min(w - 1.0);
        let y0 = (self.y * h).floor().min(h - 1.0);
        let x1 = ((self.x + self.w) * w).ceil().clamp(x0 + 1.0, w);
        let y1 = ((self.y + self.h) * h).ceil().clamp(y0 + 1.0, h);
        Self {
            x: x0 / w,
            y: y0 / h,
            w: (x1 - x0) / w,
            h: (y1 - y0) / h,
        }
    }
}

impl Detection {
    /// A box found inside a crop (normalised to the crop) → normalised to the whole picture.
    pub fn from_crop(&self, region: &Region) -> Self {
        Self {
            x: region.x + self.x * region.w,
            y: region.y + self.y * region.h,
            w: self.w * region.w,
            h: self.h * region.h,
            ..self.clone()
        }
    }

    /// The box centre, normalised — the point zones are tested against.
    pub fn centre(&self) -> (f32, f32) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
}

/// RGBA picture → the `3×N×N` input tensor (CHW, RGB, 0–1), bilinear, letterboxed on grey.
/// `None` when the byte count disagrees with the size.
pub fn preprocess_rgba(
    rgba: &[u8],
    width: u32,
    height: u32,
    size: u32,
) -> Option<(Vec<f32>, Letterbox)> {
    let lb = Letterbox::new(width, height, size)?;
    if rgba.len() != width as usize * height as usize * 4 {
        return None;
    }
    let n = size as usize;
    let (w, h) = (width as usize, height as usize);
    let plane = n * n;
    let mut out = vec![PAD_VALUE; 3 * plane];
    let x0 = lb.pad_x.round() as usize;
    let y0 = lb.pad_y.round() as usize;
    let dst_w = ((width as f32 * lb.scale).round() as usize).clamp(1, n);
    let dst_h = ((height as f32 * lb.scale).round() as usize).clamp(1, n);
    for dy in 0..dst_h {
        let py = y0 + dy;
        if py >= n {
            break;
        }
        // pixel-centre mapping, so a downscale does not drift by half a pixel
        let sy = ((dy as f32 + 0.5) / lb.scale - 0.5).clamp(0.0, (h - 1) as f32);
        let (iy, fy) = (sy as usize, sy.fract());
        let iy1 = (iy + 1).min(h - 1);
        for dx in 0..dst_w {
            let px = x0 + dx;
            if px >= n {
                break;
            }
            let sx = ((dx as f32 + 0.5) / lb.scale - 0.5).clamp(0.0, (w - 1) as f32);
            let (ix, fx) = (sx as usize, sx.fract());
            let ix1 = (ix + 1).min(w - 1);
            for c in 0..3 {
                let at = |yy: usize, xx: usize| rgba[(yy * w + xx) * 4 + c] as f32;
                let top = at(iy, ix) * (1.0 - fx) + at(iy, ix1) * fx;
                let bottom = at(iy1, ix) * (1.0 - fx) + at(iy1, ix1) * fx;
                out[c * plane + py * n + px] = (top * (1.0 - fy) + bottom * fy) / 255.0;
            }
        }
    }
    Some((out, lb))
}

/// Turns the network output (`(4 + classes) × candidates`, row-major: all `cx`, then all `cy`,
/// …) into boxes of the original picture, keeping the best class of each candidate at or above
/// `min_score`. No suppression yet — see [`nms`].
pub fn decode(output: &[f32], classes: usize, lb: &Letterbox, min_score: f32) -> Vec<Detection> {
    let rows = 4 + classes;
    if classes == 0 || output.is_empty() || !output.len().is_multiple_of(rows) {
        return Vec::new();
    }
    let m = output.len() / rows;
    let at = |row: usize, i: usize| output[row * m + i];
    let (sw, sh) = (lb.src_w as f32, lb.src_h as f32);
    let mut found = Vec::new();
    for i in 0..m {
        let (mut class, mut score) = (0, f32::MIN);
        for c in 0..classes {
            let s = at(4 + c, i);
            if s > score {
                (class, score) = (c, s);
            }
        }
        if score < min_score {
            continue;
        }
        let (cx, cy, w, h) = (at(0, i), at(1, i), at(2, i), at(3, i));
        // square pixels → original pixels → normalised, clipped to the picture
        let left = ((cx - w / 2.0 - lb.pad_x) / lb.scale / sw).clamp(0.0, 1.0);
        let top = ((cy - h / 2.0 - lb.pad_y) / lb.scale / sh).clamp(0.0, 1.0);
        let right = ((cx + w / 2.0 - lb.pad_x) / lb.scale / sw).clamp(0.0, 1.0);
        let bottom = ((cy + h / 2.0 - lb.pad_y) / lb.scale / sh).clamp(0.0, 1.0);
        if right <= left || bottom <= top {
            continue;
        }
        found.push(Detection {
            class,
            score,
            x: left,
            y: top,
            w: right - left,
            h: bottom - top,
        });
    }
    found
}

/// Non-maximum suppression, per class: of boxes that overlap more than `iou_threshold`, only the
/// highest score stays. Result is sorted by score, best first.
pub fn nms(mut dets: Vec<Detection>, iou_threshold: f32) -> Vec<Detection> {
    dets.sort_by(|a, b| b.score.total_cmp(&a.score));
    let mut kept: Vec<Detection> = Vec::new();
    for d in dets {
        if kept
            .iter()
            .all(|k| k.class != d.class || k.iou(&d) <= iou_threshold)
        {
            kept.push(d);
        }
    }
    kept
}

/// `decode` + `nms`: the whole post-processing of one inference.
pub fn postprocess(
    output: &[f32],
    classes: usize,
    lb: &Letterbox,
    min_score: f32,
    iou_threshold: f32,
) -> Vec<Detection> {
    nms(decode(output, classes, lb, min_score), iou_threshold)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn det(class: usize, score: f32, x: f32, y: f32, w: f32, h: f32) -> Detection {
        Detection {
            class,
            score,
            x,
            y,
            w,
            h,
        }
    }

    #[test]
    fn there_are_eighty_classes_and_the_first_is_person() {
        assert_eq!(COCO_LABELS.len(), 80);
        assert_eq!(COCO_LABELS[0], "person");
        assert_eq!(COCO_LABELS[2], "car");
        assert_eq!(COCO_LABELS[79], "toothbrush");
    }

    #[test]
    fn a_wide_picture_is_padded_above_and_below() {
        let lb = Letterbox::new(1920, 1080, 640).unwrap();
        assert!((lb.scale - 1.0 / 3.0).abs() < 1e-6);
        assert_eq!(lb.pad_x, 0.0);
        assert!((lb.pad_y - 140.0).abs() < 1e-3, "{}", lb.pad_y);
        assert!(Letterbox::new(0, 10, 640).is_none());
    }

    #[test]
    fn preprocessing_pads_with_grey_and_keeps_the_picture_in_the_middle() {
        // 4×2 white picture into 8×8: scale 2, 8×4 content, 2 rows of grey above and below
        let rgba = vec![255u8; 4 * 2 * 4];
        let (t, lb) = preprocess_rgba(&rgba, 4, 2, 8).unwrap();
        assert_eq!(t.len(), 3 * 64);
        assert_eq!(lb.pad_y, 2.0);
        for c in 0..3 {
            assert!((t[c * 64] - PAD_VALUE).abs() < 1e-6, "topo cinza");
            assert!((t[c * 64 + 3 * 8 + 4] - 1.0).abs() < 1e-5, "meio branco");
            assert!((t[c * 64 + 7 * 8] - PAD_VALUE).abs() < 1e-6, "base cinza");
        }
    }

    #[test]
    fn preprocessing_keeps_channels_apart() {
        // uma imagem 2×2 toda vermelha: R=1, G=B=0
        let rgba: Vec<u8> = [255, 0, 0, 255].repeat(4);
        let (t, _) = preprocess_rgba(&rgba, 2, 2, 4).unwrap();
        assert!((t[5] - 1.0).abs() < 1e-5);
        assert!(t[16 + 5].abs() < 1e-5 && t[32 + 5].abs() < 1e-5);
    }

    #[test]
    fn a_wrong_byte_count_is_refused() {
        assert!(preprocess_rgba(&[0; 7], 2, 2, 4).is_none());
    }

    /// Builds a `(4+classes)×m` output with a single candidate set.
    fn output(classes: usize, m: usize, cands: &[(usize, [f32; 4], usize, f32)]) -> Vec<f32> {
        let mut o = vec![0.0; (4 + classes) * m];
        for &(i, b, class, score) in cands {
            for (r, v) in b.iter().enumerate() {
                o[r * m + i] = *v;
            }
            o[(4 + class) * m + i] = score;
        }
        o
    }

    #[test]
    fn a_box_maps_back_to_the_original_picture() {
        // 1920×1080 → 640 (scale 1/3, pad_y 140). Uma caixa de 100×100 px no quadrado, centro (320, 320):
        // original: x 660..960 ... em pixels do original = (220/ (1/3)) .. ; normalizado /1920
        let lb = Letterbox::new(1920, 1080, 640).unwrap();
        let o = output(80, 5, &[(2, [320.0, 320.0, 100.0, 100.0], 0, 0.9)]);
        let d = decode(&o, 80, &lb, 0.25);
        assert_eq!(d.len(), 1);
        let d = &d[0];
        assert_eq!((d.class, d.label()), (0, "person"));
        assert!((d.x - 270.0 * 3.0 / 1920.0).abs() < 1e-4, "{}", d.x);
        assert!((d.w - 300.0 / 1920.0).abs() < 1e-4);
        assert!(
            (d.y - (270.0 - 140.0) * 3.0 / 1080.0).abs() < 1e-4,
            "{}",
            d.y
        );
        assert!((d.h - 300.0 / 1080.0).abs() < 1e-4);
    }

    #[test]
    fn low_scores_are_dropped_and_the_best_class_wins() {
        let lb = Letterbox::new(640, 640, 640).unwrap();
        let mut o = output(80, 3, &[(0, [100.0, 100.0, 50.0, 50.0], 2, 0.4)]);
        o[(4 + 7) * 3] = 0.6; // a mesma caixa também é "truck", com mais score
        o[(4 + 1) * 3 + 1] = 0.1; // o candidato 1 é fraco demais
        let d = decode(&o, 80, &lb, 0.25);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].label(), "truck");
    }

    #[test]
    fn boxes_are_clipped_to_the_picture_and_empty_ones_dropped() {
        let lb = Letterbox::new(640, 640, 640).unwrap();
        // metade da caixa para fora do canto superior esquerdo
        let o = output(80, 2, &[(0, [0.0, 0.0, 100.0, 100.0], 0, 0.9)]);
        let d = decode(&o, 80, &lb, 0.25);
        assert_eq!((d[0].x, d[0].y), (0.0, 0.0));
        assert!((d[0].w - 50.0 / 640.0).abs() < 1e-5);
        // inteiramente fora: some
        let o = output(80, 2, &[(0, [-100.0, -100.0, 20.0, 20.0], 0, 0.9)]);
        assert!(decode(&o, 80, &lb, 0.25).is_empty());
    }

    #[test]
    fn a_malformed_output_yields_nothing() {
        let lb = Letterbox::new(640, 640, 640).unwrap();
        assert!(decode(&[], 80, &lb, 0.1).is_empty());
        assert!(decode(&[0.0; 83], 80, &lb, 0.1).is_empty());
        assert!(decode(&[0.0; 84], 0, &lb, 0.1).is_empty());
    }

    #[test]
    fn iou_of_identical_disjoint_and_half_overlapping_boxes() {
        let a = det(0, 1.0, 0.0, 0.0, 0.5, 0.5);
        assert!((a.iou(&a) - 1.0).abs() < 1e-6);
        assert_eq!(a.iou(&det(0, 1.0, 0.6, 0.6, 0.2, 0.2)), 0.0);
        let half = det(0, 1.0, 0.25, 0.0, 0.5, 0.5);
        assert!((a.iou(&half) - 1.0 / 3.0).abs() < 1e-6);
    }

    #[test]
    fn nms_keeps_the_best_of_overlapping_boxes_of_the_same_class_only() {
        let kept = nms(
            vec![
                det(0, 0.6, 0.10, 0.10, 0.4, 0.4),
                det(0, 0.9, 0.12, 0.10, 0.4, 0.4), // sobrepõe e vence
                det(2, 0.7, 0.10, 0.10, 0.4, 0.4), // outra classe: fica
                det(0, 0.5, 0.70, 0.70, 0.2, 0.2), // longe: fica
            ],
            0.45,
        );
        assert_eq!(kept.len(), 3);
        assert_eq!(kept[0].score, 0.9);
        assert!(kept.iter().all(|d| d.score != 0.6));
    }

    #[test]
    fn postprocess_is_decode_then_nms() {
        let lb = Letterbox::new(640, 640, 640).unwrap();
        let o = output(
            80,
            4,
            &[
                (0, [200.0, 200.0, 100.0, 100.0], 0, 0.9),
                (1, [204.0, 200.0, 100.0, 100.0], 0, 0.8),
                (2, [500.0, 500.0, 60.0, 60.0], 2, 0.7),
            ],
        );
        let d = postprocess(&o, 80, &lb, 0.25, 0.45);
        assert_eq!(d.len(), 2);
        assert_eq!((d[0].label(), d[1].label()), ("person", "car"));
    }

    #[test]
    fn the_region_of_zones_is_their_bounding_box_and_only_active_ones_count() {
        use crate::domain::zones::{MotionZone, Point, ZoneConfig};
        let tri = |x: f64, y: f64| {
            MotionZone::new(
                "z",
                vec![
                    Point { x, y },
                    Point { x: x + 0.2, y },
                    Point { x, y: y + 0.3 },
                ],
            )
        };
        let mut cfg = ZoneConfig {
            zones: vec![tri(0.1, 0.2), tri(0.5, 0.4)],
        };
        let r = Region::of_zones(&cfg).unwrap();
        assert!((r.x - 0.1).abs() < 1e-6 && (r.y - 0.2).abs() < 1e-6);
        assert!(
            (r.w - 0.6).abs() < 1e-6 && (r.h - 0.5).abs() < 1e-6,
            "{r:?}"
        );
        cfg.zones[1].enabled = false;
        let r = Region::of_zones(&cfg).unwrap();
        assert!((r.w - 0.2).abs() < 1e-6, "zona desligada não conta: {r:?}");
        cfg.zones[0].enabled = false;
        assert!(
            Region::of_zones(&cfg).is_none(),
            "sem zona ativa: quadro todo"
        );
        assert!(Region::of_zones(&ZoneConfig::default()).is_none());
    }

    #[test]
    fn cropping_takes_the_right_pixels() {
        // 4×2: pixel (x, y) tem R = 10*y + x
        let mut rgba = Vec::new();
        for y in 0..2u8 {
            for x in 0..4u8 {
                rgba.extend_from_slice(&[10 * y + x, 0, 0, 255]);
            }
        }
        let r = Region {
            x: 0.5,
            y: 0.0,
            w: 0.5,
            h: 1.0,
        };
        let (out, w, h) = r.crop_rgba(&rgba, 4, 2).unwrap();
        assert_eq!((w, h), (2, 2));
        let reds: Vec<u8> = out.chunks(4).map(|p| p[0]).collect();
        assert_eq!(reds, vec![2, 3, 12, 13]);
        assert!(r.crop_rgba(&rgba, 4, 3).is_none(), "bytes não batem");
        assert_eq!(Region::FULL.crop_rgba(&rgba, 4, 2).unwrap().0, rgba);
    }

    #[test]
    fn a_box_in_a_crop_maps_back_to_the_whole_picture() {
        let region = Region {
            x: 0.5,
            y: 0.25,
            w: 0.4,
            h: 0.5,
        };
        let inner = det(0, 0.9, 0.5, 0.5, 0.5, 0.25);
        let d = inner.from_crop(&region);
        assert!((d.x - 0.7).abs() < 1e-6 && (d.y - 0.5).abs() < 1e-6);
        assert!((d.w - 0.2).abs() < 1e-6 && (d.h - 0.125).abs() < 1e-6);
        assert_eq!((d.class, d.score), (0, 0.9));
        let (cx, cy) = d.centre();
        assert!((cx - 0.8).abs() < 1e-6 && (cy - 0.5625).abs() < 1e-6);
    }

    #[test]
    fn snapping_follows_the_whole_pixels_the_crop_uses() {
        let r = Region {
            x: 0.30,
            y: 0.0,
            w: 0.30,
            h: 1.0,
        };
        // 10 px: de 3,0 a 6,0 -> 3..6
        let s = r.snapped(10, 10);
        assert!((s.x - 0.3).abs() < 1e-6 && (s.w - 0.3).abs() < 1e-6);
        // 7 px: de 2,1 a 4,2 -> pixels 2..5
        let s = r.snapped(7, 7);
        assert!(
            (s.x - 2.0 / 7.0).abs() < 1e-6 && (s.w - 3.0 / 7.0).abs() < 1e-6,
            "{s:?}"
        );
    }

    #[test]
    fn detect_config_defaults_and_validation() {
        let c = DetectFile::default().into_config().unwrap();
        assert_eq!(c, DetectConfig::default());
        assert!(!c.enabled);
        let f = |toml: &str| toml::from_str::<DetectFile>(toml).unwrap().into_config();
        assert_eq!(
            f("enabled = true\nbackend = \"CUDA\"\nmin_score = 0.4")
                .unwrap()
                .backend,
            BackendChoice::Cuda
        );
        assert!(f("backend = \"tpu\"").unwrap_err().contains("tpu"));
        assert!(f("min_score = 0.0").is_err());
        assert!(f("min_score = 1.5").is_err());
        assert!(f("iou = 0.95").is_err());
        assert!(!c.record && !c.snapshot);
        let c = f("record = true\nsnapshot = true\ncameras = [\"Garagem\"]").unwrap();
        assert!(c.record && c.snapshot);
        assert_eq!(c.cameras, ["Garagem"]);
    }

    #[test]
    fn labels_resolve_to_class_ids_and_unknown_ones_are_rejected() {
        assert_eq!(class_id("person"), Some(0));
        assert_eq!(class_id(" Car "), Some(2));
        assert_eq!(class_id("unicorn"), None);
        let f = |t: &str| toml::from_str::<DetectFile>(t).unwrap().into_config();
        let c = f("labels = [\"person\", \"car\", \"PERSON\"]\ncooldown_secs = 10").unwrap();
        assert_eq!((c.labels, c.cooldown_secs), (vec![0, 2], 10));
        assert!(f("labels = [\"unicorn\"]").unwrap_err().contains("unicorn"));
        assert!(f("cooldown_secs = 1").is_err());
        assert_eq!(label_pt("person"), "pessoa");
        assert_eq!(label_pt("kite"), "kite");
    }
}
