use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::domain::codec;
use crate::domain::metrics::{MAX_PENDING_DECODES, Metrics};
use crate::domain::recording::{Container, generate_filename};
use crate::domain::redact::mask_credentials;
use crate::infrastructure::launch::quote_launch_value;
use crate::infrastructure::recording_paths::ensure_recording_dir;
use crate::infrastructure::store::StoreCmd;

use super::bridge::{
    EMA_ALPHA_X1000, GStreamerBridge, RecordingBranch, SAMPLE_EVERY_N, ema_update, now_unix_secs,
    sample_image_quality_rgba,
};

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;

use bytes::Bytes;

/// Name given to the decoder element so the decode-time probes can find it.
const DECODER_NAME: &str = "video_decoder";

/// Post-decode `queue` — its only job is to give the sink its own thread so
/// the decoder never blocks on a slow paint. `leaky=downstream` drops the
/// oldest queued frame instead of back-pressuring the decoder (correct for a
/// live view: always show the freshest frame, never wedge decode). No
/// `max-size-*` caps: buffering is the upstream elements' job (`rtspsrc
/// latency`, `uridecodebin`/`queue2`), not a hand-tuned cache here.
const POSTDEC_QUEUE: &str = "queue name=postdec_queue leaky=downstream";

/// Size of the reduced detection frame. Fixed (not aspect-preserving): zones
/// are normalised 0..1 and the detector only compares frames with each other,
/// so stretching is harmless and the cost per camera is constant.
const DETECT_WIDTH: i32 = 320;
const DETECT_HEIGHT: i32 = 180;
/// Detection frames per second. The detector samples at ~2 Hz (see
/// `update::detect_camera_motion`), so more would be dropped unread.
const DETECT_FPS: i32 = 2;

/// Give the configured decoder element an explicit name so we can attach
/// probes to it, unless the user already named it themselves.
fn named_decoder(decoder: &str) -> String {
    if decoder.contains("name=") {
        decoder.to_string()
    } else {
        format!("{decoder} name={DECODER_NAME}")
    }
}

/// Name of the `tee` on the *encoded* stream (before the decoder).
const ENCODED_TEE: &str = "enc_tee";

/// The encoded tap: `parsebin ! tee ! queue`, placed right after `rtspsrc`, so a
/// recording can take the camera's own H.264/H.265 and write it without
/// decoding and re-encoding it (plan 3.1). Only for the default `decodebin`: a
/// custom `decoder` chain (e.g. one that starts with `rtph264depay`) expects RTP.
fn encoded_tap(decoder: &str, explicit_pads: bool) -> String {
    if decoder.trim() == "decodebin" {
        // With `explicit_pads` the `rtspsrc → parsebin` link is left out of the string:
        // `connect_rtsp_pads` makes it, choosing the *video* pad and leaving the audio
        // for its own branch (the launch syntax would link whichever pad came first).
        let link = if explicit_pads { "" } else { "! " };
        format!("{link}parsebin name=parser ! tee name={ENCODED_TEE} ! queue name=enc_dec_queue")
    } else {
        String::new()
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
        && let Some(sink_peer) = filter_src.peer()
    {
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

/// Attach the reduced detection branch to the `tee`:
///
/// `tee → queue(leaky) → videorate(2 fps) → videoscale(320 px) → RGBA → appsink`
///
/// The sink is `async=false`: it must not take part in the pipeline's preroll.
/// `videorate` holds the first buffer until it sees a second, and in PAUSED only
/// the preroll frame flows, so a sink that waited for it left the *whole*
/// pipeline stuck in an async change to PLAYING (one frame shown, then frozen).
///
/// `videorate` comes *before* `videoscale` so only [`DETECT_FPS`] frames a
/// second are resized, instead of every decoded frame. Motion detection reads
/// this small frame instead of diffing full-resolution display frames. The
/// branch lives for the whole pipeline (it is cheap); only the cameras that
/// need it (`bridge.detect_enabled`) get it.
fn insert_detect_branch(pipeline: &gst::Pipeline, bridge: &GStreamerBridge) -> Result<(), String> {
    let tee = pipeline.by_name("tee").ok_or("tee not found")?;
    let desc = format!(
        "queue name=detect_queue leaky=downstream max-size-buffers=2 max-size-bytes=0 \
         max-size-time=0 \
         ! videorate name=detect_rate drop-only=true \
         ! video/x-raw,framerate={DETECT_FPS}/1 \
         ! videoscale \
         ! video/x-raw,width={DETECT_WIDTH},height={DETECT_HEIGHT} \
         ! videoconvert \
         ! video/x-raw,format=RGBA \
         ! appsink name=detect_sink sync=false async=false emit-signals=true max-buffers=1 drop=true"
    );
    let bin = gst::parse::bin_from_description(&desc, true)
        .map_err(|e| format!("detect branch parse error: {e}"))?;
    let appsink = bin
        .by_name("detect_sink")
        .ok_or("appsink 'detect_sink' not found")?
        .dynamic_cast::<gst_app::AppSink>()
        .map_err(|_| "detect_sink is not an appsink".to_string())?;

    let slot = bridge.detect_frame.clone();
    appsink.set_callbacks(
        gst_app::AppSinkCallbacks::builder()
            .new_sample(move |sink| {
                let sample = sink.pull_sample().map_err(|_| gst::FlowError::Error)?;
                let buffer = sample.buffer().ok_or(gst::FlowError::Error)?;
                let map = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;
                let caps = sample.caps().ok_or(gst::FlowError::Error)?;
                let st = caps.structure(0).ok_or(gst::FlowError::Error)?;
                let w = st.get::<i32>("width").unwrap_or(0) as u32;
                let h = st.get::<i32>("height").unwrap_or(0) as u32;
                // Same guard as the display sink: a frame that disagrees with
                // its caps would make the detector index out of bounds.
                if w == 0 || h == 0 || map.len() != w as usize * h as usize * 4 {
                    return Ok(gst::FlowSuccess::Ok);
                }
                let rgba = Bytes::copy_from_slice(&map);
                *slot.lock().unwrap_or_else(|e| e.into_inner()) =
                    Some(super::bridge::DetectFrame {
                        rgba,
                        width: w,
                        height: h,
                    });
                Ok(gst::FlowSuccess::Ok)
            })
            .build(),
    );

    pipeline
        .add(&bin)
        .map_err(|e| format!("Failed to add detect branch: {e}"))?;
    bin.sync_state_with_parent()
        .map_err(|e| format!("Failed to sync detect branch: {e}"))?;
    let tee_pad = tee
        .request_pad_simple("src_%u")
        .ok_or("tee refused a src pad for detection")?;
    let sink_pad = bin.static_pad("sink").ok_or("detect branch has no sink")?;
    tee_pad
        .link(&sink_pad)
        .map_err(|e| format!("tee → detect branch link failed: {e}"))?;
    Ok(())
}

/// Attach the pre-roll ring to the encoded tap:
/// `enc_tee → queue(leaky) → appsink(ring_sink)`. Every encoded buffer lands in a
/// [`GopRing`](crate::domain::preroll::GopRing); a recording that starts drains it first, so
/// the file begins before the motion that triggered it (plan 3.6). The sink is
/// `async=false` like every branch sink that is not the display.
/// A fresh ring. The audio window is the pre-roll plus the longest GOP the video ring keeps,
/// so the audio history always reaches back as far as the video's.
fn new_ring_state(preroll_secs: u32) -> super::bridge::RingState {
    super::bridge::RingState {
        ring: crate::domain::preroll::GopRing::new(preroll_secs as i64 * 1000),
        sink: None,
        fresh: false,
        audio: crate::domain::preroll::AudioRing::new(
            preroll_secs as i64 * 1000 + crate::domain::preroll::MAX_GOP_SPAN_MS + 2_000,
        ),
        audio_caps: None,
        audio_sink: None,
        audio_fresh: false,
        history_from: None,
    }
}

/// Make `appsink` (the camera's parsed audio) feed the audio ring and, while a recording
/// runs, its audio `appsrc`.
fn attach_audio_sink(appsink: &gst_app::AppSink, state: Arc<Mutex<super::bridge::RingState>>) {
    appsink.set_callbacks(
        gst_app::AppSinkCallbacks::builder()
            .new_sample(move |sink| {
                let sample = sink.pull_sample().map_err(|_| gst::FlowError::Error)?;
                let Some(buf) = sample.buffer() else {
                    return Ok(gst::FlowSuccess::Ok);
                };
                let mut st = state.lock().unwrap_or_else(|e| e.into_inner());
                let pts = buf
                    .pts()
                    .or(buf.dts())
                    .map(|t| t.mseconds() as i64)
                    .or(st.audio.newest_pts_ms())
                    .unwrap_or(0);
                if st.audio_caps.is_none() {
                    st.audio_caps = sample.caps().map(|c| c.to_owned());
                }
                st.audio.push(pts, sample.clone());
                if let Some(src) = st.audio_sink.clone() {
                    if st.audio_fresh {
                        // Wait until the video side has said where its history starts,
                        // so both tracks begin together.
                        if let Some(from) = st.history_from {
                            st.audio_fresh = false;
                            for s in st.audio.since(from) {
                                let _ = src.push_sample(&s);
                            }
                        }
                    } else {
                        let _ = src.push_sample(&sample);
                    }
                }
                Ok(gst::FlowSuccess::Ok)
            })
            .build(),
    );
}

/// Link the `rtspsrc` pads by hand: the video pad to the parser (the encoded tap) and the
/// audio pad to its own `parsebin → appsink(audio_ring_sink)`, which feeds the audio ring.
/// Call before the pipeline starts; pads appear while it connects.
fn connect_rtsp_pads(
    pipeline: &gst::Pipeline,
    state: Arc<Mutex<super::bridge::RingState>>,
) -> Result<(), String> {
    let source = pipeline
        .by_name("source")
        .ok_or("rtspsrc 'source' not found")?;
    let parser = pipeline
        .by_name("parser")
        .ok_or("parsebin 'parser' not found")?;
    let weak = pipeline.downgrade();
    let audio_done = Arc::new(AtomicBool::new(false));
    source.connect_pad_added(move |_, pad| {
        let Some(pipeline) = weak.upgrade() else {
            return;
        };
        let caps = pad.current_caps().unwrap_or_else(|| pad.query_caps(None));
        let media = caps
            .structure(0)
            .and_then(|s| s.get::<String>("media").ok())
            .unwrap_or_default();
        match media.as_str() {
            "video" => {
                if let Some(sink) = parser.static_pad("sink")
                    && !sink.is_linked()
                    && let Err(e) = pad.link(&sink)
                {
                    log::warn!("video pad → parser link failed: {e:?}");
                }
            }
            "audio" if !audio_done.swap(true, Ordering::SeqCst) => {
                if let Err(e) = build_audio_branch(&pipeline, pad, state.clone()) {
                    log::warn!("audio branch not built (the recording will have no audio): {e}");
                }
            }
            _ => {}
        }
    });
    Ok(())
}

/// `audio pad → parsebin → appsink(audio_ring_sink)`.
fn build_audio_branch(
    pipeline: &gst::Pipeline,
    pad: &gst::Pad,
    state: Arc<Mutex<super::bridge::RingState>>,
) -> Result<(), String> {
    let parse = gst::ElementFactory::make("parsebin")
        .name("audio_parser")
        .build()
        .map_err(|e| e.to_string())?;
    let sink = gst::ElementFactory::make("appsink")
        .name("audio_ring_sink")
        .property("sync", false)
        .property("async", false)
        .property("emit-signals", true)
        .build()
        .map_err(|e| e.to_string())?
        .downcast::<gst_app::AppSink>()
        .map_err(|_| "audio_ring_sink is not an appsink".to_string())?;
    attach_audio_sink(&sink, state);
    pipeline
        .add_many([&parse, sink.upcast_ref()])
        .map_err(|e| e.to_string())?;
    let sink_el = sink.clone().upcast::<gst::Element>();
    parse.connect_pad_added(move |_, p| {
        if let Some(sp) = sink_el.static_pad("sink")
            && !sp.is_linked()
            && let Err(e) = p.link(&sp)
        {
            log::warn!("audio parser → sink link failed: {e:?}");
        }
    });
    parse.sync_state_with_parent().map_err(|e| e.to_string())?;
    sink.sync_state_with_parent().map_err(|e| e.to_string())?;
    pad.link(&parse.static_pad("sink").ok_or("parsebin has no sink")?)
        .map(|_| ())
        .map_err(|e| format!("{e:?}"))
}

fn insert_ring_branch(
    pipeline: &gst::Pipeline,
    bridge: &mut GStreamerBridge,
) -> Result<(), String> {
    let Some(tee) = pipeline.by_name(ENCODED_TEE) else {
        return Ok(()); // HLS, file or a custom decoder: no encoded tap, no ring
    };
    let desc = "queue name=ring_queue leaky=downstream max-size-buffers=0 max-size-bytes=0 \
                max-size-time=4000000000 \
                ! appsink name=ring_sink sync=false async=false emit-signals=true";
    let bin = gst::parse::bin_from_description(desc, true)
        .map_err(|e| format!("ring branch parse error: {e}"))?;
    let appsink = bin
        .by_name("ring_sink")
        .ok_or("appsink 'ring_sink' not found")?
        .dynamic_cast::<gst_app::AppSink>()
        .map_err(|_| "ring_sink is not an appsink".to_string())?;

    let state = Arc::new(Mutex::new(new_ring_state(bridge.preroll_secs)));
    let cb_state = state.clone();
    let preroll_ms = bridge.preroll_ms.clone();
    appsink.set_callbacks(
        gst_app::AppSinkCallbacks::builder()
            .new_sample(move |sink| {
                let sample = sink.pull_sample().map_err(|_| gst::FlowError::Error)?;
                let Some(buf) = sample.buffer() else {
                    return Ok(gst::FlowSuccess::Ok);
                };
                let key = !buf.flags().contains(gst::BufferFlags::DELTA_UNIT);
                let mut st = cb_state.lock().unwrap_or_else(|e| e.into_inner());
                // A buffer with no PTS (it happens at discontinuities) must not enter the
                // ring as time 0: that would put it out of order and the ring would never
                // trim. DTS, or else the previous instant, keeps the order.
                let pts = buf
                    .pts()
                    .or(buf.dts())
                    .map(|t| t.mseconds() as i64)
                    .or(st.ring.newest_pts_ms())
                    .unwrap_or(0);
                st.ring.push(pts, key, sample.clone());
                if let Some(src) = st.sink.clone() {
                    if st.fresh {
                        // The recording just started: the history (which ends with
                        // this very sample) goes in first, then live samples follow.
                        st.fresh = false;
                        st.history_from = st.ring.oldest_pts_ms();
                        preroll_ms.store(st.ring.span_ms(), Ordering::Relaxed);
                        for s in st.ring.history() {
                            let _ = src.push_sample(&s);
                        }
                    } else {
                        let _ = src.push_sample(&sample);
                    }
                }
                Ok(gst::FlowSuccess::Ok)
            })
            .build(),
    );

    pipeline
        .add(&bin)
        .map_err(|e| format!("Failed to add ring branch: {e}"))?;
    bin.sync_state_with_parent()
        .map_err(|e| format!("Failed to sync ring branch: {e}"))?;
    let tee_pad = tee
        .request_pad_simple("src_%u")
        .ok_or("tee refused a src pad for the ring")?;
    let sink_pad = bin.static_pad("sink").ok_or("ring branch has no sink")?;
    tee_pad
        .link(&sink_pad)
        .map_err(|e| format!("tee → ring branch link failed: {e}"))?;
    bridge.ring = Some(state);
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
                && let Ok(mut map) = m.pending_decode_starts.lock()
            {
                if map.len() >= MAX_PENDING_DECODES
                    && let Some(&k) = map.keys().next()
                {
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
                && let Some(pts) = buf.pts()
            {
                let now = m.mono_ns();
                if let Ok(mut map) = m.pending_decode_starts.lock()
                    && let Some(start_ns) = map.remove(&pts.nseconds())
                {
                    let dt_us = now.saturating_sub(start_ns) / 1_000;
                    let prev = m.decode_time_us_ema.load(Ordering::Relaxed);
                    m.decode_time_us_ema
                        .store(ema_update(prev, dt_us, EMA_ALPHA_X1000), Ordering::Relaxed);
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
    let headless = bridge.headless;
    if headless && let Some(filter) = pipeline.by_name("filter") {
        // No RGBA conversion for a picture nobody shows: the decoder's own
        // format flows on to the recording and detection branches.
        filter.set_property("caps", gst::Caps::new_empty_simple("video/x-raw"));
    }
    // The same handler serves `new_sample` (playing) and `new_preroll` (the frame a
    // paused pipeline shows after a seek or a step), so scrubbing a recording works.
    let on_sample = move |appsink: &gst_app::AppSink, preroll: bool| {
        // `pull_sample` inside the preroll callback would block forever: the sample
        // only exists once preroll is done.
        let sample = if preroll {
            appsink.pull_preroll()
        } else {
            appsink.pull_sample()
        }
        .map_err(|_| gst::FlowError::Error)?;
        let gst_buffer = sample.buffer().ok_or(gst::FlowError::Error)?;
        let caps = sample.caps().ok_or(gst::FlowError::Error)?;
        let structure = caps.structure(0).ok_or(gst::FlowError::Error)?;
        let w = structure.get::<i32>("width").unwrap_or(0) as u32;
        let h = structure.get::<i32>("height").unwrap_or(0) as u32;

        // Headless: nobody looks at the picture, so the frame is not
        // converted to RGBA nor copied; it only proves flow.
        let bytes = if headless {
            Bytes::new()
        } else {
            let map = gst_buffer
                .map_readable()
                .map_err(|_| gst::FlowError::Error)?;
            // A frame whose byte count disagrees with its caps would
            // make `Handle::from_rgba` render garbage and the snapshot
            // encoder panic. Drop it instead.
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
            // Reference-counted, so the handle and the snapshot buffer
            // share one allocation instead of each taking a full copy.
            Bytes::copy_from_slice(&map)
        };

        // Frames are flowing → the stream is live, whatever a stale
        // bus error left `is_live` at. Without this, one transient
        // error on a flaky HLS feed pins `is_live=false` forever
        // (nothing else flips it back while the pipeline stays in
        // PLAYING), and the reconnect watchdog then rebuilds the
        // pipeline every 15 s — freezing every camera on the UI thread.
        is_live.store(true, Ordering::Relaxed);

        // `bytes_counter` (bitrate) is fed from the decoder's sink pad
        // — see `GStreamerBridge::discover_decoder`.
        metrics.frame_count.fetch_add(1, Ordering::Relaxed);

        let now_ns = metrics.mono_ns();
        let prev_ns = metrics.last_frame_mono_ns.swap(now_ns, Ordering::Relaxed);
        if prev_ns > 0 {
            let delta_ns = now_ns.saturating_sub(prev_ns);
            let si = metrics.snapshot_stream_info();
            let expected_ns = match (si.framerate_num, si.framerate_den) {
                (Some(n), Some(d)) if n > 0 && d > 0 => Some((d as u64 * 1_000_000_000) / n as u64),
                _ => None,
            };
            let sample_ns = match expected_ns {
                Some(exp) => delta_ns.abs_diff(exp),
                None => delta_ns,
            };
            let prev = metrics.jitter_ema_ns.load(Ordering::Relaxed);
            metrics.jitter_ema_ns.store(
                ema_update(prev, sample_ns, EMA_ALPHA_X1000),
                Ordering::Relaxed,
            );
        }

        let frame_count = metrics.frame_count.load(Ordering::Relaxed);
        if !headless && frame_count.is_multiple_of(SAMPLE_EVERY_N) {
            sample_image_quality_rgba(&metrics, &bytes, w as usize, h as usize);
        }

        let mut state = frame.lock().unwrap_or_else(|e| e.into_inner());
        state.width = w;
        state.height = h;
        state.raw_rgba = bytes;
        state.generation = state.generation.wrapping_add(1);
        state.frame_count = state.frame_count.wrapping_add(1);
        Ok(gst::FlowSuccess::Ok)
    };
    appsink.set_callbacks(
        gst_app::AppSinkCallbacks::builder()
            .new_sample({
                let f = on_sample.clone();
                move |a| f(a, false)
            })
            .new_preroll(move |a| on_sample(a, true))
            .build(),
    );

    if let Some(pad) = appsink_elem.static_pad("sink") {
        let metrics = bridge.metrics.clone();
        let pl_pipeline = pipeline.downgrade();
        pad.add_probe(gst::PadProbeType::EVENT_DOWNSTREAM, move |_pad, info| {
            if let Some(gst::PadProbeData::Event(ref ev)) = info.data
                && let gst::EventView::Caps(caps_ev) = ev.view()
            {
                let caps = caps_ev.caps();
                if let Some(s) = caps.structure(0) {
                    let mut si = metrics
                        .stream_info
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    if si.width.is_none()
                        && let Ok(w) = s.get::<i32>("width")
                    {
                        si.width = Some(w);
                    }
                    if si.height.is_none()
                        && let Ok(h) = s.get::<i32>("height")
                    {
                        si.height = Some(h);
                    }
                    if si.framerate_num.is_none()
                        && let Ok(fr) = s.get::<gst::Fraction>("framerate")
                        && fr.denom() != 0
                    {
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
                                        } else if factory.contains("vp8") && factory.contains("dec")
                                        {
                                            Some("VP8")
                                        } else if factory.contains("vp9") && factory.contains("dec")
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

/// Software decoders (`avdec_*`) default to one thread per CPU core, each holding
/// frames in flight. With many cameras that multiplies memory and wakeups for no
/// gain (a camera stream is a single decode at 20–30 fps). Cap them.
const DECODER_THREADS: i32 = 2;

/// Threads per software decoder: `RRV_DECODER_THREADS` (1–16) or [`DECODER_THREADS`].
fn decoder_threads() -> i32 {
    std::env::var("RRV_DECODER_THREADS")
        .ok()
        .and_then(|v| v.parse::<i32>().ok())
        .filter(|n| (1..=16).contains(n))
        .unwrap_or(DECODER_THREADS)
}

/// Limit `max-threads` of every software decoder the pipeline creates, now or
/// later (`decodebin` creates its decoder at runtime, hence the signal).
fn limit_decoder_threads(pipeline: &gst::Pipeline) {
    let threads = decoder_threads();
    let cap = move |el: &gst::Element| {
        let factory = el.factory().map(|f| f.name().to_string());
        let name = factory.as_deref().unwrap_or_default();
        if name.starts_with("avdec_") && el.has_property("max-threads") {
            el.set_property("max-threads", threads);
        }
        // A recording that joins the encoded stream mid-way needs the SPS/PPS
        // (VPS for H.265) in front of every keyframe, not only at the start.
        if (name == "h264parse" || name == "h265parse") && el.has_property("config-interval") {
            el.set_property("config-interval", -1i32);
        }
    };
    pipeline.connect("deep-element-added", false, move |args| {
        if let Ok(el) = args[2].get::<gst::Element>() {
            cap(&el);
        }
        None
    });
}

/// Turn `rust-rtsp-viewer-2026-08-10-143022-000.mkv` into
/// `rust-rtsp-viewer-<camera>-2026-08-10-143022-%03d.mkv`, the pattern `splitmuxsink`
/// expands with the fragment index.
///
/// The camera is part of the name on purpose: two cameras that start recording in the
/// same second (motion in a room with several of them) would otherwise get the *same*
/// file name and one would overwrite the other.
fn segment_location_pattern(now_unix: u64, container: Container, camera: &str) -> String {
    let name = generate_filename(now_unix, 0, container);
    let suffix = format!("-000.{}", container.extension());
    let stem = match name.strip_suffix(&suffix) {
        Some(stem) => stem.to_string(),
        None => return name,
    };
    let camera = crate::infrastructure::recording_paths::safe_filename(camera);
    let stem = match (camera.is_empty(), stem.strip_prefix("rust-rtsp-viewer-")) {
        (false, Some(rest)) => format!("rust-rtsp-viewer-{camera}-{rest}"),
        _ => stem,
    };
    format!("{stem}-%03d.{}", container.extension())
}

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
        // Prefer the camera's own stream (no decode, no encode) when it is
        // H.264/H.265 and the pipeline has the encoded tap.
        let parser_factory = pipeline.by_name(ENCODED_TEE).and_then(|t| {
            let caps = t.static_pad("sink")?.current_caps()?;
            match caps.structure(0)?.name().as_str() {
                "video/x-h264" => Some("h264parse"),
                "video/x-h265" => Some("h265parse"),
                _ => None,
            }
        });
        if parser_factory.is_none()
            && let Some(t) = pipeline.by_name(ENCODED_TEE)
        {
            log::info!(
                "Encoded tap not usable for recording (caps: {:?}); re-encoding",
                t.static_pad("sink").and_then(|p| p.current_caps())
            );
        }
        // With a pre-roll ring that already holds video, the recording is fed by the
        // ring (history first, then live) instead of a `tee` pad.
        let ring_state: Option<Arc<Mutex<super::bridge::RingState>>> = if parser_factory.is_some() {
            self.ring
                .clone()
                .filter(|r| !r.lock().unwrap_or_else(|e| e.into_inner()).ring.is_empty())
        } else {
            None
        };
        self.preroll_ms.store(0, Ordering::Relaxed);
        if let Some(r) = &ring_state {
            let st = r.lock().unwrap_or_else(|e| e.into_inner());
            log::debug!(
                "ring at start: video {:?}..{} ms, audio {:?} ({} samples)",
                st.ring.oldest_pts_ms(),
                st.ring.span_ms(),
                st.audio.bounds(),
                st.audio.len()
            );
        }
        let (tee_name, tee) = match parser_factory {
            Some(_) => (ENCODED_TEE, pipeline.by_name(ENCODED_TEE)),
            None => ("tee", pipeline.by_name("tee")),
        };
        let tee = tee.ok_or("tee not found — pipeline was not built for recording")?;

        self.recording_config
            .validate()
            .map_err(|e| format!("Invalid recording config: {e}"))?;
        ensure_recording_dir(&self.recording_config.dir)
            .map_err(|e| format!("Failed to create recording dir: {e}"))?;

        let container = self.recording_config.container;
        let pattern = segment_location_pattern(now_unix_secs(), container, &self.camera_label);
        let location = self.recording_config.dir.join(&pattern);

        let queue: gst::Element = match &ring_state {
            Some(r) => {
                let caps = r
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .ring
                    .newest()
                    .and_then(|s| s.caps().map(|c| c.to_owned()));
                let src = gst::ElementFactory::make("appsrc")
                    .name(format!("recording_src_{seq}"))
                    .property("format", gst::Format::Time)
                    .property("max-bytes", 0u64)
                    .build()
                    .map_err(|e| format!("Failed to create recording appsrc: {e}"))?;
                if let Some(c) = caps {
                    src.set_property("caps", c);
                }
                src
            }
            None => gst::ElementFactory::make("queue")
                .name(format!("recording_queue_{seq}"))
                .build()
                .map_err(|e| format!("Failed to create recording_queue: {e}"))?,
        };
        // Re-encode path (decoded frames): videoconvert → x264enc. Copy path: a
        // parser only, so no CPU goes into encoding.
        let chain: Vec<gst::Element> = match parser_factory {
            Some(factory) => vec![
                gst::ElementFactory::make(factory)
                    .name(format!("rec_parse_{seq}"))
                    .property("config-interval", -1i32)
                    .build()
                    .map_err(|e| format!("Failed to create {factory}: {e}"))?,
            ],
            None => {
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
                vec![convert, encoder]
            }
        };

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
            .property(
                "max-size-bytes",
                self.recording_config.max_segment_size_bytes,
            )
            .property("send-keyframe-requests", true)
            // splitmuxsink is a sink-like bin whose own state change is async.
            // Added to a pipeline that is already PLAYING, that made the whole
            // pipeline lose its state and drop back to PAUSED, where nothing
            // flows, so the splitmuxsink never got the first buffer it needed
            // to finish its change: a deadlock (picture frozen, recording
            // empty). `async-handling` keeps the async change inside the bin.
            .property("async-handling", true)
            .property("muxer", &muxer)
            .build()
            .map_err(|e| format!("Failed to create splitmuxsink: {e}"))?;

        // Tell the history database about every segment as it opens and closes.
        let current_segment = Arc::new(Mutex::new(None::<String>));
        if let Some((store, camera)) = self.store.clone() {
            let current = current_segment.clone();
            let mode = self.recording_mode.to_string();
            let pattern = location.to_string_lossy().to_string();
            let preroll_ms = self.preroll_ms.clone();
            sink.connect("format-location", false, move |args| {
                let id = args[1].get::<u32>().unwrap_or(0);
                let path = pattern.replacen("%03d", &format!("{id:03}"), 1);
                let mut now = crate::engine::now_ms();
                if id == 0 {
                    // The first file starts with the pre-roll: it begins that much
                    // before the moment the recording was asked for.
                    now -= preroll_ms.load(Ordering::Relaxed);
                }
                let mut cur = current.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(prev) = cur.replace(path.clone()) {
                    store.send(StoreCmd::SegmentClosed {
                        path: prev,
                        ts_end: now,
                    });
                }
                store.send(StoreCmd::SegmentOpened {
                    camera: camera.clone(),
                    path: path.clone(),
                    ts: now,
                    mode: mode.clone(),
                });
                Some(path.to_value())
            });
        }

        // The audio track, when the camera has one and `record_audio` is on: an `appsrc`
        // fed by the audio ring, in the same file as the video.
        let audio_src: Option<gst::Element> = if self.record_audio {
            ring_state.as_ref().and_then(|r| {
                let caps = r
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .audio_caps
                    .clone()?;
                gst::ElementFactory::make("appsrc")
                    .name(format!("recording_audio_src_{seq}"))
                    .property("format", gst::Format::Time)
                    .property("max-bytes", 0u64)
                    .property("caps", caps)
                    .build()
                    .ok()
            })
        } else {
            None
        };

        let mut elements = vec![queue.clone()];
        elements.extend(chain.iter().cloned());
        if let Some(a) = &audio_src {
            elements.push(a.clone());
        }
        elements.push(sink.clone());
        for el in &elements {
            pipeline
                .add(el)
                .map_err(|e| format!("Failed to add recording element: {e}"))?;
        }

        let tee_pad_slot = std::cell::RefCell::new(None::<gst::Pad>);
        let audio_linked = std::cell::Cell::new(false);
        let attach = || -> Result<Arc<AtomicBool>, String> {
            let mut upstream = queue.clone();
            for el in &chain {
                upstream
                    .link(el)
                    .map_err(|e| format!("{} → {} link failed: {e}", upstream.name(), el.name()))?;
                upstream = el.clone();
            }

            let enc_src = upstream.static_pad("src").ok_or("encoder has no src pad")?;
            if parser_factory.is_some() {
                // The file must start on a keyframe: a segment that begins in
                // the middle of a GOP shows grey/garbage until the next one.
                let seen_key = AtomicBool::new(false);
                enc_src.add_probe(gst::PadProbeType::BUFFER, move |_pad, info| {
                    if let Some(gst::PadProbeData::Buffer(ref b)) = info.data {
                        if seen_key.load(Ordering::Relaxed) {
                            return gst::PadProbeReturn::Ok;
                        }
                        if !b.flags().contains(gst::BufferFlags::DELTA_UNIT) {
                            seen_key.store(true, Ordering::Relaxed);
                            return gst::PadProbeReturn::Ok;
                        }
                        return gst::PadProbeReturn::Drop;
                    }
                    gst::PadProbeReturn::Ok
                });
            }
            let mux_pad = sink
                .request_pad_simple("video")
                .ok_or("splitmuxsink refused a video pad")?;
            enc_src
                .link(&mux_pad)
                .map_err(|e| format!("encoder → splitmuxsink link failed: {e}"))?;

            // Watch for EOS so teardown knows when the file has been finalised: once on
            // every track (video, and audio when there is one).
            let eos_seen = Arc::new(AtomicBool::new(false));
            let eos_count = Arc::new(AtomicUsize::new(0));
            let eos_expected = Arc::new(AtomicUsize::new(1));
            let watch_eos = |pad: &gst::Pad| {
                let (flag, count, expected) =
                    (eos_seen.clone(), eos_count.clone(), eos_expected.clone());
                pad.add_probe(gst::PadProbeType::EVENT_DOWNSTREAM, move |_pad, info| {
                    if let Some(gst::PadProbeData::Event(ref ev)) = info.data
                        && ev.type_() == gst::EventType::Eos
                        && count.fetch_add(1, Ordering::SeqCst) + 1
                            >= expected.load(Ordering::SeqCst)
                    {
                        flag.store(true, Ordering::Relaxed);
                    }
                    gst::PadProbeReturn::Ok
                });
            };
            watch_eos(&mux_pad);
            if let Some(a) = &audio_src {
                // Ask the muxer first: linking a format it cannot take (G.711 in an mp4) fails
                // later with a pipeline ERROR, and the engine would take the camera for dead
                // and reconnect it every time a recording starts.
                let audio_caps = a.property::<Option<gst::Caps>>("caps");
                match (sink.request_pad_simple("audio_%u"), a.static_pad("src")) {
                    (Some(apad), _)
                        if audio_caps
                            .as_ref()
                            .is_some_and(|c| !apad.query_accept_caps(c)) =>
                    {
                        log::warn!(
                            "recording without audio: the {} muxer does not take {}",
                            container.extension(),
                            audio_caps
                                .as_ref()
                                .map_or_else(String::new, |c| c.to_string())
                        );
                        sink.release_request_pad(&apad);
                    }
                    (Some(apad), Some(asrc)) => match asrc.link(&apad) {
                        Ok(_) => {
                            watch_eos(&apad);
                            eos_expected.store(2, Ordering::SeqCst);
                            audio_linked.set(true);
                        }
                        Err(e) => {
                            log::warn!("recording without audio (link failed: {e:?})");
                            sink.release_request_pad(&apad);
                        }
                    },
                    _ => log::warn!("recording without audio (the muxer has no audio pad)"),
                }
            }

            // Bring the branch up before data reaches it.
            for el in elements.iter().rev() {
                el.sync_state_with_parent()
                    .map_err(|e| format!("Failed to start recording element: {e}"))?;
            }

            if let Some(r) = &ring_state {
                // Hand the branch to the ring: from the next sample on it delivers the
                // history and then the live stream.
                let src = queue
                    .clone()
                    .downcast::<gst_app::AppSrc>()
                    .map_err(|_| "recording source is not an appsrc".to_string())?;
                let mut st = r.lock().unwrap_or_else(|e| e.into_inner());
                st.sink = Some(src);
                st.fresh = true;
                st.history_from = None;
                if audio_linked.get()
                    && let Some(a) = &audio_src
                    && let Ok(asrc) = a.clone().downcast::<gst_app::AppSrc>()
                {
                    st.audio_sink = Some(asrc);
                    st.audio_fresh = true;
                }
            } else {
                let tee_pad = tee
                    .request_pad_simple("src_%u")
                    .ok_or("tee refused a src pad")?;
                // Park it first so the error path can release it even if linking fails.
                *tee_pad_slot.borrow_mut() = Some(tee_pad.clone());
                let queue_sink = queue.static_pad("sink").ok_or("queue has no sink pad")?;
                tee_pad
                    .link(&queue_sink)
                    .map_err(|e| format!("tee → recording_queue link failed: {e}"))?;
            }

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
        let tee_pad = tee_pad_slot.into_inner();
        if tee_pad.is_none() && ring_state.is_none() {
            return Err("tee pad missing after attach".into());
        }

        log::info!(
            "Recording started{}: {} (segments: {}s / {} bytes)",
            if ring_state.is_some() {
                " (camera stream, no re-encode, with pre-roll)"
            } else if parser_factory.is_some() {
                " (camera stream, no re-encode)"
            } else {
                ""
            },
            location.display(),
            self.recording_config.max_segment_duration_secs,
            self.recording_config.max_segment_size_bytes
        );

        self.recording = Some(RecordingBranch {
            ring: ring_state.clone(),
            tee_name,
            store: self.store.clone(),
            current_segment,
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

        // Fed by the pre-roll ring: take the appsrc out of the ring under its lock (no
        // sample is pushed after that) and end its stream.
        if let Some(ring) = &branch.ring {
            let (src, audio_src) = {
                let mut st = ring.lock().unwrap_or_else(|e| e.into_inner());
                (st.sink.take(), st.audio_sink.take())
            };
            if let Some(src) = src {
                let _ = src.end_of_stream();
            }
            if let Some(a) = audio_src {
                let _ = a.end_of_stream();
            }
            return Ok(Some(branch));
        }

        let queue_sink = branch
            .queue
            .static_pad("sink")
            .ok_or("recording queue has no sink pad")?;
        let tee_pad = branch.tee_pad.clone().ok_or("recording has no tee pad")?;

        let qs = queue_sink.clone();
        tee_pad.add_probe(gst::PadProbeType::IDLE, move |pad, _info| {
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
        if let Some(pad) = &branch.tee_pad
            && let Some(tee) = pipeline.by_name(branch.tee_name)
        {
            tee.release_request_pad(pad);
        }
        if let Some((store, _)) = &branch.store
            && let Some(path) = branch
                .current_segment
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take()
        {
            store.send(StoreCmd::SegmentClosed {
                path,
                ts_end: crate::engine::now_ms(),
            });
        }
        log::info!("Recording stopped and finalised");
    }

    /// Stop recording while the pipeline keeps running (user pressed `r`).
    /// The EOS drain + teardown runs on a detached thread so the UI update
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

        // The audio recording needs the pads of the source handled one by one.
        let explicit_pads = self.record_audio && decoder.trim() == "decodebin";
        let pipeline_str = format!(
            "rtspsrc name=source location={} latency={} protocols=tcp timeout=5000000000 udp-reconnect=true{} \
             {} \
             ! {} \
             ! {} \
             ! videoconvert name=converter \
             ! capsfilter name=filter caps=\"video/x-raw,format=RGBA\" \
             ! appsink name=display_sink sync=true emit-signals=true max-buffers=2 drop=true",
            quote_launch_value(url),
            latency_ms,
            if do_retransmission {
                " do-retransmission=true"
            } else {
                ""
            },
            encoded_tap(decoder, explicit_pads),
            named_decoder(decoder),
            POSTDEC_QUEUE,
        );

        self.camera_log(
            "INFO",
            &format!("RTSP pipeline: {}", mask_credentials(&pipeline_str)),
        );

        let pipeline = gst::parse::launch(&pipeline_str)
            .map_err(|e| {
                let err = format!("RTSP pipeline parse error: {e}");
                self.camera_log("ERROR", &err);
                err
            })?
            .downcast::<gst::Pipeline>()
            .map_err(|_| "parse_launch must produce a Pipeline".to_string())?;

        limit_decoder_threads(&pipeline);
        setup_appsink(&pipeline, self)?;
        insert_tee(&pipeline)?;
        if self.detect_enabled {
            insert_detect_branch(&pipeline, self)?;
        }
        self.ring = None;
        if self.preroll_secs > 0 || explicit_pads {
            insert_ring_branch(&pipeline, self)?;
        }
        if explicit_pads {
            match self.ring.clone() {
                Some(state) => connect_rtsp_pads(&pipeline, state)?,
                None => return Err("no encoded tap to record audio from".into()),
            }
        }
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

        self.camera_log(
            "INFO",
            &format!("HLS pipeline: {}", mask_credentials(&pipeline_str)),
        );

        let pipeline = gst::parse::launch(&pipeline_str)
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

        limit_decoder_threads(&pipeline);
        setup_appsink(&pipeline, self)?;
        insert_tee(&pipeline)?;
        if self.detect_enabled {
            insert_detect_branch(&pipeline, self)?;
        }

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

        self.camera_log(
            "INFO",
            &format!("File pipeline: {}", mask_credentials(&pipeline_str)),
        );

        let pipeline = gst::parse::launch(&pipeline_str)
            .map_err(|e| format!("File pipeline parse error: {e}"))?
            .downcast::<gst::Pipeline>()
            .map_err(|_| "parse_launch must produce a Pipeline".to_string())?;

        limit_decoder_threads(&pipeline);
        setup_appsink(&pipeline, self)?;
        insert_tee(&pipeline)?;
        if self.detect_enabled {
            insert_detect_branch(&pipeline, self)?;
        }
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
        assert_eq!(quote_launch_value("rtsp://cam/live"), "\"rtsp://cam/live\"");
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
        let p = segment_location_pattern(1_755_000_000, Container::Mkv, "");
        assert!(p.ends_with("-%03d.mkv"), "got {p}");
        assert!(p.starts_with("rust-rtsp-viewer-"), "got {p}");
    }

    /// Two cameras starting in the same second must not share a file name.
    #[test]
    fn the_segment_name_carries_the_camera() {
        let a = segment_location_pattern(1_755_000_000, Container::Mkv, "Garagem");
        let b = segment_location_pattern(1_755_000_000, Container::Mkv, "Portão da frente");
        assert_ne!(a, b);
        assert!(a.starts_with("rust-rtsp-viewer-garagem-"), "got {a}");
        assert!(b.starts_with("rust-rtsp-viewer-port"), "got {b}");
        assert!(!b.contains(' ') && !b.contains('/'), "safe for a path: {b}");
        // a hostile name cannot escape the recordings directory: no path separator survives
        let evil = segment_location_pattern(1_755_000_000, Container::Mkv, "../../etc/x");
        assert!(!evil.contains('/') && !evil.contains('\\'), "got {evil}");
        assert!(evil.starts_with("rust-rtsp-viewer-"), "got {evil}");
    }

    #[test]
    fn segment_pattern_follows_the_container_extension() {
        let p = segment_location_pattern(1_755_000_000, Container::Mp4, "");
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
        let pipeline = gst::parse::launch(desc)
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

    /// The detection branch delivers small RGBA frames, not full-resolution
    /// ones, and stops delivering once the bridge is stopped.
    #[test]
    fn detect_branch_delivers_reduced_frames() {
        let _ = gst::init();
        let desc = "videotestsrc is-live=true \
             ! video/x-raw,width=1280,height=720,framerate=30/1 \
             ! videoconvert name=converter \
             ! capsfilter name=filter caps=\"video/x-raw,format=RGBA\" \
             ! appsink name=display_sink sync=false emit-signals=true max-buffers=2 drop=true";
        let pipeline = gst::parse::launch(desc)
            .unwrap()
            .downcast::<gst::Pipeline>()
            .unwrap();
        let mut bridge = GStreamerBridge::new(1280, 720).unwrap();
        setup_appsink(&pipeline, &mut bridge).unwrap();
        insert_tee(&pipeline).unwrap();
        insert_detect_branch(&pipeline, &bridge).unwrap();
        bridge.pipeline = Some(pipeline);
        bridge.start_playing().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1500));

        let (rgba, w, h) = bridge
            .capture_detect_frame()
            .expect("the detection branch produced no frame");
        assert_eq!(w, DETECT_WIDTH as u32);
        assert_eq!(h, DETECT_HEIGHT as u32);
        assert_eq!(rgba.len(), (w * h * 4) as usize);
        // The display path keeps its full resolution.
        let (_, dw, dh) = bridge.capture_frame().expect("no display frame");
        assert_eq!((dw, dh), (1280, 720));

        bridge.stop();
        assert!(
            bridge.capture_detect_frame().is_none(),
            "stop() must drop the stale detection frame"
        );
    }

    /// Play a file back through `decodebin` and report whether it reaches EOS
    /// without error — i.e. whether the muxer actually finalised it.
    fn file_is_playable(path: &std::path::Path) -> bool {
        let desc = format!(
            "filesrc location={} ! decodebin ! fakesink sync=false",
            quote_launch_value(&path.to_string_lossy())
        );
        let Ok(element) = gst::parse::launch(&desc) else {
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
        let pipeline = gst::parse::launch(desc)
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

    /// With the encoded tap the recording takes the camera's own H.264: there is
    /// no re-encoder in the branch, the file is playable and it starts on a
    /// keyframe, even though it was attached in the middle of a GOP.
    #[test]
    fn recording_from_the_encoded_tap_does_not_reencode() {
        let _ = gst::init();

        let dir = std::env::temp_dir().join(format!("rrv-rec-copy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        // Long GOP (60 frames = 2 s), so attaching at 0.4 s lands mid-GOP.
        let desc = "videotestsrc is-live=true \
             ! video/x-raw,format=I420,width=320,height=240,framerate=30/1 \
             ! x264enc tune=zerolatency speed-preset=ultrafast key-int-max=60 \
             ! h264parse \
             ! tee name=enc_tee \
             ! queue \
             ! avdec_h264 \
             ! videoconvert name=converter \
             ! capsfilter name=filter caps=\"video/x-raw,format=RGBA\" \
             ! appsink name=display_sink sync=false emit-signals=true max-buffers=2 drop=true";
        let pipeline = gst::parse::launch(desc)
            .unwrap()
            .downcast::<gst::Pipeline>()
            .unwrap();
        let mut bridge = GStreamerBridge::new(320, 240).unwrap();
        bridge.recording_config.dir = dir.clone();
        setup_appsink(&pipeline, &mut bridge).unwrap();
        insert_tee(&pipeline).unwrap();
        bridge.pipeline = Some(pipeline.clone());
        bridge.start_playing().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(400));

        bridge.start_recording().expect("recording should start");
        assert!(
            !pipeline
                .iterate_recurse()
                .into_iter()
                .flatten()
                .any(|e| e.name().starts_with("rec_encoder")),
            "the copy path must not add an encoder"
        );
        assert!(pipeline.by_name("rec_parse_1").is_some());
        std::thread::sleep(std::time::Duration::from_millis(3500));
        bridge.stop_recording_blocking().unwrap();

        let segments: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "mkv"))
            .collect();
        assert_eq!(segments.len(), 1, "expected one segment, got {segments:?}");
        assert!(std::fs::metadata(&segments[0]).unwrap().len() > 1024);
        assert!(
            file_is_playable(&segments[0]),
            "the copied segment did not decode to EOS"
        );
        bridge.stop();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Duration (ms) of a media file, from a paused decodebin.
    fn file_duration_ms(path: &std::path::Path) -> u64 {
        let desc = format!(
            "filesrc location={} ! decodebin ! fakesink sync=false",
            quote_launch_value(&path.to_string_lossy())
        );
        let p = gst::parse::launch(&desc)
            .unwrap()
            .downcast::<gst::Pipeline>()
            .unwrap();
        p.set_state(gst::State::Paused).unwrap();
        let _ = p.state(gst::ClockTime::from_seconds(5));
        let d = p
            .query_duration::<gst::ClockTime>()
            .map_or(0, |d| d.mseconds());
        let _ = p.set_state(gst::State::Null);
        d
    }

    /// Plan 3.6: a recording that starts on the pre-roll ring begins *before* the moment
    /// it was asked for. A 1.5 s recording with a 2 s ring must be well over 3 s long
    /// (without the ring it would be about 1.5 s), still playable, with no re-encode.
    #[test]
    fn a_recording_started_on_the_ring_includes_the_pre_roll() {
        let _ = gst::init();
        let dir = std::env::temp_dir().join(format!("rrv-rec-ring-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        // GOP of 1 s so the ring keeps whole GOPs close to the asked 2 s.
        let desc = "videotestsrc is-live=true \
             ! video/x-raw,format=I420,width=320,height=240,framerate=30/1 \
             ! x264enc tune=zerolatency speed-preset=ultrafast key-int-max=30 \
             ! h264parse \
             ! tee name=enc_tee \
             ! queue \
             ! avdec_h264 \
             ! videoconvert name=converter \
             ! capsfilter name=filter caps=\"video/x-raw,format=RGBA\" \
             ! appsink name=display_sink sync=false emit-signals=true max-buffers=2 drop=true";
        let pipeline = gst::parse::launch(desc)
            .unwrap()
            .downcast::<gst::Pipeline>()
            .unwrap();
        let mut bridge = GStreamerBridge::new(320, 240).unwrap();
        bridge.recording_config.dir = dir.clone();
        bridge.preroll_secs = 2;
        setup_appsink(&pipeline, &mut bridge).unwrap();
        insert_tee(&pipeline).unwrap();
        insert_ring_branch(&pipeline, &mut bridge).unwrap();
        bridge.pipeline = Some(pipeline.clone());
        bridge.start_playing().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(3500)); // the ring fills

        bridge.start_recording().expect("recording should start");
        assert!(
            pipeline.by_name("recording_src_1").is_some(),
            "fed by the ring, not by a tee pad"
        );
        std::thread::sleep(std::time::Duration::from_millis(1500));
        bridge.stop_recording_blocking().unwrap();

        let segments: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "mkv"))
            .collect();
        assert_eq!(segments.len(), 1, "{segments:?}");
        assert!(file_is_playable(&segments[0]), "did not play to EOS");
        let dur = file_duration_ms(&segments[0]);
        assert!(
            (3_000..=5_500).contains(&dur),
            "expected ~1.5 s + 2..3 s of pre-roll, got {dur} ms"
        );
        assert!(
            bridge.preroll_ms.load(Ordering::Relaxed) >= 1_900,
            "the first segment's start must be moved back by the pre-roll"
        );
        bridge.stop();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The media kinds of a matroska file's tracks ("video/x-h264", "audio/mpeg", ...).
    fn file_track_kinds(path: &std::path::Path) -> Vec<String> {
        let demux = if path.extension().is_some_and(|e| e == "mp4") {
            "qtdemux"
        } else {
            "matroskademux"
        };
        let desc = format!(
            "filesrc location={} ! {demux} name=d",
            quote_launch_value(&path.to_string_lossy())
        );
        let p = gst::parse::launch(&desc)
            .unwrap()
            .downcast::<gst::Pipeline>()
            .unwrap();
        let kinds = Arc::new(Mutex::new(Vec::<String>::new()));
        let k = kinds.clone();
        p.by_name("d").unwrap().connect_pad_added(move |_, pad| {
            let caps = pad.current_caps().unwrap_or_else(|| pad.query_caps(None));
            if let Some(s) = caps.structure(0) {
                k.lock().unwrap().push(s.name().to_string());
            }
        });
        p.set_state(gst::State::Paused).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(800));
        let _ = p.set_state(gst::State::Null);
        let guard = kinds.lock().unwrap();
        guard.clone()
    }

    /// Records ~1.5 s (plus a 2 s pre-roll) from a synthetic camera whose audio comes from
    /// `audio_chain` (an encoder + parser ending in the caps the camera would send), into a
    /// `container` file, and returns the file's track kinds and duration.
    fn record_with_audio(
        tag: &str,
        audio_chain: &str,
        container: Container,
    ) -> (Vec<String>, u64, bool, Option<String>) {
        let _ = gst::init();
        let dir = std::env::temp_dir().join(format!("rrv-rec-audio-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        // x264enc shifts its timestamps by 1000 hours (so DTS never goes negative); a real
        // camera gives audio and video one time base, so the synthetic audio is shifted
        // to match (a pad probe on the audio sink).
        let desc = format!(
            "videotestsrc is-live=true \
             ! video/x-raw,format=I420,width=320,height=240,framerate=30/1 \
             ! x264enc tune=zerolatency speed-preset=ultrafast key-int-max=30 \
             ! h264parse \
             ! tee name=enc_tee \
             ! queue \
             ! avdec_h264 \
             ! videoconvert name=converter \
             ! capsfilter name=filter caps=\"video/x-raw,format=RGBA\" \
             ! appsink name=display_sink sync=false emit-signals=true max-buffers=2 drop=true \
             audiotestsrc is-live=true samplesperbuffer=1024 \
             ! {audio_chain} \
             ! appsink name=audio_ring_sink sync=false async=false emit-signals=true"
        );
        let pipeline = gst::parse::launch(&desc)
            .unwrap()
            .downcast::<gst::Pipeline>()
            .unwrap();
        let mut bridge = GStreamerBridge::new(320, 240).unwrap();
        bridge.recording_config.dir = dir.clone();
        bridge.recording_config.container = container;
        bridge.preroll_secs = 2;
        bridge.record_audio = true;
        setup_appsink(&pipeline, &mut bridge).unwrap();
        insert_tee(&pipeline).unwrap();
        insert_ring_branch(&pipeline, &mut bridge).unwrap();
        let audio_sink = pipeline
            .by_name("audio_ring_sink")
            .unwrap()
            .downcast::<gst_app::AppSink>()
            .unwrap();
        audio_sink.static_pad("sink").unwrap().add_probe(
            gst::PadProbeType::BUFFER,
            |_pad, info| {
                if let Some(gst::PadProbeData::Buffer(ref mut b)) = info.data {
                    let buf = b.make_mut();
                    if let Some(pts) = buf.pts() {
                        buf.set_pts(pts + gst::ClockTime::from_seconds(3_600_000));
                    }
                }
                gst::PadProbeReturn::Ok
            },
        );
        attach_audio_sink(&audio_sink, bridge.ring.clone().unwrap());
        bridge.pipeline = Some(pipeline.clone());
        bridge.start_playing().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(3500));

        bridge.start_recording().expect("recording should start");
        std::thread::sleep(std::time::Duration::from_millis(1500));
        bridge.stop_recording_blocking().unwrap();

        let ext = container.extension();
        let segments: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == ext))
            .collect();
        assert_eq!(segments.len(), 1, "{tag}: {segments:?}");
        let kinds = file_track_kinds(&segments[0]);
        let dur = file_duration_ms(&segments[0]);
        let playable = file_is_playable(&segments[0]);
        // An error on the pipeline bus would make the engine treat the camera as dead and
        // reconnect it: an audio format the muxer refuses must never do that.
        let bus_error = pipeline.bus().and_then(|b| {
            b.pop_filtered(&[gst::MessageType::Error])
                .map(|m| match m.view() {
                    gst::MessageView::Error(e) => format!("{} ({:?})", e.error(), e.debug()),
                    _ => "error".into(),
                })
        });
        if std::env::var_os("RRV_KEEP_TEST_FILES").is_some() {
            let _ = std::fs::copy(
                &segments[0],
                std::env::temp_dir().join(format!("rrv-audio-keep-{tag}.{ext}")),
            );
        }
        bridge.stop();
        let _ = std::fs::remove_dir_all(&dir);
        (kinds, dur, playable, bus_error)
    }

    /// Plan 3.1: with `record_audio`, the camera's audio track goes in the same file as
    /// the video, from the same instant (the pre-roll history covers both).
    #[test]
    fn a_recording_with_audio_has_both_tracks_and_the_pre_roll() {
        let _ = gst::init();
        if gst::ElementFactory::find("avenc_aac").is_none() {
            eprintln!("avenc_aac missing: skipping");
            return;
        }
        let (kinds, dur, _, bus_error) = record_with_audio(
            "aac-mkv",
            "audio/x-raw,rate=16000,channels=1 ! avenc_aac ! aacparse",
            Container::Mkv,
        );
        assert_eq!(bus_error, None);
        assert!(kinds.iter().any(|k| k == "video/x-h264"), "{kinds:?}");
        assert!(
            kinds.iter().any(|k| k == "audio/mpeg"),
            "no audio track: {kinds:?}"
        );
        assert!(
            (3_000..=6_000).contains(&dur),
            "1.5 s + pre-roll, got {dur} ms"
        );
    }

    /// The same recording in an mp4 container: video and AAC audio, playable.
    #[test]
    fn aac_audio_also_records_into_mp4() {
        let (kinds, dur, playable, bus_error) = record_with_audio(
            "aac-mp4",
            "audio/x-raw,rate=16000,channels=1 ! avenc_aac ! aacparse",
            Container::Mp4,
        );
        assert!(kinds.iter().any(|k| k == "video/x-h264"), "{kinds:?}");
        assert_eq!(bus_error, None);
        assert!(kinds.iter().any(|k| k == "audio/mpeg"), "{kinds:?}");
        assert!(playable, "the mp4 does not play to EOS");
        assert!((3_000..=6_000).contains(&dur), "got {dur} ms");
    }

    /// IP cameras often send G.711 (A-law in Europe/Brazil, µ-law in the Americas). The mkv
    /// keeps that audio as it is. The mp4 muxer does not take G.711: the recording then has
    /// video only (and says so in the log) but must stay playable and raise no pipeline error.
    #[test]
    fn g711_audio_records_into_mkv_and_degrades_cleanly_in_mp4() {
        for (tag, enc, caps, container, audio_expected) in [
            ("alaw-mkv", "alawenc", "audio/x-alaw", Container::Mkv, true),
            (
                "mulaw-mkv",
                "mulawenc",
                "audio/x-mulaw",
                Container::Mkv,
                true,
            ),
            ("alaw-mp4", "alawenc", "audio/x-alaw", Container::Mp4, false),
            (
                "mulaw-mp4",
                "mulawenc",
                "audio/x-mulaw",
                Container::Mp4,
                false,
            ),
        ] {
            let chain = format!("audio/x-raw,rate=8000,channels=1 ! {enc}");
            let (kinds, _, playable, bus_error) = record_with_audio(tag, &chain, container);
            assert_eq!(
                bus_error, None,
                "{tag}: an audio format the muxer refuses broke the pipeline"
            );
            assert!(
                kinds.iter().any(|k| k == "video/x-h264"),
                "{tag}: {kinds:?}"
            );
            assert_eq!(
                kinds.iter().any(|k| k == caps),
                audio_expected,
                "{tag}: audio track presence: {kinds:?}"
            );
            assert!(playable, "{tag}: does not play to EOS");
        }
    }

    /// Without `record_audio` nothing about the recording changes: video only.
    #[test]
    fn a_recording_without_the_audio_option_stays_video_only() {
        let _ = gst::init();
        let dir = std::env::temp_dir().join(format!("rrv-rec-noaudio-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let desc = "videotestsrc is-live=true \
             ! video/x-raw,format=I420,width=320,height=240,framerate=30/1 \
             ! x264enc tune=zerolatency speed-preset=ultrafast key-int-max=30 \
             ! h264parse ! tee name=enc_tee ! queue ! avdec_h264 \
             ! videoconvert name=converter \
             ! capsfilter name=filter caps=\"video/x-raw,format=RGBA\" \
             ! appsink name=display_sink sync=false emit-signals=true max-buffers=2 drop=true";
        let pipeline = gst::parse::launch(desc)
            .unwrap()
            .downcast::<gst::Pipeline>()
            .unwrap();
        let mut bridge = GStreamerBridge::new(320, 240).unwrap();
        bridge.recording_config.dir = dir.clone();
        bridge.preroll_secs = 1;
        setup_appsink(&pipeline, &mut bridge).unwrap();
        insert_tee(&pipeline).unwrap();
        insert_ring_branch(&pipeline, &mut bridge).unwrap();
        bridge.pipeline = Some(pipeline.clone());
        bridge.start_playing().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2500));
        bridge.start_recording().unwrap();
        assert!(pipeline.by_name("recording_audio_src_1").is_none());
        std::thread::sleep(std::time::Duration::from_millis(1000));
        bridge.stop_recording_blocking().unwrap();
        let f = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .next()
            .unwrap()
            .path();
        let kinds = file_track_kinds(&f);
        assert!(!kinds.iter().any(|k| k.starts_with("audio")), "{kinds:?}");
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
        let pipeline = gst::parse::launch(desc)
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
        let pipeline = gst::parse::launch(desc)
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
        assert!(
            file_is_playable(&segments[0]),
            "segment truncated by early stop()"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
