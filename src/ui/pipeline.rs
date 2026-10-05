use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use crate::domain::codec;
use crate::domain::metrics::{Metrics, MAX_PENDING_DECODES};
use crate::domain::recording::{generate_filename, Container};
use crate::domain::redact::mask_credentials;
use crate::infrastructure::launch::quote_launch_value;
use crate::infrastructure::recording_paths::ensure_recording_dir;

use super::bridge::{
    ema_update, now_unix_secs, sample_image_quality_rgba, GStreamerBridge, RecordingBranch,
    EMA_ALPHA_X1000, SAMPLE_EVERY_N,
};

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;

use iced::advanced::image::Bytes;

/// Name given to the decoder element so the decode-time probes can find it.
const DECODER_NAME: &str = "video_decoder";

/// Post-decode `queue` — its only job is to give the sink its own thread so
/// the decoder never blocks on a slow paint. `leaky=downstream` drops the
/// oldest queued frame instead of back-pressuring the decoder (correct for a
/// live view: always show the freshest frame, never wedge decode). No
/// `max-size-*` caps: buffering is the upstream elements' job (`rtspsrc
/// latency`, `uridecodebin`/`queue2`), not a hand-tuned cache here.
const POSTDEC_QUEUE: &str = "queue name=postdec_queue leaky=downstream";

/// Give the configured decoder element an explicit name so we can attach
/// probes to it, unless the user already named it themselves.
fn named_decoder(decoder: &str) -> String {
    if decoder.contains("name=") {
        decoder.to_string()
    } else {
        format!("{decoder} name={DECODER_NAME}")
    }
}

/// Insert the `tee` that lets a recording branch be attached later.
///
/// Before: `... → capsfilter → appsink`
/// After:  `... → capsfilter → tee → queue(display_queue) → appsink`
///
/// Only the display path is built here. The encoder chain is created on
/// demand by [`GStreamerBridge::start_recording`] — leaving an `x264enc`
/// running permanently with its sink pointed at `/dev/null` costs a core per
/// camera for output nobody asked for.
fn insert_tee(pipeline: &gst::Pipeline) -> Result<(), String> {
    let capsfilter = pipeline
        .by_name("filter")
        .ok_or("capsfilter 'filter' not found")?;
    let appsink_elem = pipeline
        .by_name("display_sink")
        .ok_or("appsink 'display_sink' not found")?;

    let tee = gst::ElementFactory::make("tee")
        .name("tee")
        .build()
        .map_err(|e| format!("Failed to create tee: {e}"))?;
    let display_queue = gst::ElementFactory::make("queue")
        .name("display_queue")
        .build()
        .map_err(|e| format!("Failed to create display_queue: {e}"))?;

    for el in [&tee, &display_queue] {
        pipeline
            .add(el)
            .map_err(|e| format!("Failed to add element: {e}"))?;
        el.sync_state_with_parent()
            .map_err(|e| format!("Failed to sync state: {e}"))?;
    }

    // Disconnect capsfilter → appsink before splicing the tee in.
    if let Some(filter_src) = capsfilter.static_pad("src")
        && let Some(sink_peer) = filter_src.peer() {
            let _ = filter_src.unlink(&sink_peer);
        }

    capsfilter
        .link(&tee)
        .map_err(|e| format!("capsfilter → tee link failed: {e}"))?;
    tee.link(&display_queue)
        .map_err(|e| format!("tee → display_queue link failed: {e}"))?;
    display_queue
        .link(&appsink_elem)
        .map_err(|e| format!("display_queue → appsink link failed: {e}"))?;

    Ok(())
}

/// Measure per-frame decode time by matching buffer PTS between the
/// decoder's input and the post-decode queue's input.
///
/// The previous implementation stamped and looked up PTS at the *same* point
/// (the appsink), so the lookup never hit and the metric stayed at zero.
fn install_decode_time_probes(pipeline: &gst::Pipeline, metrics: &Arc<Metrics>) {
    let (Some(decoder), Some(postdec)) = (
        pipeline.by_name(DECODER_NAME),
        pipeline.by_name("postdec_queue"),
    ) else {
        // HLS/DASH goes through `uridecodebin`, which has no separate decoder
        // element to probe; decode time stays unavailable there.
        return;
    };

    if let Some(pad) = decoder.static_pad("sink") {
        let m = metrics.clone();
        pad.add_probe(gst::PadProbeType::BUFFER, move |_pad, info| {
            if let Some(gst::PadProbeData::Buffer(ref buf)) = info.data
                && let Some(pts) = buf.pts()
                    && let Ok(mut map) = m.pending_decode_starts.lock() {
                        if map.len() >= MAX_PENDING_DECODES
                            && let Some(&k) = map.keys().next() {
                                map.remove(&k);
                            }
                        map.insert(pts.nseconds(), m.mono_ns());
                    }
            gst::PadProbeReturn::Ok
        });
    }

    if let Some(pad) = postdec.static_pad("sink") {
        let m = metrics.clone();
        pad.add_probe(gst::PadProbeType::BUFFER, move |_pad, info| {
            if let Some(gst::PadProbeData::Buffer(ref buf)) = info.data
                && let Some(pts) = buf.pts() {
                    let now = m.mono_ns();
                    if let Ok(mut map) = m.pending_decode_starts.lock()
                        && let Some(start_ns) = map.remove(&pts.nseconds()) {
                            let dt_us = now.saturating_sub(start_ns) / 1_000;
                            let prev = m.decode_time_us_ema.load(Ordering::Relaxed);
                            m.decode_time_us_ema.store(
                                ema_update(prev, dt_us, EMA_ALPHA_X1000),
                                Ordering::Relaxed,
                            );
                        }
                }
            gst::PadProbeReturn::Ok
        });
    }
}

fn setup_appsink(pipeline: &gst::Pipeline, bridge: &mut GStreamerBridge) -> Result<(), String> {
    let appsink_elem = pipeline
        .by_name("display_sink")
        .ok_or("appsink 'display_sink' not found")?;
    let appsink = appsink_elem
        .clone()
        .dynamic_cast::<gst_app::AppSink>()
        .map_err(|_| "Element is not an appsink".to_string())?;

    let frame = bridge.frame.clone();
    let metrics = bridge.metrics.clone();
    let is_live = bridge.is_live.clone();
    appsink.set_callbacks(
        gst_app::AppSinkCallbacks::builder()
            .new_sample(move |appsink| {
                let sample = appsink.pull_sample().map_err(|_| gst::FlowError::Error)?;
                let gst_buffer = sample.buffer().ok_or(gst::FlowError::Error)?;
                let map = gst_buffer.map_readable().map_err(|_| gst::FlowError::Error)?;
                let caps = sample.caps().ok_or(gst::FlowError::Error)?;
                let structure = caps.structure(0).ok_or(gst::FlowError::Error)?;
                let w = structure.get::<i32>("width").unwrap_or(0) as u32;
                let h = structure.get::<i32>("height").unwrap_or(0) as u32;

                // A frame whose byte count disagrees with its caps would make
                // `Handle::from_rgba` render garbage and the snapshot encoder
                // panic. Drop it instead.
                let expected = w as usize * h as usize * 4;
                if w == 0 || h == 0 || map.len() != expected {
                    log::debug!(
                        "Dropping malformed frame: {}x{} with {} bytes (expected {})",
                        w,
                        h,
                        map.len(),
                        expected
                    );
                    return Ok(gst::FlowSuccess::Ok);
                }

                // Frames are flowing → the stream is live, whatever a stale
                // bus error left `is_live` at. Without this, one transient
                // error on a flaky HLS feed pins `is_live=false` forever
                // (nothing else flips it back while the pipeline stays in
                // PLAYING), and the reconnect watchdog then rebuilds the
                // pipeline every 15 s — freezing every camera on the UI thread.
                is_live.store(true, Ordering::Relaxed);

                // Reference-counted, so the handle and the snapshot buffer
                // share one allocation instead of each taking a full copy.
                let bytes = Bytes::copy_from_slice(&map);
                drop(map);

                // `bytes_counter` (bitrate) is fed from the decoder's sink pad
                // — see `GStreamerBridge::discover_decoder`.
                metrics.frame_count.fetch_add(1, Ordering::Relaxed);

                let now_ns = metrics.mono_ns();
                let prev_ns = metrics.last_frame_mono_ns.swap(now_ns, Ordering::Relaxed);
                if prev_ns > 0 {
                    let delta_ns = now_ns.saturating_sub(prev_ns);
                    let si = metrics.snapshot_stream_info();
                    let expected_ns = match (si.framerate_num, si.framerate_den) {
                        (Some(n), Some(d)) if n > 0 && d > 0 => {
                            Some((d as u64 * 1_000_000_000) / n as u64)
                        }
                        _ => None,
                    };
                    let sample_ns = match expected_ns {
                        Some(exp) => {
                            delta_ns.abs_diff(exp)
                        }
                        None => delta_ns,
                    };
                    let prev = metrics.jitter_ema_ns.load(Ordering::Relaxed);
                    metrics.jitter_ema_ns.store(
                        ema_update(prev, sample_ns, EMA_ALPHA_X1000),
                        Ordering::Relaxed,
                    );
                }

                let frame_count = metrics.frame_count.load(Ordering::Relaxed);
                if frame_count.is_multiple_of(SAMPLE_EVERY_N) {
                    sample_image_quality_rgba(&metrics, &bytes, w as usize, h as usize);
                }

                let handle = iced::widget::image::Handle::from_rgba(w, h, bytes.clone());
                let mut state = frame.lock().unwrap_or_else(|e| e.into_inner());
                state.handle = Some(handle);
                state.width = w;
                state.height = h;
                state.raw_rgba = bytes;
                state.generation = state.generation.wrapping_add(1);
                state.frame_count = state.frame_count.wrapping_add(1);
                Ok(gst::FlowSuccess::Ok)
            })
            .build(),
    );

    if let Some(pad) = appsink_elem.static_pad("sink") {
        let metrics = bridge.metrics.clone();
        let pl_pipeline = pipeline.downgrade();
        pad.add_probe(gst::PadProbeType::EVENT_DOWNSTREAM, move |_pad, info| {
            if let Some(gst::PadProbeData::Event(ref ev)) = info.data
                && let gst::EventView::Caps(caps_ev) = ev.view() {
                    let caps = caps_ev.caps();
                    if let Some(s) = caps.structure(0) {
                        let mut si = metrics
                            .stream_info
                            .lock()
                            .unwrap_or_else(|e| e.into_inner());
                        if si.width.is_none()
                            && let Ok(w) = s.get::<i32>("width") {
                                si.width = Some(w);
                            }
                        if si.height.is_none()
                            && let Ok(h) = s.get::<i32>("height") {
                                si.height = Some(h);
                            }
                        if si.framerate_num.is_none()
                            && let Ok(fr) = s.get::<gst::Fraction>("framerate")
                                && fr.denom() != 0 {
                                    si.framerate_num = Some(fr.numer());
                                    si.framerate_den = Some(fr.denom());
                                }
                        if si.codec.is_none() && si.parsed_codec.is_none() {
                            if let Some(parsed) = codec::from_caps(&caps.to_owned()) {
                                si.parsed_codec = Some(parsed);
                            }
                            if let Some(pl) = pl_pipeline.upgrade() {
                                let mut iter = pl.iterate_recurse();
                                loop {
                                    match iter.next() {
                                        Ok(Some(el)) => {
                                            let factory = el
                                                .factory()
                                                .map(|f| f.name().to_string())
                                                .unwrap_or_default();
                                            let name = el.name();
                                            let detected = if name.as_str().contains("h264parse")
                                                || factory.contains("h264parse")
                                            {
                                                Some("H.264")
                                            } else if name.as_str().contains("h265parse")
                                                || factory.contains("h265parse")
                                            {
                                                Some("H.265")
                                            } else if factory == "avdec_h264" {
                                                Some("H.264")
                                            } else if factory == "avdec_h265" {
                                                Some("H.265")
                                            } else if factory.contains("vp8")
                                                && factory.contains("dec")
                                            {
                                                Some("VP8")
                                            } else if factory.contains("vp9")
                                                && factory.contains("dec")
                                            {
                                                Some("VP9")
                                            } else if factory.contains("jpeg")
                                                && factory.contains("dec")
                                            {
                                                Some("MJPEG")
                                            } else {
                                                None
                                            };
                                            if let Some(c) = detected {
                                                si.codec = Some(c.into());
                                                break;
                                            }
                                        }
                                        Ok(None) => break,
                                        Err(gst::IteratorError::Resync) => iter.resync(),
                                        Err(gst::IteratorError::Error) => break,
                                    }
                                }
                            }
                        }
                    }
                }
            gst::PadProbeReturn::Ok
        });
    }

    Ok(())
}

/// Turn `rust-rtsp-viewer-2026-08-10-143022-000.mkv` into
/// `rust-rtsp-viewer-2026-08-10-143022-%03d.mkv`, the pattern `splitmuxsink`
/// expands with the fragment index.
fn segment_location_pattern(now_unix: u64, container: Container) -> String {
    let name = generate_filename(now_unix, 0, container);
    let suffix = format!("-000.{}", container.extension());
    match name.strip_suffix(&suffix) {
        Some(stem) => format!("{stem}-%03d.{}", container.extension()),
        None => name,
    }
}

#[allow(dead_code)]
impl GStreamerBridge {
    /// Build the encoder chain and attach it to the `tee`, starting a new
    /// recording. Segments roll over automatically at the configured
    /// duration/size limits.
    pub fn start_recording(&mut self) -> Result<(), String> {
        if self.is_recording() {
            return Ok(());
        }
        self.recording_seq += 1;
        let seq = self.recording_seq;
        let pipeline = self.pipeline.as_ref().ok_or("No pipeline active")?;
        let tee = pipeline
            .by_name("tee")
            .ok_or("tee not found — pipeline was not built for recording")?;

        self.recording_config
            .validate()
            .map_err(|e| format!("Invalid recording config: {e}"))?;
        ensure_recording_dir(&self.recording_config.dir)
            .map_err(|e| format!("Failed to create recording dir: {e}"))?;

        let container = self.recording_config.container;
        let pattern = segment_location_pattern(now_unix_secs(), container);
        let location = self.recording_config.dir.join(&pattern);

        let queue = gst::ElementFactory::make("queue")
            .name(format!("recording_queue_{seq}"))
            .build()
            .map_err(|e| format!("Failed to create recording_queue: {e}"))?;
        let convert = gst::ElementFactory::make("videoconvert")
            .name(format!("rec_convert_{seq}"))
            .build()
            .map_err(|e| format!("Failed to create rec videoconvert: {e}"))?;
        let encoder = gst::ElementFactory::make("x264enc")
            .name(format!("rec_encoder_{seq}"))
            .property("bitrate", 2048u32)
            .property("key-int-max", 30u32)
            .build()
            .map_err(|e| format!("Failed to create x264enc: {e}"))?;
        encoder.set_property_from_str("speed-preset", "ultrafast");
        encoder.set_property_from_str("tune", "zerolatency");

        let muxer = gst::ElementFactory::make(container.muxer_element())
            .name(format!("rec_muxer_{seq}"))
            .build()
            .map_err(|e| format!("Failed to create {}: {e}", container.muxer_element()))?;

        // splitmuxsink owns the muxer and the file sink, and finalises each
        // segment properly — including the last one, on EOS.
        let sink = gst::ElementFactory::make("splitmuxsink")
            .name(format!("recording_sink_{seq}"))
            .property("location", location.to_string_lossy().to_string())
            .property(
                "max-size-time",
                self.recording_config.max_segment_duration_secs as u64 * 1_000_000_000,
            )
            .property("max-size-bytes", self.recording_config.max_segment_size_bytes)
            .property("send-keyframe-requests", true)
            .property("muxer", &muxer)
            .build()
            .map_err(|e| format!("Failed to create splitmuxsink: {e}"))?;

        let elements = vec![queue.clone(), convert.clone(), encoder.clone(), sink.clone()];
        for el in &elements {
            pipeline
                .add(el)
                .map_err(|e| format!("Failed to add recording element: {e}"))?;
        }

        let tee_pad_slot = std::cell::RefCell::new(None::<gst::Pad>);
        let attach = || -> Result<Arc<AtomicBool>, String> {
            queue
                .link(&convert)
                .map_err(|e| format!("recording_queue → rec_convert link failed: {e}"))?;
            convert
                .link(&encoder)
                .map_err(|e| format!("rec_convert → encoder link failed: {e}"))?;

            let enc_src = encoder
                .static_pad("src")
                .ok_or("encoder has no src pad")?;
            let mux_pad = sink
                .request_pad_simple("video")
                .ok_or("splitmuxsink refused a video pad")?;
            enc_src
                .link(&mux_pad)
                .map_err(|e| format!("encoder → splitmuxsink link failed: {e}"))?;

            // Watch for EOS so teardown knows when the file has been finalised.
            let eos_seen = Arc::new(AtomicBool::new(false));
            let flag = eos_seen.clone();
            mux_pad.add_probe(gst::PadProbeType::EVENT_DOWNSTREAM, move |_pad, info| {
                if let Some(gst::PadProbeData::Event(ref ev)) = info.data
                    && ev.type_() == gst::EventType::Eos {
                        flag.store(true, Ordering::Relaxed);
                    }
                gst::PadProbeReturn::Ok
            });

            // Bring the branch up before data reaches it.
            for el in elements.iter().rev() {
                el.sync_state_with_parent()
                    .map_err(|e| format!("Failed to start recording element: {e}"))?;
            }

            let tee_pad = tee
                .request_pad_simple("src_%u")
                .ok_or("tee refused a src pad")?;
            // Park it first so the error path can release it even if linking fails.
            *tee_pad_slot.borrow_mut() = Some(tee_pad.clone());
            let queue_sink = queue.static_pad("sink").ok_or("queue has no sink pad")?;
            tee_pad
                .link(&queue_sink)
                .map_err(|e| format!("tee → recording_queue link failed: {e}"))?;

            Ok(eos_seen)
        };
        let eos_seen = match attach() {
            Ok(flag) => flag,
            Err(e) => {
                // Don't leave a half-wired branch (or a tee request pad) behind.
                for el in &elements {
                    let _ = el.set_state(gst::State::Null);
                    let _ = pipeline.remove(el);
                }
                if let Some(pad) = tee_pad_slot.borrow_mut().take() {
                    tee.release_request_pad(&pad);
                }
                return Err(e);
            }
        };
        let tee_pad = tee_pad_slot
            .into_inner()
            .ok_or("tee pad missing after attach")?;

        log::info!(
            "Recording started: {} (segments: {}s / {} bytes)",
            location.display(),
            self.recording_config.max_segment_duration_secs,
            self.recording_config.max_segment_size_bytes
        );

        self.recording = Some(RecordingBranch {
            tee_pad,
            queue,
            elements,
            eos_seen,
            started_at: Instant::now(),
        });
        Ok(())
    }

    /// Detach the recording branch from the `tee` and inject EOS so the
    /// encoder flushes and the muxer writes its trailer. Returns the branch
    /// (now detached, EOS in flight) so the caller can drain and dispose of it
    /// either synchronously or on a worker thread.
    pub(crate) fn detach_recording_branch(&mut self) -> Result<Option<RecordingBranch>, String> {
        let Some(branch) = self.recording.take() else {
            return Ok(None);
        };
        if self.pipeline.is_none() {
            // Pipeline already gone; nothing left to unwire.
            return Ok(None);
        }

        let queue_sink = branch
            .queue
            .static_pad("sink")
            .ok_or("recording queue has no sink pad")?;

        let qs = queue_sink.clone();
        branch
            .tee_pad
            .add_probe(gst::PadProbeType::IDLE, move |pad, _info| {
                let _ = pad.unlink(&qs);
                let _ = qs.send_event(gst::event::Eos::new());
                gst::PadProbeReturn::Remove
            });

        Ok(Some(branch))
    }

    /// Wait for EOS to reach the muxer, then set the branch elements to `Null`,
    /// remove them from `pipeline`, and release the `tee` request pad. Blocks
    /// up to `RECORDING_EOS_TIMEOUT_MS`; run on a worker thread when the
    /// calling thread must not stall.
    pub(crate) fn finalise_recording_branch(branch: RecordingBranch, pipeline: gst::Pipeline) {
        if !Self::await_recording_eos(&branch.eos_seen) {
            log::warn!("Recording EOS did not drain in time; last segment may be truncated");
        }
        for el in &branch.elements {
            let _ = el.set_state(gst::State::Null);
        }
        for el in &branch.elements {
            let _ = pipeline.remove(el);
        }
        if let Some(tee) = pipeline.by_name("tee") {
            tee.release_request_pad(&branch.tee_pad);
        }
        log::info!("Recording stopped and finalised");
    }

    /// Stop recording while the pipeline keeps running (user pressed `r`).
    /// The EOS drain + teardown runs on a detached thread so the iced update
    /// loop is not blocked for up to `RECORDING_EOS_TIMEOUT_MS`. Safe because
    /// each branch's elements are uniquely named per `recording_seq`, so a
    /// fresh `start_recording` cannot collide with the in-flight teardown.
    pub fn stop_recording(&mut self) -> Result<(), String> {
        let Some(branch) = self.detach_recording_branch()? else {
            return Ok(());
        };
        let Some(pipeline) = self.pipeline.clone() else {
            return Ok(());
        };
        self.finalisers.retain(|h| !h.is_finished());
        self.finalisers.push(std::thread::spawn(move || {
            Self::finalise_recording_branch(branch, pipeline)
        }));
        Ok(())
    }

    /// Stop recording and block until the segment is finalised. Used on the
    /// teardown paths (`stop`, `Drop`, shutdown) where the pipeline is about to
    /// be set to `Null` and the file must be closed first.
    pub fn stop_recording_blocking(&mut self) -> Result<(), String> {
        let Some(branch) = self.detach_recording_branch()? else {
            return Ok(());
        };
        let Some(pipeline) = self.pipeline.clone() else {
            return Ok(());
        };
        Self::finalise_recording_branch(branch, pipeline);
        Ok(())
    }

    pub fn start_rtsp(
        &mut self,
        url: &str,
        latency_ms: u32,
        do_retransmission: bool,
        decoder: &str,
    ) -> Result<(), String> {
        self.stop();

        let pipeline_str = format!(
            "rtspsrc name=source location={} latency={} protocols=tcp timeout=5000000000 udp-reconnect=true{} \
             ! {} \
             ! {} \
             ! videoconvert name=converter \
             ! capsfilter name=filter caps=\"video/x-raw,format=RGBA\" \
             ! appsink name=display_sink sync=true emit-signals=true max-buffers=2 drop=true",
            quote_launch_value(url),
            latency_ms,
            if do_retransmission { " do-retransmission=true" } else { "" },
            named_decoder(decoder),
            POSTDEC_QUEUE,
        );

        self.camera_log("INFO", &format!("RTSP pipeline: {}", mask_credentials(&pipeline_str)));

        let pipeline = gst::parse_launch(&pipeline_str)
            .map_err(|e| {
                let err = format!("RTSP pipeline parse error: {e}");
                self.camera_log("ERROR", &err);
                err
            })?
            .downcast::<gst::Pipeline>()
            .map_err(|_| "parse_launch must produce a Pipeline".to_string())?;

        setup_appsink(&pipeline, self)?;
        insert_tee(&pipeline)?;
        install_decode_time_probes(&pipeline, &self.metrics);

        self.pipeline = Some(pipeline);
        // Optimistically live: the reconnect watchdog's "not live" branch has a
        // short (15 s) timeout that a slow-starting stream — buffered HLS
        // especially — routinely exceeds, which turned startup latency into a
        // reconnect storm that froze every other camera on the UI thread. The
        // FPS watchdog and bus errors still catch a genuinely dead stream.
        self.is_live.store(true, Ordering::Relaxed);
        self.metrics.is_live.store(true, Ordering::Relaxed);
        self.start_playing()
    }

    pub fn start_from_config(&mut self, cam: &crate::config::CameraConfig) -> Result<(), String> {
        let is_rtsp = cam.url.starts_with("rtsp://") || cam.url.starts_with("rtsps://");
        let is_http = cam.url.starts_with("http://") || cam.url.starts_with("https://");
        let use_uridecodebin = cam.use_uridecodebin.unwrap_or(is_http);
        let do_retransmission = cam.do_retransmission.unwrap_or(true);
        let decoder = cam.decoder.as_deref().unwrap_or("decodebin");

        if is_rtsp && !use_uridecodebin {
            self.start_rtsp(
                &cam.url,
                cam.latency_ms.unwrap_or(100),
                do_retransmission,
                decoder,
            )
        } else if is_http || use_uridecodebin {
            self.start_hls(&cam.url)
        } else {
            self.start_file(&cam.url)
        }
    }

    pub fn start_hls(&mut self, url: &str) -> Result<(), String> {
        self.stop();

        // `uridecodebin3`, not `uridecodebin`: the older one can't give
        // `hlsdemux2` the "streams-aware context" it needs, so fMP4/CMAF HLS
        // (`#EXT-X-MAP` + `.m4s`, e.g. some servicestream.io feeds) silently
        // produced no frames — a black cell. `uridecodebin3` handles both that
        // and classic MPEG-TS HLS. It buffers internally (`queue2`); no extra
        // queue here. Its dynamic video pad is linked to `converter.sink` by
        // the `pad-added` handler below (a bare space, not `!`, keeps it
        // unlinked at parse time).
        let pipeline_str = format!(
            "uridecodebin3 name=source uri={} \
             videoconvert name=converter \
             ! capsfilter name=filter caps=\"video/x-raw,format=RGBA\" \
             ! appsink name=display_sink sync=true emit-signals=true max-buffers=2 drop=true",
            quote_launch_value(url),
        );

        self.camera_log("INFO", &format!("HLS pipeline: {}", mask_credentials(&pipeline_str)));

        let pipeline = gst::parse_launch(&pipeline_str)
            .map_err(|e| format!("HLS pipeline parse error: {e}"))?
            .downcast::<gst::Pipeline>()
            .map_err(|_| "parse_launch must produce a Pipeline".to_string())?;

        let uridecodebin = pipeline
            .by_name("source")
            .ok_or("uridecodebin3 'source' not found")?;
        // Preroll from a small buffer instead of the auto default. The
        // `Buffering` handler in `poll_bus` pauses/resumes on this level.
        uridecodebin.set_property("buffer-duration", 3_000_000_000i64);
        let pipeline_weak = pipeline.downgrade();
        uridecodebin.connect_pad_added(move |_dec, pad| {
            let pad_name = pad.name();
            let current_caps = pad.current_caps();
            let allowed_caps = pad.allowed_caps();
            let caps_debug = current_caps
                .as_ref()
                .map(|c| format!("{:?}", c))
                .or_else(|| allowed_caps.as_ref().map(|c| format!("{:?}", c)))
                .unwrap_or_else(|| "no caps".to_string());

            // Prefer caps; fall back to the pad name (`uridecodebin3` exposes
            // `video_0` / `audio_0` before caps are known).
            let is_video = current_caps
                .or(allowed_caps)
                .and_then(|c| c.structure(0).map(|s| s.name().starts_with("video/")))
                .unwrap_or_else(|| !pad_name.starts_with("audio"));

            log::info!(
                "HLS pad-added: name={}, is_video={}, caps={}",
                pad_name,
                is_video,
                caps_debug
            );

            let Some(pl) = pipeline_weak.upgrade() else {
                log::warn!("HLS pad-added: pipeline dropped");
                return;
            };

            if is_video {
                let Some(conv) = pl.by_name("converter") else {
                    log::warn!("HLS pad-added: converter not found");
                    return;
                };
                let Some(sink) = conv.static_pad("sink") else {
                    log::warn!("HLS pad-added: converter has no sink pad");
                    return;
                };
                if !sink.is_linked() {
                    match pad.link(&sink) {
                        Ok(_) => log::info!("HLS video pad '{}' linked → converter.sink", pad_name),
                        Err(e) => log::warn!("HLS video pad link failed: {:?}", e),
                    }
                    return;
                } else {
                    log::info!(
                        "HLS converter.sink already linked, routing '{}' to fakesink",
                        pad_name
                    );
                }
            }

            log::info!(
                "HLS routing non-video pad '{}' to fakesink (caps={})",
                pad_name,
                caps_debug
            );
            let Ok(fake) = gst::ElementFactory::make("fakesink")
                .property("sync", false)
                .build()
            else {
                return;
            };
            let _ = pl.add(&fake);
            let _ = fake.sync_state_with_parent();
            if let Some(sink) = fake.static_pad("sink") {
                let _ = pad.link(&sink);
            }
        });

        setup_appsink(&pipeline, self)?;
        insert_tee(&pipeline)?;

        self.pipeline = Some(pipeline);
        // Optimistically live: the reconnect watchdog's "not live" branch has a
        // short (15 s) timeout that a slow-starting stream — buffered HLS
        // especially — routinely exceeds, which turned startup latency into a
        // reconnect storm that froze every other camera on the UI thread. The
        // FPS watchdog and bus errors still catch a genuinely dead stream.
        self.is_live.store(true, Ordering::Relaxed);
        self.metrics.is_live.store(true, Ordering::Relaxed);
        self.start_playing()
    }

    pub fn start_file(&mut self, path: &str) -> Result<(), String> {
        self.stop();

        let pipeline_str = format!(
            "filesrc location={} \
             ! decodebin name={} \
             ! {} \
             ! videoconvert name=converter \
             ! capsfilter name=filter caps=\"video/x-raw,format=RGBA\" \
             ! appsink name=display_sink sync=true emit-signals=true max-buffers=2 drop=true",
            quote_launch_value(path),
            DECODER_NAME,
            POSTDEC_QUEUE,
        );

        self.camera_log("INFO", &format!("File pipeline: {}", mask_credentials(&pipeline_str)));

        let pipeline = gst::parse_launch(&pipeline_str)
            .map_err(|e| format!("File pipeline parse error: {e}"))?
            .downcast::<gst::Pipeline>()
            .map_err(|_| "parse_launch must produce a Pipeline".to_string())?;

        setup_appsink(&pipeline, self)?;
        insert_tee(&pipeline)?;
        install_decode_time_probes(&pipeline, &self.metrics);

        self.pipeline = Some(pipeline);
        // Optimistically live: the reconnect watchdog's "not live" branch has a
        // short (15 s) timeout that a slow-starting stream — buffered HLS
        // especially — routinely exceeds, which turned startup latency into a
        // reconnect storm that froze every other camera on the UI thread. The
        // FPS watchdog and bus errors still catch a genuinely dead stream.
        self.is_live.store(true, Ordering::Relaxed);
        self.metrics.is_live.store(true, Ordering::Relaxed);
        self.start_playing()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_plain_url() {
        assert_eq!(
            quote_launch_value("rtsp://cam/live"),
            "\"rtsp://cam/live\""
        );
    }

    #[test]
    fn quotes_path_with_spaces() {
        assert_eq!(
            quote_launch_value("/home/u/My Videos/a.mp4"),
            "\"/home/u/My Videos/a.mp4\""
        );
    }

    #[test]
    fn escapes_embedded_quotes_and_backslashes() {
        assert_eq!(quote_launch_value(r#"a"b\c"#), r#""a\"b\\c""#);
    }

    #[test]
    fn decoder_gets_a_name_for_probing() {
        assert_eq!(named_decoder("decodebin"), "decodebin name=video_decoder");
    }

    #[test]
    fn decoder_keeps_a_user_supplied_name() {
        assert_eq!(named_decoder("decodebin name=mine"), "decodebin name=mine");
    }

    #[test]
    fn segment_pattern_has_a_fragment_placeholder() {
        let p = segment_location_pattern(1_755_000_000, Container::Mkv);
        assert!(p.ends_with("-%03d.mkv"), "got {p}");
        assert!(p.starts_with("rust-rtsp-viewer-"), "got {p}");
    }

    #[test]
    fn segment_pattern_follows_the_container_extension() {
        let p = segment_location_pattern(1_755_000_000, Container::Mp4);
        assert!(p.ends_with("-%03d.mp4"), "got {p}");
    }

    /// A frame reaching the appsink flips `is_live` true even if a stale bus
    /// error left it false — the guard that stops a flaky feed from being
    /// force-reconnected (and freezing every camera) every 15 s.
    #[test]
    fn frame_flow_reasserts_is_live() {
        let _ = gst::init();
        let desc = "videotestsrc is-live=true \
             ! video/x-raw,width=160,height=120,framerate=15/1 \
             ! videoconvert name=converter \
             ! capsfilter name=filter caps=\"video/x-raw,format=RGBA\" \
             ! appsink name=display_sink sync=false emit-signals=true max-buffers=2 drop=true";
        let pipeline = gst::parse_launch(desc)
            .unwrap()
            .downcast::<gst::Pipeline>()
            .unwrap();
        let mut bridge = GStreamerBridge::new(160, 120).unwrap();
        setup_appsink(&pipeline, &mut bridge).unwrap();
        insert_tee(&pipeline).unwrap();
        bridge.pipeline = Some(pipeline);
        bridge.start_playing().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(400));
        // Simulate a stale error having pinned it false.
        bridge.is_live.store(false, Ordering::Relaxed);
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert!(bridge.is_live(), "a flowing frame must reassert is_live");
        bridge.stop();
    }

    /// Play a file back through `decodebin` and report whether it reaches EOS
    /// without error — i.e. whether the muxer actually finalised it.
    fn file_is_playable(path: &std::path::Path) -> bool {
        let desc = format!(
            "filesrc location={} ! decodebin ! fakesink sync=false",
            quote_launch_value(&path.to_string_lossy())
        );
        let Ok(element) = gst::parse_launch(&desc) else {
            return false;
        };
        let Ok(pipeline) = element.downcast::<gst::Pipeline>() else {
            return false;
        };
        if pipeline.set_state(gst::State::Playing).is_err() {
            return false;
        }
        let playable = pipeline
            .bus()
            .and_then(|bus| {
                bus.timed_pop_filtered(
                    gst::ClockTime::from_seconds(10),
                    &[gst::MessageType::Eos, gst::MessageType::Error],
                )
            })
            .map(|msg| matches!(msg.view(), gst::MessageView::Eos(_)))
            .unwrap_or(false);
        let _ = pipeline.set_state(gst::State::Null);
        playable
    }

    /// End-to-end check on the on-demand recording branch: attach it to a
    /// live pipeline, record for a moment, detach it, and confirm the segment
    /// on disk is a complete, decodable file.
    ///
    /// The previous implementation swapped `filesink location` while the
    /// pipeline was playing and never sent EOS, which left the muxer's
    /// trailer unwritten — this test fails against that behaviour.
    #[test]
    fn recording_branch_writes_a_playable_segment() {
        let _ = gst::init();

        let dir = std::env::temp_dir().join(format!("rrv-rec-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let desc = "videotestsrc is-live=true \
             ! video/x-raw,width=320,height=240,framerate=30/1 \
             ! videoconvert name=converter \
             ! capsfilter name=filter caps=\"video/x-raw,format=RGBA\" \
             ! appsink name=display_sink sync=false emit-signals=true max-buffers=2 drop=true";
        let pipeline = gst::parse_launch(desc)
            .expect("test pipeline should parse")
            .downcast::<gst::Pipeline>()
            .expect("should be a pipeline");

        let mut bridge = GStreamerBridge::new(320, 240).unwrap();
        bridge.recording_config.dir = dir.clone();

        setup_appsink(&pipeline, &mut bridge).unwrap();
        insert_tee(&pipeline).unwrap();
        bridge.pipeline = Some(pipeline);
        bridge.start_playing().unwrap();

        // Let frames flow before attaching the encoder.
        std::thread::sleep(std::time::Duration::from_millis(400));
        assert!(!bridge.is_recording());

        bridge.start_recording().expect("recording should start");
        assert!(bridge.is_recording());
        std::thread::sleep(std::time::Duration::from_millis(1200));

        bridge
            .stop_recording_blocking()
            .expect("recording should stop cleanly");
        assert!(!bridge.is_recording());

        let segments: Vec<_> = std::fs::read_dir(&dir)
            .expect("recording dir should exist")
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "mkv"))
            .collect();
        assert_eq!(segments.len(), 1, "expected one segment, got {segments:?}");

        let size = std::fs::metadata(&segments[0]).unwrap().len();
        assert!(size > 1024, "segment is suspiciously small: {size} bytes");
        assert!(
            file_is_playable(&segments[0]),
            "segment did not decode to EOS — the muxer was not finalised"
        );

        bridge.stop();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Stopping a recording must fully detach the branch, so a second
    /// start/stop cycle produces its own independent segment.
    #[test]
    fn recording_can_be_restarted_after_stopping() {
        let _ = gst::init();

        let dir = std::env::temp_dir().join(format!("rrv-rec-restart-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let desc = "videotestsrc is-live=true \
             ! video/x-raw,width=320,height=240,framerate=30/1 \
             ! videoconvert name=converter \
             ! capsfilter name=filter caps=\"video/x-raw,format=RGBA\" \
             ! appsink name=display_sink sync=false emit-signals=true max-buffers=2 drop=true";
        let pipeline = gst::parse_launch(desc)
            .unwrap()
            .downcast::<gst::Pipeline>()
            .unwrap();

        let mut bridge = GStreamerBridge::new(320, 240).unwrap();
        bridge.recording_config.dir = dir.clone();
        setup_appsink(&pipeline, &mut bridge).unwrap();
        insert_tee(&pipeline).unwrap();
        bridge.pipeline = Some(pipeline);
        bridge.start_playing().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(300));

        for cycle in 0..2 {
            bridge
                .start_recording()
                .unwrap_or_else(|e| panic!("cycle {cycle} start failed: {e}"));
            std::thread::sleep(std::time::Duration::from_millis(900));
            bridge
                .stop_recording_blocking()
                .unwrap_or_else(|e| panic!("cycle {cycle} stop failed: {e}"));
            // Distinct timestamps keep the two cycles in separate files.
            std::thread::sleep(std::time::Duration::from_millis(1100));
        }

        let segments: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "mkv"))
            .collect();
        assert_eq!(segments.len(), 2, "expected two segments, got {segments:?}");
        for seg in &segments {
            assert!(file_is_playable(seg), "{} is not playable", seg.display());
        }

        bridge.stop();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `stop_recording()` hands teardown to a worker thread; a `stop()` right
    /// behind it (page flip, reconnect, quit) must wait for that worker
    /// instead of nulling the pipeline under the muxer.
    #[test]
    fn stop_right_after_async_stop_recording_still_finalises() {
        let _ = gst::init();

        let dir = std::env::temp_dir().join(format!("rrv-rec-race-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let desc = "videotestsrc is-live=true \
             ! video/x-raw,width=320,height=240,framerate=30/1 \
             ! videoconvert name=converter \
             ! capsfilter name=filter caps=\"video/x-raw,format=RGBA\" \
             ! appsink name=display_sink sync=false emit-signals=true max-buffers=2 drop=true";
        let pipeline = gst::parse_launch(desc)
            .unwrap()
            .downcast::<gst::Pipeline>()
            .unwrap();

        let mut bridge = GStreamerBridge::new(320, 240).unwrap();
        bridge.recording_config.dir = dir.clone();
        setup_appsink(&pipeline, &mut bridge).unwrap();
        insert_tee(&pipeline).unwrap();
        bridge.pipeline = Some(pipeline);
        bridge.start_playing().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(300));

        bridge.start_recording().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1000));
        bridge.stop_recording().unwrap();
        bridge.stop(); // immediately, while the worker is still draining

        let segments: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "mkv"))
            .collect();
        assert_eq!(segments.len(), 1, "got {segments:?}");
        assert!(file_is_playable(&segments[0]), "segment truncated by early stop()");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
