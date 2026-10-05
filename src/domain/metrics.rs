use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use crate::domain::codec::Codec;

/// Stream info extracted from RTP caps (right after rtspsrc, before decode).
#[derive(Default, Debug, Clone)]
pub struct StreamInfo {
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub framerate_num: Option<i32>,
    pub framerate_den: Option<i32>,
    pub codec: Option<String>,
    /// Raw-video pixel format reported by the post-decoder caps (e.g.
    /// `"I420"`, `"NV12"`, `"RGB"`). Used to decide whether the image-quality
    /// sampler can read the Y plane directly.
    pub raw_format: Option<String>,
    /// Parsed codec from the post-decode caps (e.g. `H264 High@L4.0`).
    /// Filled by the post-decode probe via `codec::from_caps`. When
    /// `Some`, the sidebar renders the canonical label; when `None`,
    /// it falls back to the raw `codec` mime (set by the pre-decode
    /// probe from `encoding-name`).
    pub parsed_codec: Option<Codec>,
    /// GStreamer factory name of the video decoder actually chosen (e.g.
    /// `avdec_h264`, `nvh264dec`). `decodebin` picks it at runtime, so it is
    /// found by walking the pipeline once the stream is up.
    pub decoder: Option<String>,
    /// The decoder runs on a GPU / fixed-function block (factory klass has
    /// `Hardware`).
    pub decoder_hw: bool,
}

/// Interpret a factory's `klass` metadata: `Some(is_hardware)` for a video
/// decoder, `None` for anything else.
///
/// `Codec/Decoder/Video` is software (`avdec_*`); hardware decoders add a
/// `/Hardware` segment (`Codec/Decoder/Video/Hardware`: `nvh264dec`,
/// `vah264dec`).
pub fn video_decoder_kind(klass: &str) -> Option<bool> {
    let parts: Vec<&str> = klass.split('/').collect();
    if parts.contains(&"Decoder") && parts.contains(&"Video") {
        Some(parts.contains(&"Hardware"))
    } else {
        None
    }
}

/// Returns true when `format` is a planar (or semi-planar) YUV format whose
/// first plane is the luminance (Y) plane — the layout assumed by the
/// image-quality sampler. Covers the formats GStreamer decoders typically
/// emit (`I420`, `NV12`, `Y444`, …) plus single-plane `GRAY8`.
pub fn is_y_first_format(format: &str) -> bool {
    matches!(
        format,
        "I420"
            | "YV12"
            | "I422"
            | "Y42B"
            | "I444"
            | "Y444"
            | "NV12"
            | "NV21"
            | "GRAY8"
            | "GRAY16_LE"
            | "GRAY16_BE"
    )
}

/// Per-stream RTP-jitterbuffer statistics aggregated across every
/// `rtpjitterbuffer` created by `rtspsrc` (one per RTP stream). All counters
/// are cumulative since the stream started.
#[derive(Default, Debug, Clone, Copy)]
pub struct PacketStats {
    /// True once we've successfully read stats from at least one
    /// `rtpjitterbuffer`. When false the overlay shows `N/A` so the user
    /// knows the camera/transport doesn't expose this data.
    pub available: bool,
    /// Cumulative number of RTP packets received and forwarded.
    pub pushed: u64,
    /// Cumulative number of packets the jitterbuffer detected as missing
    /// (gap in RTP sequence numbers, never recovered).
    pub lost: u64,
    /// Cumulative number of packets that arrived after their deadline
    /// (jitterbuffer ejected them too late to be useful).
    pub late: u64,
    /// Cumulative number of duplicate packets (same RTP seqnum twice).
    pub duplicates: u64,
}

impl PacketStats {
    /// Loss percentage with 2 decimals. Returns `None` when `pushed + lost`
    /// is zero (no traffic yet) so the overlay can render `--` instead of
    /// `0.00%`.
    pub fn loss_pct(&self) -> Option<f64> {
        let total = self.pushed.saturating_add(self.lost);
        if total == 0 {
            None
        } else {
            Some((self.lost as f64 / total as f64) * 100.0)
        }
    }
}

/// Pending pre-decode timestamps keyed by buffer PTS. Used to measure
/// per-frame decode time by matching the same buffer at the decoder's
/// sink pad and the `postdec_queue` sink pad.
///
/// Bounded: when the map grows past `MAX_PENDING_DECODES` we evict
/// arbitrarily — a few stale entries (e.g. dropped buffers that never
/// reach the post-decode probe) is fine, we still get representative
/// samples for the EMA.
pub const MAX_PENDING_DECODES: usize = 256;

/// Centralised, thread-safe state for every metric the overlay cares about.
///
/// Cloned `Arc<Metrics>` is shared between GStreamer probe callbacks (writers)
/// and the main bus loop (readers).
pub struct Metrics {
    // --- Frame & throughput ---
    pub frame_count: AtomicU64,
    /// Most recent rolling FPS reading (frames per second over the last
    /// `update_overlay` interval), stored as `fps * 1000` as an integer so it
    /// fits in an `AtomicU64` (no CAS on f64 needed). Refreshed by the bus
    /// loop. `u64::MAX` is the sentinel for "no sample yet" — `None` from
    /// `snapshot_current_fps()`. 0 means "the bus loop sampled and saw
    /// zero frames in the last interval" — `Some(0.0)`, which the
    /// diagnostics module treats as a stalled decoder.
    pub current_fps_x1000: AtomicU64,
    pub bytes_counter: AtomicU64,
    pub dropped_frames: AtomicU64,

    // --- Network ---
    pub reconnect_count: AtomicU32,
    pub is_live: AtomicBool,
    /// Pipeline end-to-end latency in nanoseconds, refreshed from
    /// `Pipeline::query_latency()`. 0 means "not yet queried".
    pub actual_latency_ns: AtomicU64,
    /// EMA of |inter-frame interval - expected interval| in nanoseconds.
    /// Computed in the post-decode buffer probe. 0 = no samples yet.
    /// The "expected interval" is derived from the negotiated framerate
    /// (`1/fps`); when the framerate is unknown we fall back to the EMA of
    /// the inter-frame interval itself (which captures absolute jitter
    /// magnitude even without a reference).
    pub jitter_ema_ns: AtomicU64,
    /// Monotonic wall-clock nanoseconds of the previous frame seen by the
    /// post-decode probe. 0 = no previous sample.
    pub last_frame_mono_ns: AtomicU64,
    /// Unix epoch seconds when the pipeline last left Playing state.
    /// 0 = no disconnect yet.
    pub last_disconnect_unix_secs: AtomicU64,
    /// Duration of the most recent reconnect in milliseconds (time between
    /// leaving Playing and entering Playing again). 0 = never reconnected.
    pub last_reconnect_duration_ms: AtomicU64,

    // --- Decoder ---
    /// Cumulative count of error/warning bus messages whose source element
    /// looks like a decoder (factory name contains "dec" or matches our
    /// configured `--decoder`). Tracks "the codec is unhappy".
    pub decode_errors: AtomicU64,
    /// EMA of per-frame decode wall-clock time in microseconds (matched by
    /// PTS between predec and postdec probes). 0 = no samples yet.
    pub decode_time_us_ema: AtomicU64,
    /// PTS→enter-time map used to measure decode time. Locked briefly on
    /// each probe call; bounded by `MAX_PENDING_DECODES`.
    pub pending_decode_starts: Mutex<HashMap<u64, u64>>,

    // --- RTP statistics (from rtpjitterbuffer) ---
    pub packet_stats: Mutex<PacketStats>,

    // --- Image quality (computed on a sampled subset of decoded frames) ---
    /// Most recent average luma (Y plane) of a sampled frame, 0-255.
    /// `u64::MAX` is the sentinel for "no sample yet".
    pub last_avg_luma: AtomicU64,
    /// Most recent luma standard deviation (proxy for contrast), 0-255.
    /// `u64::MAX` = "no sample yet".
    pub last_luma_stddev: AtomicU64,
    /// Wall-clock seconds (Unix epoch) when the luma last changed
    /// meaningfully. 0 = never sampled.
    pub last_scene_change_unix_secs: AtomicU64,
    /// Unix epoch seconds of the most recent sample (lets us compute
    /// "seconds since last sample" without storing a separate Instant).
    pub last_sample_unix_secs: AtomicU64,

    // --- Stream identity & error display ---
    pub stream_info: Mutex<StreamInfo>,
    pub last_error: Mutex<Option<String>>,

    // --- Seeking (file playback) ---
    /// Current playback position in nanoseconds, refreshed by tick loop.
    pub position_ns: AtomicU64,
    /// Stream duration in nanoseconds, refreshed by tick loop.
    pub duration_ns: AtomicU64,
    /// True if the source is seekable (file playback).
    pub is_seekable: AtomicBool,

    // --- Monotonic clock origin ---
    /// Process-start `Instant`. Used by `mono_ns()` so probes have a cheap
    /// monotonic clock that doesn't depend on the system wall clock (which
    /// can jump due to NTP).
    start_instant: Instant,
}

impl Metrics {
    pub fn new() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            frame_count: AtomicU64::new(0),
            current_fps_x1000: AtomicU64::new(u64::MAX),
            bytes_counter: AtomicU64::new(0),
            dropped_frames: AtomicU64::new(0),
            reconnect_count: AtomicU32::new(0),
            is_live: AtomicBool::new(false),
            actual_latency_ns: AtomicU64::new(0),
            jitter_ema_ns: AtomicU64::new(0),
            last_frame_mono_ns: AtomicU64::new(0),
            last_disconnect_unix_secs: AtomicU64::new(0),
            last_reconnect_duration_ms: AtomicU64::new(0),
            decode_errors: AtomicU64::new(0),
            decode_time_us_ema: AtomicU64::new(0),
            pending_decode_starts: Mutex::new(HashMap::new()),
            packet_stats: Mutex::new(PacketStats::default()),
            last_avg_luma: AtomicU64::new(u64::MAX),
            last_luma_stddev: AtomicU64::new(u64::MAX),
            last_scene_change_unix_secs: AtomicU64::new(0),
            last_sample_unix_secs: AtomicU64::new(0),
            stream_info: Mutex::new(StreamInfo::default()),
            last_error: Mutex::new(None),
            position_ns: AtomicU64::new(0),
            duration_ns: AtomicU64::new(0),
            is_seekable: AtomicBool::new(false),
            start_instant: Instant::now(),
        })
    }

    /// Monotonic nanoseconds since process start. Cheap (one syscall on
    /// Linux), monotonic, and safe to compare across threads.
    pub fn mono_ns(&self) -> u64 {
        self.start_instant.elapsed().as_nanos() as u64
    }

    /// Returns the measured end-to-end pipeline latency in milliseconds,
    /// or `None` if the latency has never been sampled (0 ns).
    pub fn snapshot_actual_latency_ms(&self) -> Option<u64> {
        let ns = self.actual_latency_ns.load(Ordering::Relaxed);
        if ns == 0 { None } else { Some(ns / 1_000_000) }
    }

    /// Returns the most recent rolling FPS reading.
    /// * `None` — bus loop hasn't published a sample yet (process just
    ///   started, or counter was reset).
    /// * `Some(0.0)` — bus loop sampled and saw zero frames in the last
    ///   interval (decoder is stalled).
    /// * `Some(fps)` — the measured rate.
    pub fn snapshot_current_fps(&self) -> Option<f64> {
        let v = self.current_fps_x1000.load(Ordering::Relaxed);
        if v == u64::MAX {
            None
        } else {
            Some(v as f64 / 1000.0)
        }
    }

    /// EMA jitter in milliseconds, or `None` if no samples yet.
    pub fn snapshot_jitter_ms(&self) -> Option<u64> {
        let ns = self.jitter_ema_ns.load(Ordering::Relaxed);
        if ns == 0 { None } else { Some(ns / 1_000_000) }
    }

    /// EMA per-frame decode time in milliseconds, or `None` if no samples.
    pub fn snapshot_decode_time_ms(&self) -> Option<u64> {
        let us = self.decode_time_us_ema.load(Ordering::Relaxed);
        if us == 0 { None } else { Some(us / 1_000) }
    }

    pub fn snapshot_packet_stats(&self) -> PacketStats {
        self.packet_stats
            .lock()
            .map(|g| *g)
            .unwrap_or_default()
    }

    pub fn snapshot_stream_info(&self) -> StreamInfo {
        self.stream_info
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default()
    }

    pub fn snapshot_last_error(&self) -> Option<String> {
        self.last_error.lock().ok().and_then(|g| g.clone())
    }

    pub fn set_last_error(&self, err: String) {
        if let Ok(mut slot) = self.last_error.lock() {
            *slot = Some(err);
        }
    }

    /// Snapshot the most recent average luma (0-255) and stddev (0-255).
    /// Each returns `None` when no frame has been sampled yet.
    pub fn snapshot_luma(&self) -> (Option<u8>, Option<u8>) {
        let avg = self.last_avg_luma.load(Ordering::Relaxed);
        let dev = self.last_luma_stddev.load(Ordering::Relaxed);
        let avg = if avg == u64::MAX { None } else { Some(avg as u8) };
        let dev = if dev == u64::MAX { None } else { Some(dev as u8) };
        (avg, dev)
    }

    /// Item 6 — average luma if and only if a sample was
    /// written within the last `max_age_secs` wall-clock
    /// seconds. Returns `None` when no sample exists, or when
    /// the last sample is older than the threshold (so a
    /// 5-minute-old luma reading from a stalled pipeline
    /// doesn't keep firing the `Night` hint after the camera
    /// recovers).
    pub fn snapshot_recent_avg_luma(&self, now_unix_secs: u64, max_age_secs: u64) -> Option<u8> {
        let last = self.last_sample_unix_secs.load(Ordering::Relaxed);
        if last == 0 {
            return None;
        }
        if now_unix_secs.saturating_sub(last) > max_age_secs {
            return None;
        }
        let avg = self.last_avg_luma.load(Ordering::Relaxed);
        if avg == u64::MAX {
            None
        } else {
            Some(avg as u8)
        }
    }

    /// Seconds since the image last changed meaningfully (scene-change
    /// detector). Returns `None` if no frame has ever been sampled.
    pub fn snapshot_static_secs(&self, now_unix_secs: u64) -> Option<u64> {
        let last = self.last_scene_change_unix_secs.load(Ordering::Relaxed);
        if last == 0 {
            None
        } else {
            Some(now_unix_secs.saturating_sub(last))
        }
    }
}

#[cfg(test)]
 mod tests {
    #[test]
    fn decoder_klass_distinguishes_software_from_hardware() {
        assert_eq!(video_decoder_kind("Codec/Decoder/Video"), Some(false));
        assert_eq!(video_decoder_kind("Codec/Decoder/Video/Hardware"), Some(true));
        assert_eq!(video_decoder_kind("Codec/Decoder/Audio"), None);
        assert_eq!(video_decoder_kind("Codec/Parser/Converter/Video"), None);
        assert_eq!(video_decoder_kind("Codec/Encoder/Video/Hardware"), None);
        assert_eq!(video_decoder_kind(""), None);
    }

    use super::*;

    // -- Item 6: snapshot_recent_avg_luma ---------------------------

    fn m_with(avg: u64, last_sample: u64) -> std::sync::Arc<Metrics> {
        let m = Metrics::new();
        m.last_avg_luma.store(avg, Ordering::Relaxed);
        m.last_sample_unix_secs.store(last_sample, Ordering::Relaxed);
        m
    }

    #[test]
    fn recent_luma_no_sample_returns_none() {
        let m = Metrics::new();
        assert_eq!(m.snapshot_recent_avg_luma(1_000, 3), None);
    }

    #[test]
    fn recent_luma_fresh_sample_returns_value() {
        let m = m_with(15, 1_000);
        assert_eq!(m.snapshot_recent_avg_luma(1_002, 3), Some(15));
    }

    #[test]
    fn recent_luma_stale_sample_returns_none() {
        let m = m_with(15, 1_000);
        // 5 seconds old, max age 3.
        assert_eq!(m.snapshot_recent_avg_luma(1_005, 3), None);
    }

    #[test]
    fn recent_luma_exactly_max_age_is_inclusive() {
        // 3-second-old sample, max age 3 → still considered
        // recent (the check is `> max_age`, not `>=`).
        let m = m_with(20, 1_000);
        assert_eq!(m.snapshot_recent_avg_luma(1_003, 3), Some(20));
    }

    #[test]
    fn recent_luma_zero_is_a_real_reading_not_sentinel() {
        // A real "completely black" frame is luma=0, not the
        // u64::MAX sentinel. The method must return `Some(0)`
        // for it.
        let m = m_with(0, 1_000);
        assert_eq!(m.snapshot_recent_avg_luma(1_000, 3), Some(0));
    }

    #[test]
    fn packet_stats_loss_none_with_no_traffic() {
        let s = PacketStats::default();
        assert!(s.loss_pct().is_none());
    }

    #[test]
    fn packet_stats_loss_pct_basic() {
        let s = PacketStats {
            available: true,
            pushed: 990,
            lost: 10,
            late: 0,
            duplicates: 0,
        };
        // 10 out of 1000 = 1.0%
        let pct = s.loss_pct().unwrap();
        assert!((pct - 1.0).abs() < 1e-9, "got {pct}");
    }

    #[test]
    fn metrics_snapshot_returns_none_until_set() {
        let m = Metrics::new();
        assert!(m.snapshot_actual_latency_ms().is_none());
        assert!(m.snapshot_jitter_ms().is_none());
        assert!(m.snapshot_decode_time_ms().is_none());
        let (avg, dev) = m.snapshot_luma();
        assert!(avg.is_none() && dev.is_none());
        assert!(m.snapshot_static_secs(123).is_none());
    }

    #[test]
    fn metrics_luma_roundtrips_after_store() {
        let m = Metrics::new();
        m.last_avg_luma.store(128, Ordering::Relaxed);
        m.last_luma_stddev.store(42, Ordering::Relaxed);
        let (avg, dev) = m.snapshot_luma();
        assert_eq!(avg, Some(128));
        assert_eq!(dev, Some(42));
    }

    #[test]
    fn metrics_static_secs_returns_diff_when_scene_change_set() {
        let m = Metrics::new();
        m.last_scene_change_unix_secs.store(100, Ordering::Relaxed);
        assert_eq!(m.snapshot_static_secs(105), Some(5));
        assert_eq!(m.snapshot_static_secs(99), Some(0)); // saturating
    }
}
