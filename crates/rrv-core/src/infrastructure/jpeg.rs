//! Writes an RGBA picture as a JPEG (detection snapshots).

use std::path::Path;

/// JPEG quality: 85 keeps a face readable at a fraction of the PNG size.
pub const QUALITY: u8 = 85;

/// Encodes `rgba` and writes it to `path`, creating the folders. Written to a `.tmp` first and
/// renamed, so whoever browses the share never opens half a file.
pub fn write(path: &Path, rgba: &[u8], width: u32, height: u32) -> Result<(), String> {
    let (pixels, _) = rgba.as_chunks::<4>();
    let rgb: Vec<u8> = pixels.iter().flat_map(|p| [p[0], p[1], p[2]]).collect();
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, QUALITY)
        .encode(&rgb, width, height, image::ExtendedColorType::Rgb8)
        .map_err(|e| format!("JPEG: {e}"))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let tmp = path.with_extension("jpg.tmp");
    std::fs::write(&tmp, &bytes).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_picture_round_trips_through_the_file() {
        let dir = std::env::temp_dir().join(format!("rrv-jpeg-{}", std::process::id()));
        let path = dir.join("a/b/x.jpg");
        let rgba: Vec<u8> = (0..16 * 8).flat_map(|_| [200, 30, 30, 255]).collect();
        super::write(&path, &rgba, 16, 8).unwrap();
        let img = image::open(&path).unwrap();
        assert_eq!((img.width(), img.height()), (16, 8));
        assert!(!path.with_extension("jpg.tmp").exists());
        let _ = std::fs::remove_dir_all(dir);
    }
}
