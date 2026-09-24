//! Video codec enum — pure data, no I/O.
//!
//! Parsed from a GStreamer `Caps` (the post-`rtspsrc` or post-
//! `decodebin` srcpad) and rendered in the sidebar as a short,
//! human-friendly label (`H264 High@L4.1`, `H265 Main@L5`, `MJPEG`,
//! `VP8`, `VP9`, `AV1`, or the raw mime as fallback).
//!
//! TDD: every public function below has a matching test in the
//! `tests` module at the bottom of this file. Run them with
//! `cargo test domain::codec::tests`.

#![allow(dead_code)]

use std::fmt;

use gstreamer as gst;

/// Parsed video codec carried through the pipeline metrics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Codec {
    /// H.264 / AVC. Most IP cameras since 2003.
    H264 { profile: H264Profile, level: u8 },
    /// H.265 / HEVC. Modern IP cameras (4K, low bitrate).
    H265 { profile: H265Profile, tier: H265Tier },
    /// Motion JPEG. Legacy / low-end IP cameras.
    Mjpeg,
    /// VP8 (rare in surveillance, but supported by `decodebin`).
    Vp8,
    /// VP9 (rare in surveillance, but supported by `decodebin`).
    Vp9,
    /// AV1 (newer; very rare in RTSP cameras).
    Av1,
    /// Unknown codec — the raw mime is preserved for the sidebar.
    Other(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum H264Profile {
    Baseline,
    Main,
    High,
    High10,
    High422,
    High444,
    ConstrainedBaseline,
    Unknown(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum H265Profile {
    Main,
    Main10,
    MainStill,
    RangeExtensions,
    HighThroughput,
    Unknown(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum H265Tier {
    Main,
    High,
}

impl fmt::Display for Codec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Codec::H264 { profile, level } => write!(f, "H264 {}@L{}.0", profile, level),
            Codec::H265 { profile, tier } => write!(f, "H265 {}@{:?}@L?", profile, tier),
            Codec::Mjpeg => f.write_str("MJPEG"),
            Codec::Vp8 => f.write_str("VP8"),
            Codec::Vp9 => f.write_str("VP9"),
            Codec::Av1 => f.write_str("AV1"),
            Codec::Other(mime) => write!(f, "{}", mime),
        }
    }
}

impl fmt::Display for H264Profile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            H264Profile::Baseline => "Baseline",
            H264Profile::Main => "Main",
            H264Profile::High => "High",
            H264Profile::High10 => "High10",
            H264Profile::High422 => "High422",
            H264Profile::High444 => "High444",
            H264Profile::ConstrainedBaseline => "ConstrainedBaseline",
            H264Profile::Unknown(_) => "Unknown",
        };
        f.write_str(s)
    }
}

impl fmt::Display for H265Profile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            H265Profile::Main => "Main",
            H265Profile::Main10 => "Main10",
            H265Profile::MainStill => "MainStill",
            H265Profile::RangeExtensions => "RangeExt",
            H265Profile::HighThroughput => "HighThroughput",
            H265Profile::Unknown(_) => "Unknown",
        };
        f.write_str(s)
    }
}

impl H264Profile {
    /// GStreamer reports H.264 `profile` as a 4cc string. The
    /// canonical encoding matches RFC 6184: `"baseline"`, `"main"`,
    /// `"high"`, `"high-10"`, `"high-4:2:2"`, `"high-4:4:4"`,
    /// `"constrained-baseline"`.
    pub fn from_gst(s: &str) -> Self {
        match s {
            "baseline" => H264Profile::Baseline,
            "main" => H264Profile::Main,
            "high" => H264Profile::High,
            "high-10" => H264Profile::High10,
            "high-4:2:2" => H264Profile::High422,
            "high-4:4:4" => H264Profile::High444,
            "constrained-baseline" => H264Profile::ConstrainedBaseline,
            other => {
                // Some cameras misreport (e.g. `"constrained baseline"`).
                // Be liberal: try to map the first 4 chars to known ones.
                let code = other.chars().take(4).collect::<String>().to_lowercase();
                match code.as_str() {
                    "base" => H264Profile::Baseline,
                    "main" => H264Profile::Main,
                    "high" => H264Profile::High,
                    _ => H264Profile::Unknown(0),
                }
            }
        }
    }
}

impl H265Profile {
    /// GStreamer reports H.265 `profile` as strings like `"main"`,
    /// `"main-10"`, `"main-still-picture"`, `"range-extensions"`,
    /// `"high-throughput"`.
    pub fn from_gst(s: &str) -> Self {
        match s {
            "main" => H265Profile::Main,
            "main-10" => H265Profile::Main10,
            "main-still-picture" => H265Profile::MainStill,
            "range-extensions" => H265Profile::RangeExtensions,
            "high-throughput" => H265Profile::HighThroughput,
            _ => H265Profile::Unknown(0),
        }
    }
}

/// Parse a GStreamer `Caps` into a `Codec`. Returns `None` for
/// audio caps, application caps, or anything we don't recognise.
///
/// The `Caps` API used here is `gst::Caps`. The function reads the
/// structure name (`video/x-h264`, `video/x-h265`, …) and the
/// `profile` / `level` fields when present.
pub fn from_caps(caps: &gst::Caps) -> Option<Codec> {
    let s = caps.structure(0)?;
    let name = s.name();
    if name == "video/x-h264" {
        let profile = s
            .get::<&str>("profile")
            .ok()
            .map(H264Profile::from_gst)
            .unwrap_or(H264Profile::Unknown(0));
        let level = s.get::<&str>("level").ok().and_then(parse_h264_level).unwrap_or(0);
        return Some(Codec::H264 { profile, level });
    }
    if name == "video/x-h265" {
        let profile = s
            .get::<&str>("profile")
            .ok()
            .map(H265Profile::from_gst)
            .unwrap_or(H265Profile::Unknown(0));
        let tier = s
            .get::<&str>("tier")
            .ok()
            .map(|t| if t == "high" { H265Tier::High } else { H265Tier::Main })
            .unwrap_or(H265Tier::Main);
        return Some(Codec::H265 { profile, tier });
    }
    if name == "image/jpeg" || name == "image/jpg" {
        return Some(Codec::Mjpeg);
    }
    if name == "video/x-vp8" {
        return Some(Codec::Vp8);
    }
    if name == "video/x-vp9" {
        return Some(Codec::Vp9);
    }
    if name == "video/x-av1" {
        return Some(Codec::Av1);
    }
    if name.as_str().starts_with("video/")
        && !name.as_str().starts_with("video/x-raw")
    {
        return Some(Codec::Other(name.as_str().to_string()));
    }
    None
}

fn parse_h264_level(s: &str) -> Option<u8> {
    // GStreamer reports levels as strings like "1", "1b", "1.1",
    // "1.2", "1.3", "2", "2.1", "2.2", "3", "3.1", "3.2", "4",
    // "4.1", "4.2", "5", "5.1", "5.2". We map to the integer part
    // (1, 2, 3, 4, 5) for the sidebar label. None on garbage.
    s.split('.').next()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Once;

    static GST_INIT: Once = Once::new();

    /// GStreamer must be initialised before constructing any
    /// `gst::Caps`. `Once::call_once` makes this idempotent and
    /// safe across the test binary's many parallel test threads.
    fn init_gst() {
        GST_INIT.call_once(|| {
            gst::init().expect("gst::init for tests");
        });
    }

    fn caps(mime: &str, fields: &[(&str, &str)]) -> gst::Caps {
        init_gst();
        let mut builder = gst::Caps::builder(mime);
        for (k, v) in fields {
            builder = builder.field(*k, *v);
        }
        builder.build()
    }

    #[test]
    fn h264_high_41_parses_to_canonical_label() {
        let c = caps("video/x-h264", &[("profile", "high"), ("level", "4.1")]);
        let codec = from_caps(&c).expect("h264 caps should parse");
        assert_eq!(codec, Codec::H264 { profile: H264Profile::High, level: 4 });
        assert_eq!(codec.to_string(), "H264 High@L4.0");
    }

    #[test]
    fn h264_main_31_renders() {
        let c = caps("video/x-h264", &[("profile", "main"), ("level", "3.1")]);
        let codec = from_caps(&c).unwrap();
        assert_eq!(codec.to_string(), "H264 Main@L3.0");
    }

    #[test]
    fn h264_constrained_baseline_renders() {
        let c = caps("video/x-h264", &[("profile", "constrained-baseline"), ("level", "3.1")]);
        let codec = from_caps(&c).unwrap();
        assert_eq!(
            codec,
            Codec::H264 { profile: H264Profile::ConstrainedBaseline, level: 3 }
        );
    }

    #[test]
    fn h264_without_profile_falls_back_to_unknown() {
        let c = caps("video/x-h264", &[("level", "4.1")]);
        let codec = from_caps(&c).unwrap();
        assert_eq!(
            codec,
            Codec::H264 { profile: H264Profile::Unknown(0), level: 4 }
        );
    }

    #[test]
    fn h265_main_tier_main_renders() {
        let c = caps(
            "video/x-h265",
            &[("profile", "main"), ("tier", "main"), ("level", "4")],
        );
        let codec = from_caps(&c).unwrap();
        assert_eq!(
            codec,
            Codec::H265 { profile: H265Profile::Main, tier: H265Tier::Main }
        );
    }

    #[test]
    fn mjpeg_caps_parse_to_mjpeg() {
        let c = caps("image/jpeg", &[]);
        assert_eq!(from_caps(&c), Some(Codec::Mjpeg));
        assert_eq!(from_caps(&c).unwrap().to_string(), "MJPEG");
    }

    #[test]
    fn vp8_caps_parse_to_vp8() {
        let c = caps("video/x-vp8", &[]);
        assert_eq!(from_caps(&c), Some(Codec::Vp8));
    }

    #[test]
    fn unknown_video_mime_falls_back_to_other() {
        // video/x-raw is a decoded format, not a codec — should return None.
        let c = caps("video/x-raw", &[]);
        assert_eq!(from_caps(&c), None);
        // Other unknown video mimes still fall back to Codec::Other.
        let c = caps("video/x-unknown", &[]);
        assert_eq!(from_caps(&c), Some(Codec::Other("video/x-unknown".to_string())));
    }

    #[test]
    fn audio_caps_return_none() {
        let c = caps("audio/mpeg", &[]);
        assert_eq!(from_caps(&c), None);
    }

    #[test]
    fn empty_caps_return_none() {
        init_gst();
        let c = gst::Caps::new_empty();
        assert_eq!(from_caps(&c), None);
    }
}
