//! Helpers for building `gst_parse_launch` descriptions.

/// Quote a value for embedding in a `gst_parse_launch` description.
///
/// Unquoted URLs and paths break the parser as soon as they contain a space,
/// `!`, or `&` — all of which turn up in real camera URLs and file paths.
pub fn quote_launch_value(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_plain_url() {
        assert_eq!(quote_launch_value("rtsp://cam/live"), "\"rtsp://cam/live\"");
    }

    #[test]
    fn escapes_embedded_quotes_and_backslashes() {
        assert_eq!(quote_launch_value(r#"a"b\c"#), r#""a\"b\\c""#);
    }

    #[test]
    fn keeps_ampersand_and_bang_inside_quotes() {
        assert_eq!(quote_launch_value("http://h/p?a=1&b=2!x"), "\"http://h/p?a=1&b=2!x\"");
    }
}
