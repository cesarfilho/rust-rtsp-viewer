//! Credential redaction for log output.
//!
//! RTSP camera URLs almost always carry inline credentials
//! (`rtsp://admin:senha@192.168.1.10/stream`). Those URLs end up in
//! GStreamer pipeline strings, which we log at startup and on every
//! reconnect — so without masking, the password lands in plain text in
//! journald, log files, and any bug report the user pastes.
//!
//! [`mask_credentials`] rewrites the userinfo component of every URL it
//! finds in a string, keeping the username visible (useful for debugging
//! auth failures) and replacing the password with `***`.

/// Replace the password in every `scheme://user:pass@host` occurrence
/// with `***`, leaving the rest of the string untouched.
///
/// Handles arbitrary text containing zero or more URLs, so it can be
/// applied to a whole GStreamer pipeline description rather than just a
/// bare URL.
pub fn mask_credentials(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;

    while let Some(scheme_end) = rest.find("://") {
        let after_scheme = scheme_end + 3;
        // The authority runs until the first `/`, `?`, `#`, whitespace or
        // quote — whichever terminates the host portion first.
        let authority_len = rest[after_scheme..]
            .find(|c: char| c == '/' || c == '?' || c == '#' || c == '"' || c.is_whitespace())
            .unwrap_or(rest.len() - after_scheme);
        let authority = &rest[after_scheme..after_scheme + authority_len];

        out.push_str(&rest[..after_scheme]);

        // `userinfo@host` — only the last `@` separates them, since a
        // password may itself contain `@`.
        match authority.rfind('@') {
            Some(at) => {
                let userinfo = &authority[..at];
                let host = &authority[at..];
                match userinfo.find(':') {
                    Some(colon) => {
                        out.push_str(&userinfo[..colon]);
                        out.push_str(":***");
                    }
                    // `user@host` with no password — nothing to hide.
                    None => out.push_str(userinfo),
                }
                out.push_str(host);
            }
            None => out.push_str(authority),
        }

        rest = &rest[after_scheme + authority_len..];
    }

    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_password_keeps_user_and_host() {
        assert_eq!(
            mask_credentials("rtsp://admin:s3cr3t@192.168.1.10:554/stream1"),
            "rtsp://admin:***@192.168.1.10:554/stream1"
        );
    }

    #[test]
    fn leaves_url_without_credentials_untouched() {
        let url = "rtsp://192.168.1.10:554/stream1";
        assert_eq!(mask_credentials(url), url);
    }

    #[test]
    fn leaves_user_only_url_untouched() {
        let url = "rtsp://admin@192.168.1.10/stream";
        assert_eq!(mask_credentials(url), url);
    }

    #[test]
    fn masks_inside_a_full_pipeline_string() {
        let pipeline = "rtspsrc name=source location=\"rtsp://admin:pw@cam/live\" latency=100 ! decodebin";
        assert_eq!(
            mask_credentials(pipeline),
            "rtspsrc name=source location=\"rtsp://admin:***@cam/live\" latency=100 ! decodebin"
        );
    }

    #[test]
    fn masks_every_url_in_the_string() {
        assert_eq!(
            mask_credentials("a=http://u1:p1@h1/x b=rtsp://u2:p2@h2/y"),
            "a=http://u1:***@h1/x b=rtsp://u2:***@h2/y"
        );
    }

    #[test]
    fn password_containing_at_sign_is_fully_masked() {
        assert_eq!(
            mask_credentials("rtsp://admin:p@ss@10.0.0.1/s"),
            "rtsp://admin:***@10.0.0.1/s"
        );
    }

    #[test]
    fn handles_authority_at_end_of_string() {
        assert_eq!(
            mask_credentials("rtsp://admin:pw@host"),
            "rtsp://admin:***@host"
        );
    }

    #[test]
    fn text_without_any_url_is_unchanged() {
        assert_eq!(mask_credentials("no urls here"), "no urls here");
    }
}
