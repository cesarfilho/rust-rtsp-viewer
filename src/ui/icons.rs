//! Icon glyphs need a font that actually has them. The system fallback list
//! used by iced/cosmic-text misses many Geometric Shapes (e.g. `◱ ◲`), which
//! rendered as empty boxes. DejaVu Sans covers every glyph the UI uses, so it
//! is embedded and applied explicitly to icon text — never to prose.

use iced::Font;

/// Registered once on the application (`.font(FONT_BYTES)`).
pub const FONT_BYTES: &[u8] = include_bytes!("../../assets/fonts/DejaVuSans.ttf");

/// Family name inside `FONT_BYTES`.
pub const FONT: Font = Font::with_name("DejaVu Sans");
