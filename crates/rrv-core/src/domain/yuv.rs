//! NV12 → RGBA, the CPU reference for what the window's shader does on the GPU.
//!
//! The window shows NV12 frames straight from the decoder (1.5 bytes per pixel
//! instead of 4) and converts them in the fragment shader. Snapshots and the
//! tests need the same colours on the CPU, so the maths lives here once.
//!
//! The frame is always *tightly packed*: the Y plane (`width × height`) and then
//! the interleaved UV plane (`ceil(width/2) × 2` bytes per row, `ceil(height/2)` rows).

/// Which YCbCr → RGB matrix the stream was encoded with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum YuvMatrix {
    Bt601,
    Bt709,
}

impl YuvMatrix {
    /// The luma weights `(kr, kb)` of the standard.
    pub fn weights(self) -> (f32, f32) {
        match self {
            Self::Bt601 => (0.299, 0.114),
            Self::Bt709 => (0.2126, 0.0722),
        }
    }

    /// What players assume when the stream does not say: HD is 709, the rest 601.
    pub fn guess(height: u32) -> Self {
        if height >= 720 {
            Self::Bt709
        } else {
            Self::Bt601
        }
    }
}

/// How an NV12 frame must be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct YuvFormat {
    pub matrix: YuvMatrix,
    /// `true` for 0–255 (JPEG-style) samples, `false` for the usual 16–235 / 16–240.
    pub full_range: bool,
}

/// Bytes of a tightly packed NV12 frame, `None` for a zero or overflowing size.
pub fn nv12_len(width: u32, height: u32) -> Option<usize> {
    let (w, h) = (width as usize, height as usize);
    if w == 0 || h == 0 {
        return None;
    }
    let chroma = w.div_ceil(2).checked_mul(2)?.checked_mul(h.div_ceil(2))?;
    w.checked_mul(h)?.checked_add(chroma)
}

/// One pixel: `(y, u, v)` samples to `[r, g, b]`.
pub fn pixel_to_rgb(y: u8, u: u8, v: u8, fmt: YuvFormat) -> [u8; 3] {
    let (kr, kb) = fmt.matrix.weights();
    let kg = 1.0 - kr - kb;
    let (yy, cb, cr) = if fmt.full_range {
        (y as f32, u as f32 - 128.0, v as f32 - 128.0)
    } else {
        (
            (y as f32 - 16.0) * (255.0 / 219.0),
            (u as f32 - 128.0) * (255.0 / 224.0),
            (v as f32 - 128.0) * (255.0 / 224.0),
        )
    };
    let r = yy + 2.0 * (1.0 - kr) * cr;
    let b = yy + 2.0 * (1.0 - kb) * cb;
    let g = (yy - kr * r - kb * b) / kg;
    let q = |c: f32| c.round().clamp(0.0, 255.0) as u8;
    [q(r), q(g), q(b)]
}

/// Converts a tightly packed NV12 frame to RGBA (alpha 255). `None` when `data`
/// does not have exactly the bytes the size calls for.
pub fn nv12_to_rgba(data: &[u8], width: u32, height: u32, fmt: YuvFormat) -> Option<Vec<u8>> {
    if nv12_len(width, height)? != data.len() {
        return None;
    }
    let (w, h) = (width as usize, height as usize);
    let (y_plane, uv_plane) = data.split_at(w * h);
    let uv_stride = w.div_ceil(2) * 2;
    let mut out = vec![255u8; w * h * 4];
    for row in 0..h {
        let uv_row = &uv_plane[(row / 2) * uv_stride..];
        for col in 0..w {
            let y = y_plane[row * w + col];
            let u = uv_row[(col / 2) * 2];
            let v = uv_row[(col / 2) * 2 + 1];
            let [r, g, b] = pixel_to_rgb(y, u, v, fmt);
            let o = (row * w + col) * 4;
            out[o] = r;
            out[o + 1] = g;
            out[o + 2] = b;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMITED_601: YuvFormat = YuvFormat {
        matrix: YuvMatrix::Bt601,
        full_range: false,
    };

    #[test]
    fn length_counts_odd_sizes_up() {
        assert_eq!(nv12_len(4, 4), Some(16 + 8));
        assert_eq!(nv12_len(3, 3), Some(9 + 4 * 2));
        assert_eq!(nv12_len(0, 4), None);
    }

    #[test]
    fn limited_range_black_white_and_grey() {
        assert_eq!(pixel_to_rgb(16, 128, 128, LIMITED_601), [0, 0, 0]);
        assert_eq!(pixel_to_rgb(235, 128, 128, LIMITED_601), [255, 255, 255]);
        let g = pixel_to_rgb(126, 128, 128, LIMITED_601);
        assert!(g[0].abs_diff(128) <= 1 && g[0] == g[1] && g[1] == g[2]);
    }

    #[test]
    fn full_range_uses_the_whole_scale() {
        let f = YuvFormat {
            full_range: true,
            ..LIMITED_601
        };
        assert_eq!(pixel_to_rgb(0, 128, 128, f), [0, 0, 0]);
        assert_eq!(pixel_to_rgb(255, 128, 128, f), [255, 255, 255]);
    }

    #[test]
    fn primaries_land_where_the_standard_puts_them() {
        // BT.601 limited: red is (81, 90, 240), blue is (41, 240, 110).
        let red = pixel_to_rgb(81, 90, 240, LIMITED_601);
        assert!(red[0] >= 253 && red[1] <= 3 && red[2] <= 3, "{red:?}");
        let blue = pixel_to_rgb(41, 240, 110, LIMITED_601);
        assert!(blue[2] >= 253 && blue[0] <= 3 && blue[1] <= 3, "{blue:?}");
    }

    #[test]
    fn the_two_matrices_disagree_on_saturated_colours() {
        let m709 = YuvFormat {
            matrix: YuvMatrix::Bt709,
            ..LIMITED_601
        };
        assert_ne!(
            pixel_to_rgb(81, 90, 240, LIMITED_601),
            pixel_to_rgb(81, 90, 240, m709)
        );
    }

    #[test]
    fn frame_conversion_shares_chroma_per_2x2_block() {
        // 2x2: four different luma values, one UV pair (grey).
        let data = [16, 235, 16, 235, 128, 128];
        let rgba = nv12_to_rgba(&data, 2, 2, LIMITED_601).unwrap();
        assert_eq!(&rgba[0..4], &[0, 0, 0, 255]);
        assert_eq!(&rgba[4..8], &[255, 255, 255, 255]);
        assert_eq!(&rgba[8..12], &[0, 0, 0, 255]);
        assert_eq!(&rgba[12..16], &[255, 255, 255, 255]);
    }

    #[test]
    fn wrong_length_is_refused() {
        assert!(nv12_to_rgba(&[0; 5], 2, 2, LIMITED_601).is_none());
    }

    #[test]
    fn hd_guesses_709() {
        assert_eq!(YuvMatrix::guess(1080), YuvMatrix::Bt709);
        assert_eq!(YuvMatrix::guess(480), YuvMatrix::Bt601);
    }
}
