use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use gstreamer as gst;
use gstreamer::prelude::*;

use bytes::Bytes;

use crate::domain::camera_status::{BitrateReading, CameraStatus, StatusReading};
use crate::domain::metrics::{Metrics, PacketStats};
use crate::domain::recording::RecordingConfig;
use crate::domain::yuv::{YuvFormat, nv12_to_rgba};

pub(crate) const EMA_ALPHA_X1000: u64 = 200;
pub(crate) const SAMPLE_EVERY_N: u64 = 30;

/// How long `stop_recording` waits for EOS to drain through the encoder and
/// muxer before tearing the branch down anyway. Normally the drain completes
/// in a few milliseconds; the cap exists so a wedged encoder can't freeze the
/// UI thread indefinitely.
const RECORDING_EOS_TIMEOUT_MS: u64 = 3000;

pub(crate) struct FrameState {
    pub width: u32,
    pub height: u32,
    pub generation: u64,
    pub frame_count: u64,
    /// Pixels of the most recent frame, in `format`, kept for the window and snapshots.
    ///
    /// `Bytes` is reference-counted, so the appsink callback hands the same
    /// allocation to the window's texture upload and to this field, and
    /// `read_frame` clones it, without ever copying the pixel data.
    pub pixels: Bytes,
    pub format: PixelFormat,
}

/// How `FrameState::pixels` is laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    /// 4 bytes per pixel, `width × height × 4`.
    Rgba,
    /// Tightly packed NV12 (see `domain::yuv`): Y plane, then interleaved UV.
    Nv12(YuvFormat),
}

/// A decoded frame as the window receives it: shared bytes plus how to read them.
#[derive(Clone)]
pub struct VideoFrame {
    pub pixels: Bytes,
    pub width: u32,
    pub height: u32,
    pub format: PixelFormat,
}

impl VideoFrame {
    /// The picture as RGBA, converting on the CPU when it is NV12 (snapshots only:
    /// the window converts on the GPU). `None` if the byte count disagrees with the size.
    pub fn to_rgba(&self) -> Option<Bytes> {
        match self.format {
            PixelFormat::Rgba => (self.pixels.len()
                == self.width as usize * self.height as usize * 4)
                .then(|| self.pixels.clone()),
            PixelFormat::Nv12(fmt) => {
                nv12_to_rgba(&self.pixels, self.width, self.height, fmt).map(Bytes::from)
            }
        }
    }
}

/// Latest frame of the reduced detection branch (see `pipeline::insert_detect_branch`).
///
/// A few hundred pixels wide, so comparing two of them costs a fraction of
/// comparing full-resolution display frames.
pub(crate) struct DetectFrame {
    pub rgba: Bytes,
    pub width: u32,
    pub height: u32,
    /// The frame's timestamp (stream running time, ns): which full-size picture it was cut from.
    pub pts: Option<u64>,
}

/// The pre-roll ring and, while a recording is on, the `appsrc`s that feed it: one for
/// the video (GOP ring) and, when the camera has audio and `record_audio` is on, one
/// for the audio, aligned to start where the video history starts.
pub(crate) struct RingState {
    pub(crate) ring: crate::domain::preroll::GopRing<gst::Sample>,
    /// Set while a recording is running: every sample is forwarded to it.
    pub(crate) sink: Option<gstreamer_app::AppSrc>,
    /// The recording just started: the ring's history has not been delivered yet.
    pub(crate) fresh: bool,
    pub(crate) audio: crate::domain::preroll::AudioRing<gst::Sample>,
    /// Caps of the audio the camera sends (set by the first audio sample).
    pub(crate) audio_caps: Option<gst::Caps>,
    pub(crate) audio_sink: Option<gstreamer_app::AppSrc>,
    pub(crate) audio_fresh: bool,
    /// Where the video history starts (ms): the audio history starts there too. Set by the
    /// video side when it delivers the history; the audio waits for it.
    pub(crate) history_from: Option<i64>,
}

/// The live recording branch, present only while recording.
///
/// Building the encoder chain on demand (rather than leaving it wired up
/// permanently with the sink pointed at `/dev/null`) is what keeps an idle
/// camera from burning a core on H.264 encoding it will never use.
pub(crate) struct RecordingBranch {
    /// The pre-roll ring that feeds this branch (instead of a `tee` pad), if any.
    pub(crate) ring: Option<Arc<Mutex<RingState>>>,
    /// Which `tee` the branch hangs from (`tee` for decoded frames, `enc_tee`
    /// for the camera's own stream).
    pub(crate) tee_name: &'static str,
    /// History database and camera name, to close the last segment on teardown.
    pub(crate) store: Option<(crate::infrastructure::store::StoreHandle, String)>,
    /// Path of the segment `splitmuxsink` has open right now.
    pub(crate) current_segment: Arc<Mutex<Option<String>>>,
    /// The `tee` request pad feeding this branch; must be released on teardown.
    pub(crate) tee_pad: Option<gst::Pad>,
    /// Head of the branch — the pad we inject EOS into to finalise the file.
    pub(crate) queue: gst::Element,
    /// Every element we added to the pipeline, in downstream order.
    pub(crate) elements: Vec<gst::Element>,
    /// Set by a pad probe once EOS has reached the muxer.
    pub(crate) eos_seen: Arc<AtomicBool>,
    pub(crate) started_at: Instant,
}

pub struct GStreamerBridge {
    pub(crate) pipeline: Option<gst::Pipeline>,
    pub(crate) frame: Arc<Mutex<FrameState>>,
    pub(crate) fps_calc: Arc<Mutex<FpsCalc>>,
    pub(crate) is_live: Arc<AtomicBool>,
    pub error_message: Arc<Mutex<Option<String>>>,
    pub(crate) metrics: Arc<Metrics>,
    pub(crate) rtp_jitterbuffers: Vec<gst::Element>,
    /// Set once jitterbuffer discovery is finished (found, or timed out), so
    /// the per-tick `iterate_recurse` graph walk stops. Cleared by `stop()`.
    pub(crate) jb_scan_done: bool,
    /// How many ticks `discover_rtp_jitterbuffers` has scanned since the last
    /// (re)start — bounds the walk to the first few seconds.
    pub(crate) jb_scan_attempts: u32,
    /// Same idea for the decoder lookup (`discover_decoder`).
    pub(crate) decoder_scan_done: bool,
    pub(crate) decoder_scan_attempts: u32,
    pub(crate) start_time: Instant,
    pub recording_config: RecordingConfig,
    /// Build the reduced detection branch when a pipeline starts. Off by
    /// default: with motion detection disabled it would be wasted work.
    pub detect_enabled: bool,
    /// No window will show this camera (the daemon): skip the full-frame RGBA
    /// conversion and copy. Must be set before the pipeline starts.
    pub headless: bool,
    /// Why the next recording starts: `"motion"` or `"manual"` (retention rules differ).
    pub recording_mode: &'static str,
    /// Seconds of pre-roll to keep (0 = no ring). Set before the pipeline starts.
    pub preroll_secs: u32,
    /// Put the camera's audio in recordings (`[recording] record_audio`).
    pub record_audio: bool,
    /// The camera's (unique) display name; recorded files carry it in their name.
    pub camera_label: String,
    /// The pre-roll ring of the running pipeline (RTSP copy path only).
    pub(crate) ring: Option<Arc<Mutex<RingState>>>,
    /// How much video came from the ring when the last recording started (ms): the
    /// first segment's start time is moved back by this much.
    pub(crate) preroll_ms: Arc<std::sync::atomic::AtomicI64>,
    /// History database and this camera's name (set by `Engine::set_store`).
    pub store: Option<(crate::infrastructure::store::StoreHandle, String)>,
    /// Most recent detection frame, `None` until the first one arrives.
    pub(crate) detect_frame: Arc<Mutex<Option<DetectFrame>>>,
    /// Keep the last seconds of decoded samples even when headless (`[detect] snapshot`): a
    /// detection snapshot needs the whole picture the model saw, which the daemon otherwise never
    /// converts nor copies. Set before the pipeline starts.
    pub keep_last_sample: bool,
    /// Those samples (references, not copies; converted only when a snapshot is taken).
    pub(crate) recent_samples: Arc<Mutex<crate::domain::recent_frames::RecentFrames<gst::Sample>>>,
    pub(crate) recording: Option<RecordingBranch>,
    /// Bumped on every `start_recording` so each recording branch gets uniquely
    /// named elements. Without this, a reconnect that stops then immediately
    /// restarts recording could race the (now off-thread) teardown of the
    /// previous branch and collide on the fixed element names.
    pub(crate) recording_seq: u64,
    /// Off-thread recording finalisers still draining EOS. `stop()` joins them
    /// before the pipeline goes to `Null`, otherwise the muxer is torn down
    /// mid-trailer and the last segment is truncated.
    pub(crate) finalisers: Vec<std::thread::JoinHandle<()>>,
    /// Monotonic nanoseconds at which the pipeline last left `Playing`.
    /// 0 = currently connected (or never connected).
    pub(crate) disconnect_mono_ns: AtomicU64,
    /// Per-camera log file. Every GStreamer bus message, state change,
    /// and reconnect event is written here so a dead camera can be
    /// diagnosed from its own log without correlating timestamps
    /// across a single shared file.
    pub(crate) logger: Option<Arc<crate::infrastructure::recording_paths::CameraLogger>>,
}

pub(crate) struct FpsCalc {
    pub last_fps_calc: Instant,
    pub last_frame_count: u64,
    pub fps_snapshot: f64,
    /// Byte counter reading at the last bitrate sample.
    pub last_bytes: u64,
    /// When that byte reading was taken.
    pub last_bytes_at: Instant,
    /// Most recent bitrate in kbit/s, computed from the byte *delta*.
    pub bitrate_kbps: u64,
}

impl GStreamerBridge {
    pub fn new(width: u32, height: u32) -> Result<Self, String> {
        gst::init().map_err(|e| format!("GStreamer init failed: {e}"))?;
        let now = Instant::now();
        Ok(Self {
            pipeline: None,
            frame: Arc::new(Mutex::new(FrameState {
                width,
                height,
                generation: 0,
                frame_count: 0,
                pixels: Bytes::new(),
                format: PixelFormat::Rgba,
            })),
            fps_calc: Arc::new(Mutex::new(FpsCalc {
                last_fps_calc: now,
                last_frame_count: 0,
                fps_snapshot: 0.0,
                last_bytes: 0,
                last_bytes_at: now,
                bitrate_kbps: 0,
            })),
            is_live: Arc::new(AtomicBool::new(false)),
            error_message: Arc::new(Mutex::new(None)),
            metrics: Metrics::new(),
            rtp_jitterbuffers: Vec::new(),
            jb_scan_done: false,
            decoder_scan_done: false,
            decoder_scan_attempts: 0,
            jb_scan_attempts: 0,
            start_time: now,
            recording_config: RecordingConfig::default(),
            detect_enabled: false,
            headless: false,
            recording_mode: "manual",
            preroll_secs: 0,
            record_audio: false,
            camera_label: String::new(),
            ring: None,
            preroll_ms: Arc::new(std::sync::atomic::AtomicI64::new(0)),
            store: None,
            detect_frame: Arc::new(Mutex::new(None)),
            keep_last_sample: false,
            recent_samples: Arc::new(Mutex::new(Default::default())),
            recording: None,
            recording_seq: 0,
            finalisers: Vec::new(),
            disconnect_mono_ns: AtomicU64::new(0),
            logger: None,
        })
    }

    /// Attach a per-camera logger. Called once at startup — the bridge
    /// must already exist because `Arc::new(Mutex::new(bridge))` happens
    /// in `new_app`, and we need a strong reference to set the logger.
    pub fn set_logger(
        &mut self,
        logger: Arc<crate::infrastructure::recording_paths::CameraLogger>,
    ) {
        self.logger = Some(logger);
    }

    /// Write a line to the per-camera log file (if configured).
    /// Falls back to the global `log` crate when no logger is set.
    pub(crate) fn camera_log(&self, level: &str, msg: &str) {
        if let Some(logger) = &self.logger {
            match level {
                "ERROR" => logger.error(msg),
                "WARN" => logger.warn(msg),
                "INFO" => logger.info(msg),
                "DEBUG" => logger.debug(msg),
                _ => logger.write_line(msg),
            }
        } else {
            match level {
                "ERROR" => log::error!("{msg}"),
                "WARN" => log::warn!("{msg}"),
                "INFO" => log::info!("{msg}"),
                "DEBUG" => log::debug!("{msg}"),
                _ => log::info!("{msg}"),
            }
        }
    }

    pub fn metrics(&self) -> &Arc<Metrics> {
        &self.metrics
    }

    pub fn pipeline(&self) -> Option<&gst::Pipeline> {
        self.pipeline.as_ref()
    }

    pub fn uptime_secs(&self) -> u64 {
        self.start_time.elapsed().as_secs()
    }

    pub fn poll_bus(&self) {
        let pipeline = match self.pipeline.as_ref() {
            Some(p) => p,
            None => return,
        };
        let bus = match pipeline.bus() {
            Some(b) => b,
            None => return,
        };
        // Cap the drain per tick: a camera flooding warnings must not stall the
        // UI thread (this runs inside the 100 ms frame tick, and each message
        // may do a `mask_credentials` + log-file write). Leftover messages are
        // picked up on the next tick.
        const MAX_MESSAGES_PER_POLL: usize = 32;
        for _ in 0..MAX_MESSAGES_PER_POLL {
            let Some(msg) = bus.pop_filtered(&[
                gst::MessageType::Error,
                gst::MessageType::Warning,
                gst::MessageType::Eos,
                gst::MessageType::StateChanged,
                gst::MessageType::Latency,
                gst::MessageType::Qos,
                gst::MessageType::Buffering,
            ]) else {
                break;
            };
            match msg.view() {
                gst::MessageView::Buffering(b) => {
                    // `uridecodebin` in buffering mode (adaptive HLS, esp.
                    // fMP4/CMAF) only *posts* buffer level — the app must
                    // pause below 100 % and resume at 100 %, or the pipeline
                    // runs starved and the camera stays black. Live `rtspsrc`
                    // never buffers, so this is a no-op there.
                    let percent = b.percent();
                    if percent < 100 {
                        let _ = pipeline.set_state(gst::State::Paused);
                    } else {
                        let _ = pipeline.set_state(gst::State::Playing);
                    }
                }
                gst::MessageView::Error(e) => {
                    self.mark_disconnected();
                    let err_str = e.error().to_string();
                    let debug_str = e.debug().map(|d| d.to_string()).unwrap_or_default();
                    if message_source_is_decoder(&msg) {
                        self.metrics.decode_errors.fetch_add(1, Ordering::Relaxed);
                    }
                    let masked_err = crate::domain::redact::mask_credentials(&err_str);
                    let masked_dbg = crate::domain::redact::mask_credentials(&debug_str);
                    let line = format!("GStreamer error: {} (debug: {})", masked_err, masked_dbg);
                    self.camera_log("ERROR", &line);
                    self.metrics.set_last_error(err_str.clone());
                    *self.error_message.lock().unwrap_or_else(|e| e.into_inner()) = Some(err_str);
                }
                gst::MessageView::Warning(w) => {
                    if message_source_is_decoder(&msg) {
                        self.metrics.decode_errors.fetch_add(1, Ordering::Relaxed);
                    }
                    let masked = crate::domain::redact::mask_credentials(&w.error().to_string());
                    self.camera_log("WARN", &format!("GStreamer warning: {masked}"));
                }
                gst::MessageView::Eos(_eos) => {
                    self.mark_disconnected();
                    self.metrics.set_last_error("End of stream".to_string());
                    *self.error_message.lock().unwrap_or_else(|e| e.into_inner()) =
                        Some("End of stream".to_string());
                    self.camera_log("ERROR", "GStreamer EOS (stream ended)");
                }
                gst::MessageView::StateChanged(sc) => {
                    // Only the pipeline's own transitions matter. Read the new
                    // state straight off the message — a `pl.state(NONE)` query
                    // blocks the UI thread until a pending async transition
                    // settles.
                    let from_pipeline = msg
                        .src()
                        .map(|obj| obj.downcast_ref::<gst::Pipeline>().is_some())
                        .unwrap_or(false);
                    if from_pipeline {
                        match sc.current() {
                            gst::State::Playing => {
                                self.mark_connected();
                                self.camera_log("INFO", "pipeline → Playing");
                            }
                            gst::State::Null => {
                                self.mark_disconnected();
                                self.camera_log("INFO", "pipeline → Null");
                            }
                            _ => {}
                        }
                    }
                }
                gst::MessageView::Latency(_latency) => {
                    if let Some(obj) = msg.src()
                        && let Ok(pl) = obj.clone().downcast::<gst::Pipeline>()
                    {
                        let _ = pl.recalculate_latency();
                    }
                }
                gst::MessageView::Qos(qos) => {
                    // Count only. QoS "frames dropped" messages arrive in a
                    // flood exactly when a stream is unhealthy — logging each
                    // one (a file write) on the UI tick was a primary cause of
                    // the whole grid freezing.
                    let (_processed, dropped) = qos.stats();
                    let dropped_val = dropped.value() as u64;
                    if dropped_val > 0 {
                        self.metrics
                            .dropped_frames
                            .fetch_add(dropped_val, Ordering::Relaxed);
                    }
                }
                _ => {}
            }
        }
    }

    /// Record that the stream went down, stamping the moment so the next
    /// successful `Playing` transition can report how long the outage lasted.
    fn mark_disconnected(&self) {
        let was_live = self.is_live.swap(false, Ordering::Relaxed);
        self.metrics.is_live.store(false, Ordering::Relaxed);
        if was_live {
            self.disconnect_mono_ns
                .store(self.metrics.mono_ns().max(1), Ordering::Relaxed);
            self.metrics
                .last_disconnect_unix_secs
                .store(now_unix_secs(), Ordering::Relaxed);
        }
    }

    /// Record that the stream came up, closing out any pending outage timer.
    fn mark_connected(&self) {
        self.is_live.store(true, Ordering::Relaxed);
        self.metrics.is_live.store(true, Ordering::Relaxed);
        *self.error_message.lock().unwrap_or_else(|e| e.into_inner()) = None;

        let down_at = self.disconnect_mono_ns.swap(0, Ordering::Relaxed);
        if down_at > 0 {
            let elapsed_ms = self.metrics.mono_ns().saturating_sub(down_at) / 1_000_000;
            self.metrics
                .last_reconnect_duration_ms
                .store(elapsed_ms, Ordering::Relaxed);
        }
    }

    /// Count a reconnect attempt. Called by the UI tick when it decides to
    /// tear the pipeline down and rebuild it.
    pub fn note_reconnect(&self) {
        self.metrics.reconnect_count.fetch_add(1, Ordering::Relaxed);
        self.mark_disconnected();
    }

    pub fn query_latency(&self) {
        if let Some(ref pipeline) = self.pipeline
            && let Some(latency) = pipeline.latency()
        {
            self.metrics
                .actual_latency_ns
                .store(latency.nseconds(), Ordering::Relaxed);
        }
    }

    /// Find which video decoder `decodebin` / `uridecodebin3` actually chose
    /// and publish it in `StreamInfo`, so the Inspector can say CPU or GPU.
    ///
    /// The decoder is created during negotiation, so it only exists once the
    /// stream is live. Same bounded walk as the jitterbuffer scan: stop once
    /// found, or after ~10 s of ticks.
    pub fn discover_decoder(&mut self) {
        if self.decoder_scan_done || !self.is_live() {
            return;
        }
        self.decoder_scan_attempts += 1;
        if self.decoder_scan_attempts > 100 {
            self.decoder_scan_done = true;
            return;
        }
        let Some(ref pipeline) = self.pipeline else {
            return;
        };
        let mut iter = pipeline.iterate_recurse();
        let mut found = None;
        while let Ok(Some(el)) = iter.next() {
            let Some(factory) = el.factory() else {
                continue;
            };
            let klass = factory.metadata("klass").unwrap_or_default();
            if let Some(hw) = crate::domain::metrics::video_decoder_kind(klass) {
                found = Some((factory.name().to_string(), hw, el.clone()));
                break;
            }
        }
        if let Some((name, hw, decoder)) = found {
            self.camera_log(
                "INFO",
                &format!("Decoder: {name} ({})", if hw { "GPU" } else { "CPU" }),
            );
            if let Ok(mut si) = self.metrics.stream_info.lock() {
                si.decoder = Some(name);
                si.decoder_hw = hw;
            }
            // Bitrate is the *compressed* stream: count what enters the
            // decoder. (The appsink only sees decoded RGBA, whose size says
            // nothing about the network rate.)
            if let Some(pad) = decoder.static_pad("sink") {
                let metrics = self.metrics.clone();
                pad.add_probe(gst::PadProbeType::BUFFER, move |_pad, info| {
                    if let Some(gst::PadProbeData::Buffer(ref buf)) = info.data {
                        metrics
                            .bytes_counter
                            .fetch_add(buf.size() as u64, Ordering::Relaxed);
                    }
                    gst::PadProbeReturn::Ok
                });
            }
            self.decoder_scan_done = true;
        }
    }

    pub fn discover_rtp_jitterbuffers(&mut self) {
        // Walking the whole pipeline graph via `iterate_recurse` every 100 ms
        // forever is wasted work — and it runs on the UI thread holding the
        // bridge lock. `rtspsrc` creates its jitterbuffers during session
        // setup, before the first frame, so once the stream is live they are
        // already found (or the source has none, e.g. HLS/`uridecodebin`).
        // Either way, stop scanning. `stop()` clears the flag so a reconnect
        // rediscovers them.
        if self.jb_scan_done {
            return;
        }
        // Bound the scanning: stop once a jitterbuffer is found, or after
        // ~5 s of the pipeline being up (rtspsrc has always created them by
        // then; a source with none — HLS — never will).
        self.jb_scan_attempts += 1;
        if !self.rtp_jitterbuffers.is_empty() || self.jb_scan_attempts > 50 {
            self.jb_scan_done = true;
            return;
        }
        if let Some(ref pipeline) = self.pipeline {
            let mut iter = pipeline.iterate_recurse();
            let mut newly_found = Vec::new();
            loop {
                match iter.next() {
                    Ok(Some(el)) => {
                        let factory_name = el
                            .factory()
                            .map(|f| f.name().to_string())
                            .unwrap_or_default();
                        if factory_name == "rtpjitterbuffer" {
                            newly_found.push(el);
                        }
                    }
                    Ok(None) => break,
                    Err(gst::IteratorError::Resync) => iter.resync(),
                    Err(gst::IteratorError::Error) => break,
                }
            }
            for el in &newly_found {
                if !self.rtp_jitterbuffers.iter().any(|e| e.name() == el.name()) {
                    self.rtp_jitterbuffers.push(el.clone());
                }
            }
        }
    }

    /// Read the `stats` structure off every `rtpjitterbuffer` and aggregate
    /// them into `metrics.packet_stats`.
    ///
    /// `rtspsrc` creates one jitterbuffer per RTP stream, so a camera with
    /// audio has two; the counters are summed because the sidebar reports a
    /// single per-camera loss figure.
    pub fn poll_rtp_stats(&self) {
        let mut agg = PacketStats::default();

        for jb in &self.rtp_jitterbuffers {
            // `property::<T>` panics if the element does not expose `stats` or
            // it is not a `Structure`; a discovered element that turns out not
            // to be a real rtpjitterbuffer must not bring the tick down.
            if !jb.has_property("stats") {
                continue;
            }
            let stats = jb.property::<gst::Structure>("stats");
            agg.available = true;
            agg.pushed = agg
                .pushed
                .saturating_add(stats.get::<u64>("num-pushed").unwrap_or(0));
            agg.lost = agg
                .lost
                .saturating_add(stats.get::<u64>("num-lost").unwrap_or(0));
            agg.late = agg
                .late
                .saturating_add(stats.get::<u64>("num-late").unwrap_or(0));
            agg.duplicates = agg
                .duplicates
                .saturating_add(stats.get::<u64>("num-duplicates").unwrap_or(0));
        }

        if let Ok(mut stats) = self.metrics.packet_stats.lock() {
            // Keep the previous reading if the jitterbuffers vanished (e.g.
            // mid-reconnect) rather than flashing zeros in the sidebar.
            if agg.available || !stats.available {
                *stats = agg;
            }
        }
    }

    pub(crate) fn start_playing(&self) -> Result<(), String> {
        match self.pipeline.as_ref() {
            Some(p) => {
                p.set_state(gst::State::Playing)
                    .map_err(|e| format!("Failed to set state: {e:?}"))?;
                self.camera_log("INFO", "pipeline → Playing");
                Ok(())
            }
            None => {
                self.camera_log("WARN", "start_playing called with no pipeline");
                Ok(())
            }
        }
    }

    /// Tear the pipeline down synchronously: finalise any in-flight recording,
    /// then set the pipeline to `Null`.
    ///
    /// Synchronous on purpose. Doing the `Null` transition on a worker thread
    /// while `start_from_config` immediately builds a replacement raced two
    /// pipelines onto the same camera — for sources that allow a single
    /// connection the new one was refused, producing an endless reconnect
    /// loop that froze every camera. `set_state(Null)` returns in a few ms
    /// for a healthy source; the rare multi-second case is a dead camera,
    /// which is reconnecting regardless.
    pub fn stop(&mut self) {
        if self.is_recording()
            && let Err(e) = self.stop_recording_blocking()
        {
            self.camera_log("WARN", &format!("Failed to finalise recording: {e}"));
        }
        // A `stop_recording()` issued just before this may still be draining.
        for handle in self.finalisers.drain(..) {
            let _ = handle.join();
        }
        // A picture of a camera that is gone must not end up in a later snapshot.
        self.recent_samples
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        if let Some(pipeline) = self.pipeline.take() {
            self.camera_log("INFO", "pipeline → Null (stop)");
            let _ = pipeline.set_state(gst::State::Null);
        }
        self.is_live.store(false, Ordering::Relaxed);
        self.metrics.is_live.store(false, Ordering::Relaxed);
        self.rtp_jitterbuffers.clear();
        self.ring = None;
        self.jb_scan_done = false;
        self.jb_scan_attempts = 0;
        self.decoder_scan_done = false;
        self.decoder_scan_attempts = 0;
        // A new attempt starts clean: a stale error would make the retry
        // logic treat a connecting pipeline as already failed.
        *self.error_message.lock().unwrap_or_else(|e| e.into_inner()) = None;
        // A detection frame from the old stream would be diffed against the
        // new one (another size, another scene) and read as motion.
        *self.detect_frame.lock().unwrap_or_else(|e| e.into_inner()) = None;
        // The next pipeline may be a different stream (sub ↔ main) with another
        // size / codec; the caps probe only fills fields that are still empty.
        if let Ok(mut si) = self.metrics.stream_info.lock() {
            *si = crate::domain::metrics::StreamInfo::default();
        }
    }

    /// Alias kept for the process-exit call sites (`shutdown`, `Drop`).
    pub fn stop_blocking(&mut self) {
        self.stop();
    }

    /// Start recording if idle, stop it if active. Returns the new state.
    pub fn toggle_recording(&mut self) -> Result<bool, String> {
        if self.is_recording() {
            self.stop_recording()?;
            Ok(false)
        } else {
            self.start_recording()?;
            Ok(true)
        }
    }

    pub fn is_recording(&self) -> bool {
        self.recording.is_some()
    }

    pub fn recording_elapsed_secs(&self) -> u64 {
        self.recording
            .as_ref()
            .map(|r| r.started_at.elapsed().as_secs())
            .unwrap_or(0)
    }

    /// Block until the recording branch reports EOS, or the timeout expires.
    /// Returns `true` if EOS was observed.
    pub(crate) fn await_recording_eos(eos_seen: &AtomicBool) -> bool {
        let deadline = Instant::now() + std::time::Duration::from_millis(RECORDING_EOS_TIMEOUT_MS);
        while Instant::now() < deadline {
            if eos_seen.load(Ordering::Relaxed) {
                // Give the muxer a moment to write its index/trailer after
                // the EOS event has been handed to it.
                std::thread::sleep(std::time::Duration::from_millis(50));
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        false
    }

    /// The latest frame (shared bytes, no copy), its size and a generation that
    /// changes with every new frame. `None` until the first frame arrives; the
    /// window uploads it to the GPU only when the generation moves.
    pub fn read_frame(&self) -> (Option<VideoFrame>, u32, u32, u64) {
        let state = self.frame.lock().unwrap_or_else(|e| e.into_inner());
        let frame = (!state.pixels.is_empty()).then(|| VideoFrame {
            pixels: state.pixels.clone(),
            width: state.width,
            height: state.height,
            format: state.format,
        });
        (frame, state.width, state.height, state.generation)
    }

    /// Whether any frame has arrived yet (cheap: nothing is cloned or converted).
    pub fn has_frame(&self) -> bool {
        let state = self.frame.lock().unwrap_or_else(|e| e.into_inner());
        !state.pixels.is_empty()
    }

    /// Most recent frame as raw RGBA, for snapshots. Free for an RGBA frame; an
    /// NV12 one is converted on the CPU here, which only happens on demand.
    pub fn capture_frame(&self) -> Option<(Bytes, u32, u32)> {
        let (frame, w, h, _) = self.read_frame();
        let rgba = frame?.to_rgba()?;
        Some((rgba, w, h))
    }

    /// What a detection snapshot is made from: the frame the window already holds, or (headless)
    /// the decoded sample closest to `pts` (the model's frame), the newest without one. Cheap (references only); the conversion to RGBA is
    /// [`SnapshotSource::into_rgba`], meant for a worker thread.
    pub fn snapshot_source(&self, pts: Option<u64>) -> Option<SnapshotSource> {
        if self.headless {
            let samples = self
                .recent_samples
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            return samples.nearest(pts).map(SnapshotSource::Sample);
        }
        let (frame, w, h, _) = self.read_frame();
        Some(SnapshotSource::Frame(frame?, w, h))
    }

    /// Most recent frame of the reduced detection branch (RGBA, ~320 px wide).
    /// `None` when the branch is off or has not produced a frame yet.
    pub fn capture_detect_frame(&self) -> Option<(Bytes, u32, u32)> {
        self.capture_detect_frame_at().map(|(b, w, h, _)| (b, w, h))
    }

    /// Like [`Self::capture_detect_frame`], with the frame's timestamp.
    pub fn capture_detect_frame_at(&self) -> Option<(Bytes, u32, u32, Option<u64>)> {
        let state = self.detect_frame.lock().unwrap_or_else(|e| e.into_inner());
        state
            .as_ref()
            .map(|f| (f.rgba.clone(), f.width, f.height, f.pts))
    }

    pub fn update_fps(&self) -> f64 {
        let state = self.frame.lock().unwrap_or_else(|e| e.into_inner());
        let mut calc = self.fps_calc.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        let elapsed = now.duration_since(calc.last_fps_calc).as_secs_f64();
        if elapsed >= 0.5 {
            let delta = state.frame_count.wrapping_sub(calc.last_frame_count);
            calc.fps_snapshot = delta as f64 / elapsed;
            calc.last_frame_count = state.frame_count;
            calc.last_fps_calc = now;
        }
        calc.fps_snapshot
    }

    /// Bitrate in kbit/s over the interval since the previous sample.
    ///
    /// `bytes_counter` is cumulative, so the rate has to come from the
    /// delta — dividing the running total by a short interval reports a
    /// number several orders of magnitude too high.
    pub fn update_bitrate_kbps(&self) -> u64 {
        let total = self.metrics.bytes_counter.load(Ordering::Relaxed);
        let mut calc = self.fps_calc.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        let elapsed = now.duration_since(calc.last_bytes_at).as_secs_f64();
        if elapsed >= 0.5 {
            let delta_bytes = total.saturating_sub(calc.last_bytes);
            calc.bitrate_kbps = ((delta_bytes * 8) as f64 / elapsed / 1000.0) as u64;
            calc.last_bytes = total;
            calc.last_bytes_at = now;
        }
        calc.bitrate_kbps
    }

    /// The fps measured by the last health tick (`0.0` before the first one).
    /// Read-only: unlike `update_fps` it does not move the sampling window.
    pub fn last_fps(&self) -> f64 {
        match self.metrics.current_fps_x1000.load(Ordering::Relaxed) {
            u64::MAX => 0.0,
            v => v as f64 / 1000.0,
        }
    }

    /// The compressed bitrate (kbit/s) of the last sample, without sampling again.
    pub fn last_bitrate_kbps(&self) -> u64 {
        self.fps_calc
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .bitrate_kbps
    }

    pub fn is_live(&self) -> bool {
        self.is_live.load(Ordering::Relaxed)
    }

    /// Sample this camera's health. `current` is the status the UI last showed;
    /// `None` means the camera is disabled and nothing should change.
    ///
    /// Pure engine output: the UI folds it into its own row
    /// (`CameraInfo::apply`) and decides how to render it.
    pub fn sample_status(&self, current: &CameraStatus) -> Option<StatusReading> {
        if *current == CameraStatus::Disabled {
            return None;
        }
        let fps = self.update_fps();
        let mut status = None;
        let mut bitrate = None;
        if self.is_live() {
            if let Some(err) = self
                .error_message
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
            {
                status = Some(CameraStatus::Offline);
                bitrate = Some(BitrateReading::Error(err.clone()));
            } else if fps > 0.0 {
                status = Some(CameraStatus::Live);
                bitrate = Some(BitrateReading::Kbps(self.update_bitrate_kbps()));
            }
        } else if self.pipeline.is_some() {
            status = Some(CameraStatus::Reconnecting);
        } else {
            status = Some(CameraStatus::Offline);
        }

        // A live, recording camera reports `Recording` so the red border/dot
        // survives the next tick. Without this, `update_recording` sets the
        // status and this method overwrites it one frame later.
        let effective = status.as_ref().unwrap_or(current);
        if self.is_recording() && *effective == CameraStatus::Live {
            status = Some(CameraStatus::Recording);
        }
        Some(StatusReading {
            fps,
            status,
            bitrate,
        })
    }
}

impl Drop for GStreamerBridge {
    fn drop(&mut self) {
        // Block here: a detached finaliser thread would outlive `self` and, at
        // process exit, be killed before closing the file.
        self.stop_blocking();
    }
}

pub fn now_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// True when a bus message came from an element that looks like a decoder or
/// parser, which is how we attribute errors to the codec rather than the
/// network.
fn message_source_is_decoder(msg: &gst::Message) -> bool {
    let Some(src) = msg.src() else { return false };
    let Ok(element) = src.clone().downcast::<gst::Element>() else {
        return false;
    };
    let factory = element
        .factory()
        .map(|f| f.name().to_string())
        .unwrap_or_default();
    let name = element.name().to_string();
    let looks_like = |s: &str| s.contains("dec") || s.contains("parse");
    looks_like(&factory) || looks_like(&name)
}

pub(crate) fn ema_update(prev: u64, sample: u64, alpha_x1000: u64) -> u64 {
    if prev == 0 {
        return sample;
    }
    let a = alpha_x1000 as f64 / 1000.0;
    let v = prev as f64 * (1.0 - a) + sample as f64 * a;
    v as u64
}

/// Luma delta (0-255) below which two consecutive samples count as "the same
/// scene". Anything at or above this marks a scene change, which is what the
/// tamper detector measures staleness against.
const SCENE_CHANGE_LUMA_DELTA: u8 = 4;

pub(crate) fn sample_image_quality_rgba(
    metrics: &Metrics,
    rgba: &[u8],
    width: usize,
    height: usize,
) {
    sample_image_quality(metrics, width, height, |row, col| {
        let idx = (row * width + col) * 4;
        (idx + 2 < rgba.len()).then(|| {
            let r = rgba[idx] as f64;
            let g = rgba[idx + 1] as f64;
            let b = rgba[idx + 2] as f64;
            (0.299 * r + 0.587 * g + 0.114 * b) as u64
        })
    });
}

/// Same sampling for an NV12 frame: its Y plane *is* the luma, no conversion needed
/// (limited-range black sits at 16; the delta thresholds are relative, so that is fine).
pub(crate) fn sample_image_quality_nv12(
    metrics: &Metrics,
    nv12: &[u8],
    width: usize,
    height: usize,
) {
    sample_image_quality(metrics, width, height, |row, col| {
        nv12.get(row * width + col).map(|&y| y as u64)
    });
}

/// Samples every 8th pixel of every 8th row through `luma(row, col)`, then updates
/// the mean/stddev/scene-change metrics.
fn sample_image_quality(
    metrics: &Metrics,
    width: usize,
    height: usize,
    luma: impl Fn(usize, usize) -> Option<u64>,
) {
    let stride = 8;
    let mut sum: u64 = 0;
    let mut count: u64 = 0;
    let mut row = 0;
    while row < height {
        let mut col = 0;
        while col < width {
            if let Some(l) = luma(row, col) {
                sum += l;
                count += 1;
            }
            col += stride;
        }
        row += stride;
    }
    if count == 0 {
        return;
    }
    let mean = (sum / count) as u8;

    let mut sq_sum: u64 = 0;
    let mean_i = mean as i64;
    row = 0;
    while row < height {
        let mut col = 0;
        while col < width {
            if let Some(l) = luma(row, col) {
                let d = l as i64 - mean_i;
                sq_sum += (d * d) as u64;
            }
            col += stride;
        }
        row += stride;
    }
    let var = sq_sum / count;
    let std = (var as f64).sqrt();
    let stddev = std.min(255.0) as u8;

    // Compare against the *previous* sample before overwriting it — reading
    // it back after the store would always yield a delta of zero, which is
    // what silently disabled tamper detection.
    let prev_luma = metrics.last_avg_luma.load(Ordering::Relaxed);
    let changed = match prev_luma {
        u64::MAX => true, // first ever sample counts as a change
        prev => (prev as i16 - mean as i16).unsigned_abs() as u8 >= SCENE_CHANGE_LUMA_DELTA,
    };

    metrics.last_avg_luma.store(mean as u64, Ordering::Relaxed);
    metrics
        .last_luma_stddev
        .store(stddev as u64, Ordering::Relaxed);

    let now_unix = now_unix_secs();
    if changed {
        metrics
            .last_scene_change_unix_secs
            .store(now_unix.max(1), Ordering::Relaxed);
    }
    metrics
        .last_sample_unix_secs
        .store(now_unix, Ordering::Relaxed);
}

/// A frame to save as a picture, not converted yet (see [`GStreamerBridge::snapshot_source`]).
pub enum SnapshotSource {
    Frame(VideoFrame, u32, u32),
    Sample(gst::Sample),
}

impl SnapshotSource {
    /// The picture as tightly packed RGBA. Slow (a full-frame colour conversion): never on the
    /// engine's loop.
    pub fn into_rgba(self) -> Option<(Vec<u8>, u32, u32)> {
        match self {
            Self::Frame(frame, w, h) => Some((frame.to_rgba()?.to_vec(), w, h)),
            Self::Sample(sample) => {
                let caps = gst::Caps::builder("video/x-raw")
                    .field("format", "RGBA")
                    .build();
                let rgba = gstreamer_video::convert_sample(
                    &sample,
                    &caps,
                    Some(gst::ClockTime::from_seconds(5)),
                )
                .ok()?;
                let info = gstreamer_video::VideoInfo::from_caps(rgba.caps()?).ok()?;
                let (w, h) = (info.width(), info.height());
                let map = rgba.buffer()?.map_readable().ok()?;
                let stride = info.stride()[0] as usize;
                let row = w as usize * 4;
                let mut out = Vec::with_capacity(row * h as usize);
                for y in 0..h as usize {
                    out.extend_from_slice(map.get(y * stride..y * stride + row)?);
                }
                Some((out, w, h))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nv12_quality_sampling_reads_the_y_plane() {
        let m = Metrics::new();
        // 16x16, Y constante 100, UV qualquer: média 100, sem desvio
        let mut nv12 = vec![100u8; 16 * 16];
        nv12.extend(std::iter::repeat_n(128u8, 16 * 8));
        sample_image_quality_nv12(&m, &nv12, 16, 16);
        assert_eq!(m.last_avg_luma.load(Ordering::Relaxed), 100);
        assert_eq!(m.last_luma_stddev.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn bridge_creation_returns_none_frame() {
        let bridge = GStreamerBridge::new(640, 480).unwrap();
        let (frame, _, _, _) = bridge.read_frame();
        assert!(frame.is_none());
    }

    #[test]
    fn bridge_default_size() {
        let bridge = GStreamerBridge::new(1920, 1080).unwrap();
        let (_, w, h, _) = bridge.read_frame();
        assert_eq!((w, h), (1920, 1080));
    }

    #[test]
    fn bridge_starts_not_live() {
        let bridge = GStreamerBridge::new(640, 480).unwrap();
        assert!(!bridge.is_live());
    }

    #[test]
    fn bridge_generation_starts_zero() {
        let bridge = GStreamerBridge::new(640, 480).unwrap();
        let (_, _, _, frame_gen) = bridge.read_frame();
        assert_eq!(frame_gen, 0);
    }

    #[test]
    fn bridge_has_metrics() {
        let bridge = GStreamerBridge::new(640, 480).unwrap();
        let m = bridge.metrics();
        assert_eq!(m.frame_count.load(Ordering::Relaxed), 0);
        assert_eq!(m.bytes_counter.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn ema_update_first_sample_returns_value() {
        assert_eq!(ema_update(0, 100, 200), 100);
    }

    #[test]
    fn ema_update_converges_toward_sample() {
        let mut prev = 100u64;
        for _ in 0..20 {
            prev = ema_update(prev, 200, 200);
        }
        assert!(prev > 180 && prev < 210, "got {prev}");
    }

    #[test]
    fn bridge_starts_not_recording() {
        let bridge = GStreamerBridge::new(640, 480).unwrap();
        assert!(!bridge.is_recording());
        assert_eq!(bridge.recording_elapsed_secs(), 0);
    }

    #[test]
    fn toggle_recording_without_pipeline_errors() {
        let mut bridge = GStreamerBridge::new(640, 480).unwrap();
        let result = bridge.toggle_recording();
        assert!(result.is_err());
    }

    #[test]
    fn recording_config_defaults() {
        let bridge = GStreamerBridge::new(640, 480).unwrap();
        assert_eq!(
            bridge.recording_config.container,
            crate::domain::recording::Container::Mkv
        );
        assert!(!bridge.recording_config.dir.as_os_str().is_empty());
    }

    /// A cumulative byte counter divided by a fresh sub-second interval used
    /// to report gigabit figures for a 2 Mbit stream; the rate must come
    /// from the delta between samples instead.
    #[test]
    fn bitrate_uses_byte_delta_not_running_total() {
        let bridge = GStreamerBridge::new(640, 480).unwrap();

        // Pretend the stream has been running a while and already moved 10 MB.
        bridge
            .metrics
            .bytes_counter
            .store(10_000_000, Ordering::Relaxed);
        {
            let mut calc = bridge.fps_calc.lock().unwrap();
            calc.last_bytes = 10_000_000;
            calc.last_bytes_at = Instant::now() - std::time::Duration::from_secs(1);
        }
        // 125_000 bytes over ~1s == 1000 kbit/s.
        bridge
            .metrics
            .bytes_counter
            .store(10_125_000, Ordering::Relaxed);

        let kbps = bridge.update_bitrate_kbps();
        assert!(
            (900..=1100).contains(&kbps),
            "expected ~1000 kbps from the delta, got {kbps}"
        );
    }

    #[test]
    fn bitrate_holds_last_value_between_samples() {
        let bridge = GStreamerBridge::new(640, 480).unwrap();
        // Interval below the 0.5s sampling floor: keep the previous reading
        // rather than dividing by a near-zero elapsed time.
        bridge
            .metrics
            .bytes_counter
            .store(500_000, Ordering::Relaxed);
        assert_eq!(bridge.update_bitrate_kbps(), 0);
    }

    #[test]
    fn scene_change_is_recorded_on_first_sample() {
        let m = Metrics::new();
        let rgba = vec![128u8; 64 * 64 * 4];
        sample_image_quality_rgba(&m, &rgba, 64, 64);
        assert!(
            m.snapshot_static_secs(now_unix_secs()).is_some(),
            "first sample must seed the scene-change clock"
        );
    }

    #[test]
    fn scene_change_clock_advances_only_when_luma_moves() {
        let m = Metrics::new();

        let dark = vec![10u8; 64 * 64 * 4];
        sample_image_quality_rgba(&m, &dark, 64, 64);
        let after_first = m.last_scene_change_unix_secs.load(Ordering::Relaxed);
        assert!(after_first > 0);

        // Same picture again — the scene-change stamp must not move.
        m.last_scene_change_unix_secs.store(1, Ordering::Relaxed);
        sample_image_quality_rgba(&m, &dark, 64, 64);
        assert_eq!(
            m.last_scene_change_unix_secs.load(Ordering::Relaxed),
            1,
            "an unchanged frame must not count as a scene change"
        );

        // A clearly brighter picture must register.
        let bright = vec![200u8; 64 * 64 * 4];
        sample_image_quality_rgba(&m, &bright, 64, 64);
        assert!(
            m.last_scene_change_unix_secs.load(Ordering::Relaxed) > 1,
            "a large luma jump must register as a scene change"
        );
    }

    #[test]
    fn luma_is_recorded_for_the_sampled_frame() {
        let m = Metrics::new();
        let grey = vec![128u8; 32 * 32 * 4];
        sample_image_quality_rgba(&m, &grey, 32, 32);
        let (avg, dev) = m.snapshot_luma();
        // The BT.601 coefficients sum to just under 1.0 in f64 and the result
        // is truncated, so a flat 128 frame reads back as 127.
        assert!(matches!(avg, Some(127 | 128)), "got {avg:?}");
        assert_eq!(dev, Some(0), "a flat frame has zero contrast");
    }
}
