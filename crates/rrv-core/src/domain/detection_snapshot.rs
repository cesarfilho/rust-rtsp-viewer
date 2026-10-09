//! The picture saved when an object is recognised (`[detect] snapshot`): where it goes and the
//! boxes drawn on it. Pure: the frame comes in as RGBA bytes, nothing is read or written here.

use std::path::{Path, PathBuf};

use super::detect::Detection;

/// `<dir>/<camera>/<YYYY-MM-DD>/<HH-MM-SS>_<label>.jpg`: one folder per camera and day, so a
/// network share stays browsable after months of events. `camera` must already be a safe file
/// name (`recording_paths::safe_filename`); the label is a COCO name (`person`, `car`...).
pub fn path_for(dir: &Path, camera: &str, at: chrono::NaiveDateTime, label: &str) -> PathBuf {
    let label: String = label
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    dir.join(camera)
        .join(at.format("%Y-%m-%d").to_string())
        .join(format!("{}_{label}.jpg", at.format("%H-%M-%S")))
}

/// Box outline thickness relative to the picture's shorter side (3 px at 1080p).
const LINE_FRACTION: f32 = 1.0 / 360.0;

/// High-contrast colours for boxes drawn over video (black label text stays readable on each).
pub const BOX_PALETTE: [[u8; 3]; 6] = [
    [51, 217, 102],  // verde
    [255, 179, 26],  // âmbar
    [77, 166, 255],  // azul
    [255, 89, 102],  // vermelho
    [204, 128, 255], // violeta
    [51, 230, 230],  // ciano
];

/// The colour of a class's boxes, in the window and in the snapshots alike: picked from the label
/// (not from the order objects appear in), so a person is always the same colour everywhere.
pub fn box_colour(label: &str) -> [u8; 3] {
    let h = label
        .bytes()
        .fold(7u32, |a, b| a.wrapping_mul(31).wrapping_add(u32::from(b)));
    BOX_PALETTE[(h as usize) % BOX_PALETTE.len()]
}

/// Draws each detection's rectangle (normalised coordinates) onto an RGBA frame in place.
/// Boxes partly outside the picture are clipped; a frame whose size does not match is left alone.
pub fn draw_boxes(rgba: &mut [u8], width: u32, height: u32, found: &[Detection]) {
    let (w, h) = (width as usize, height as usize);
    if w == 0 || h == 0 || rgba.len() < w * h * 4 {
        return;
    }
    let line = ((w.min(h) as f32 * LINE_FRACTION).round() as usize).max(1);
    let to_px = |v: f32, size: usize| ((v.clamp(0.0, 1.0) * size as f32) as usize).min(size - 1);
    for d in found {
        let (x0, y0) = (to_px(d.x, w), to_px(d.y, h));
        let (x1, y1) = (to_px(d.x + d.w, w), to_px(d.y + d.h, h));
        if x1 <= x0 || y1 <= y0 {
            continue;
        }
        let [r, g, b] = box_colour(d.label());
        let mut paint = |x: usize, y: usize| {
            let i = (y * w + x) * 4;
            rgba[i..i + 4].copy_from_slice(&[r, g, b, 255]);
        };
        for t in 0..line {
            for x in x0..=x1 {
                paint(x, (y0 + t).min(y1));
                paint(x, y1.saturating_sub(t).max(y0));
            }
            for y in y0..=y1 {
                paint((x0 + t).min(x1), y);
                paint(x1.saturating_sub(t).max(x0), y);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn det(class: usize, x: f32, y: f32, w: f32, h: f32) -> Detection {
        Detection {
            class,
            score: 0.9,
            x,
            y,
            w,
            h,
        }
    }

    #[test]
    fn the_path_has_camera_day_time_and_label() {
        let at = chrono::NaiveDate::from_ymd_opt(2026, 10, 9)
            .unwrap()
            .and_hms_opt(15, 4, 5)
            .unwrap();
        assert_eq!(
            path_for(Path::new("/data/snapshots"), "garagem", at, "traffic light"),
            Path::new("/data/snapshots/garagem/2026-10-09/15-04-05_traffic_light.jpg")
        );
    }

    #[test]
    fn a_box_outline_is_painted_and_its_inside_is_not() {
        let (w, h) = (20u32, 10u32);
        let mut px = vec![0u8; (w * h * 4) as usize];
        draw_boxes(&mut px, w, h, &[det(0, 0.25, 0.2, 0.5, 0.6)]);
        let at = |x: usize, y: usize| &px[(y * w as usize + x) * 4..][..4];
        let [r, g, b] = box_colour("person");
        assert_eq!(at(5, 2), [r, g, b, 255], "canto de cima");
        assert_eq!(at(15, 8), [r, g, b, 255], "canto de baixo");
        assert_eq!(at(10, 5), [0, 0, 0, 0], "o meio fica intacto");
        assert_eq!(at(0, 0), [0, 0, 0, 0], "fora da caixa também");
    }

    #[test]
    fn boxes_outside_or_a_wrong_sized_frame_never_panic() {
        let mut px = vec![0u8; 4 * 4 * 4];
        draw_boxes(
            &mut px,
            4,
            4,
            &[det(2, 0.9, 0.9, 0.5, 0.5), det(0, -1.0, -1.0, 0.1, 0.1)],
        );
        draw_boxes(&mut px, 8, 8, &[det(0, 0.0, 0.0, 1.0, 1.0)]);
        draw_boxes(&mut px, 0, 0, &[det(0, 0.0, 0.0, 1.0, 1.0)]);
    }
}
