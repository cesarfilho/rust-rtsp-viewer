//! Icon glyphs need a font that actually has them. The system fallback list
//! used by iced/cosmic-text misses many Geometric Shapes (e.g. `◱ ◲`), which
//! rendered as empty boxes. DejaVu Sans covers every glyph the UI uses, so it
//! is embedded and applied explicitly to icon text — never to prose.

use iced::Font;

/// Registered once on the application (`.font(FONT_BYTES)`).
pub const FONT_BYTES: &[u8] = include_bytes!("../../assets/fonts/DejaVuSans.ttf");

/// Family name inside `FONT_BYTES`.
pub const FONT: Font = Font::with_name("DejaVu Sans");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_font_is_a_truetype_file() {
        // sfnt version 1.0 — a corrupt or LFS-pointer file would fail here.
        assert_eq!(&FONT_BYTES[..4], &[0x00, 0x01, 0x00, 0x00]);
        assert!(FONT_BYTES.len() > 100_000);
    }
}
