# rust-rtsp-viewer — agent quick reference

Low-latency RTSP/HLS viewer with multi-camera grid/flex layout, audio,
snapshots, segmented recording, and an info sidebar. Iced GUI frontend.

## Architecture

- **Stack**: `iced = "0.13"` (features: `image`, `tokio`, `advanced`, `canvas`)
  + `gstreamer = "0.20"` + `gstreamer-app = "0.20"`
- **Display pipeline**:
  `rtspsrc → <decoder> → postdec_queue → videoconvert → capsfilter(RGBA) → tee → display_queue → appsink`
  - The decoder is named `video_decoder` so decode-time probes can find it.
  - `postdec_queue` is a plain `queue` at GStreamer defaults — it only
    decouples threads. **No app-side buffering knobs**: stream caching is
    entirely GStreamer's (`rtspsrc latency`, `uridecodebin`/`queue2`). There is
    no `cache_seconds`.
  - For `uridecodebin3` (HLS/HTTP) the source replaces `rtspsrc → decoder`, and
    the dynamic pad links into `postdec_queue`.
- **Recording branch** — attached to the `tee` **only while recording**:
  `tee → recording_queue → videoconvert → x264enc → splitmuxsink(muxer + filesink)`
  - Built by `GStreamerBridge::start_recording`, removed by `stop_recording`.
  - Never leave it wired up while idle: that burns a core per camera encoding
    frames nobody keeps.
  - `stop_recording` blocks the tee pad, unlinks, pushes EOS through the
    branch, waits for the EOS probe, then sets the elements to Null and
    releases the tee request pad. Skipping the EOS leaves an unplayable file.
  - `splitmuxsink` handles `max_segment_duration_secs` / `max_segment_size_bytes`.
- **Audio**: standalone audio-only pipeline per camera, started on demand by
  `infrastructure::audio::build_audio_pipeline_for_url`.
- **UI**: single `iced::application` with grid or flex layout. Grid density is
  `Auto` (balanced `grid::calc_grid`) or a fixed preset (`2x2`/`3x3`/`4x4`),
  chosen in `domain::view::GridMode`. Fixed presets paginate; the carousel
  (`App.view.rotate_*`) auto-advances pages on the frame tick. Flex shows one
  main + a thumbnail strip. Grid cells are wrapped in a `mouse_area`: left
  click selects, right click opens the context menu.
- **Lazy streaming**: `update::sync_active_streams` keeps only the visible
  page's cameras (+ next page prefetch, + selected) decoding when
  `[view] pause_hidden` is on; the rest are `bridge.stop()`ped and shown
  `Paused`. `update::drain_start_queue` starts pipelines one per
  `[view] stagger_ms` so launch / page flips never open a dozen streams at
  once. `new_app` starts **no** pipelines — it only fills `App.start_queue`.
- **View persistence**: runtime tweaks (density, carousel, camera order,
  group, layout, sidebar) are saved to
  `~/.local/state/rust-rtsp-viewer/view.toml` by `update::persist_view` and
  layered over `[view]` at startup in `new_app`.
- **Tick**: `iced::time::every(100ms)` → `Message::FrameUpdate`, which drives
  frame reads, FPS, bitrate, VU decay, toast expiry, burst capture, timeline
  events, and reconnect checks.

## Source layout

### Domain (`src/domain/` — pure logic, no I/O)

| File | Key types |
|------|-----------|
| `audio.rs` | `AudioConfig`, `AudioState` |
| `bidirectional_audio.rs` | talk-back configuration |
| `codec.rs` | `enum Codec` (H264/H265/Mjpeg/Vp8/…), `from_caps` |
| `diagnostics.rs` | `Severity`, `Hint`, `diagnose`, `overall_severity` |
| `groups.rs` | camera grouping — wired to `[[groups]]` + sidebar/grid filter |
| `hw_encoder.rs` | hardware encoder selection |
| `metrics.rs` | `Metrics` (atomic), `PacketStats`, `StreamInfo` |
| `motion.rs` | frame-difference motion detection |
| `multi_stream.rs` | main/sub stream selection |
| `ptz.rs` | `PtzCommand` |
| `recording.rs` | `RecordingConfig`, `RecordingState`, `generate_filename` |
| `redact.rs` | `mask_credentials` — strip passwords before logging |
| `snapshot.rs` | `SnapshotConfig`, `generate_filename`, `BURST_INTERVAL_MS` |
| `streaming.rs` | re-streaming configuration |
| `timelapse.rs` | timelapse configuration |
| `timeline.rs` | `EventTimeline`, `TimelineEvent`, `EventType` |
| `view.rs` | `GridMode`, `ViewSettings`, pagination/carousel/order math |
| `zones.rs` | `MotionZone`, `Point`, `ZoneConfig` |

### Infrastructure (`src/infrastructure/` — GStreamer, I/O)

| File | Responsibility |
|------|---------------|
| `audio.rs` | `AudioController`, `build_audio_pipeline_for_url`, level bus watch |
| `reconnect.rs` | `ReconnectState` (FPS watchdog + backoff decision) |
| `recording_paths.rs` | directory creation helpers |
| `view_state.rs` | `ViewStateFile` load/save (`~/.local/state/.../view.toml`) |

### UI (`src/ui/` — Iced frontend)

| File | Responsibility |
|------|---------------|
| `app.rs` | `App` struct, `new_app()`, `PendingBurst`, `ViewFocus` (Normal/Immersive/Spotlight) |
| `state.rs` | `Toast`, `BackoffState`, `ContextMenu` |
| `message.rs` | `Message` enum (includes raw `KeyPressed`) |
| `update.rs` | `update()`, `handle_key`, snapshot/record/audio handlers |
| `subscription.rs` | keyboard + resize + 100 ms tick (`TICK_MS`) |
| `bridge.rs` | `GStreamerBridge` — bus, metrics, frame state, recording lifecycle |
| `pipeline.rs` | `start_rtsp`/`start_hls`/`start_file`, recording branch, probes |
| `video_widget.rs` | `iced::widget::image` integration |
| `zone_editor.rs` | zone editor canvas — compiled, **not yet wired to any view** |
| `theme.rs` | themes |
| `grid.rs` | grid layout calculator |
| `sidebar/` | `cameras` (row = pip + name + fps sparkline; controls on hover; `⋯` opens `menu::command_menu`), `info` (Inspector: header + Stream/Rede cards + diagnostics + "Avançado" expander), `diagnostics`, `timeline`; `mod::sparkline` (canvas-free bar chart) |
| `view/` | `mod` (focus-mode composition, pointer tracking, dismiss backdrops), `menu` (`command_menu` — the ONE menu surface, used by the toolbar `⋯` and right-click), `grid_layout`, `flex_layout` (+ `spotlight_view`), `toolbar` (+ `chrome_rail`, `health_meter`, `density_segments`, `overflow_menu_layer`), `cell_overlay` (name chip / status pip / placeholder), `overlays` (`context_menu_layer` renders `menu::command_menu` at `App.pointer_pos`), `style` |

## CLI

```
rust-rtsp-viewer [CONFIG_PATH]      # default: ./config.toml
```

All configuration is in the TOML file. There are no CLI flags and no
environment variables — anything claiming otherwise is stale documentation.

## Config (`config.toml`)

See `config.toml.example` for the annotated reference. Top-level keys
(`latency_ms`, `decoder`, `do_retransmission`) are defaults that each
`[[cameras]]` entry may override. Sections: `[snapshot]`, `[recording]`,
`[audio]`, `[logs]`, `[view]` (grid density/pagination/carousel/lazy-decode/
staggered start — *initial* values; runtime tweaks persist to
`view_state::path()` and win), and `[[groups]]` (named camera groups — `name` +
0-based `cameras` indices — that become sidebar/grid filter chips).

## Build/test

```bash
cargo build           # zero warnings expected
cargo test            # 310 unit tests, all green
```

`cargo test --doc` currently fails on this machine with
`rustdoc: error while loading shared libraries: libLLVM.so...` — that is a
broken local toolchain install, not a code problem.

The recording tests in `ui::pipeline` run real GStreamer pipelines
(`videotestsrc`), write real files, and play them back to EOS to prove the
muxer finalised them. They take ~6s.

System deps: `libgstreamer1.0-dev`, `libgstreamer-plugins-base1.0-dev`,
`gstreamer1.0-plugins-base`, `gstreamer1.0-plugins-good`,
`gstreamer1.0-plugins-bad`, `gstreamer1.0-libav` (Ubuntu 24.04).

## Key bindings

| Key | Action |
|-----|--------|
| `Space` / `k` | Toggle camera selection |
| `s` / `F12` | Take snapshot |
| `r` | Toggle recording |
| `m` | Toggle mute |
| `+` / `-` | Volume up / down |
| `1`–`9` | Select camera |
| `h` | Immersive mode (hide all chrome) |
| `f` / `Enter` / double-click | Spotlight the selected camera |
| `←` / `→` | Prev / next camera (while in spotlight) |
| `Tab` | Cycle layout (grid/flex) |
| `g` | Cycle grid density (Auto/2x2/3x3/4x4) |
| `[` / `]` · `PageUp` / `PageDown` | Previous / next grid page |
| `c` | Toggle page carousel |
| `F2` | Toggle sidebar |
| `F3` | Cycle sidebar tab |
| `F11` | OS fullscreen + immersive |
| `/` | Focus camera search |
| `Esc` | Cascade: spotlight → immersive → menu → search |
| `?` | Show help popup |
| `Ctrl+Q` | Quit |

`ViewFocus` (`app.rs`) drives `view::view`: `Immersive`/`Spotlight` drop the
toolbar + sidebar and paint the video edge-to-edge; a floating reveal rail
(`toolbar::chrome_rail`) appears when `chrome_revealed` (pointer at the top edge,
via `subscription`'s `listen_with`, or any keypress) and auto-hides after
`CHROME_REVEAL_SECS`. The bottom status bar is gone — its data lives in the
toolbar's right cluster and the `⋯` overflow menu.

## Things to remember

- **Never hold a bridge `MutexGuard` across a reconnect.** `std::sync::Mutex`
  is not reentrant, and shadowing the guard (`let bridge = ...` twice in one
  scope) does *not* drop the first one. That deadlocks the UI thread forever.
  `update_frame` scopes the guard in a block for exactly this reason.
- **Keyboard shortcuts are resolved in `update`, not the subscription.** iced
  identifies a subscription by its closure *type*, so a captured
  "search is focused" flag would be frozen at first subscribe. The
  subscription forwards `Message::KeyPressed` and `handle_key` decides.
- **Quitting is behind `Ctrl+Q`.** A bare `q` used to quit even while the user
  was typing in the sidebar search box.
- **Rates must come from deltas.** `bytes_counter` and `frame_count` are
  cumulative; dividing a running total by a short interval is how the bitrate
  readout ended up orders of magnitude too high.
- **Read before you store.** `sample_image_quality_rgba` compares the new luma
  against the *previous* value before overwriting it; storing first made the
  delta always zero and silently disabled tamper detection.
- **Mask credentials before logging.** Pipeline strings contain
  `rtsp://user:pass@host`. Every log site runs them through
  `domain::redact::mask_credentials`.
- **Quote values in `parse_launch` strings.** Use `quote_launch_value` — URLs
  with `&` and paths with spaces break the parser otherwise.
- **Frame buffers are `iced::advanced::image::Bytes`** (refcounted), shared
  between the image `Handle` and the snapshot buffer. Do not go back to
  `Vec<u8>` + `.clone()`: that was two extra 8 MiB copies per 1080p frame.
- **Mutex poisoning**: recover with `unwrap_or_else(|e| e.into_inner())`.
- **HLS/HTTP sources** auto-detect via URL scheme → `uridecodebin3` (handles fMP4/CMAF HLS, not just MPEG-TS).

## Adding new features

- New UI event: add a variant to `Message` in `message.rs`, handle in `update.rs`.
- New sidebar section: extend `sidebar/types.rs` (`SidebarView`) and `sidebar/view.rs`.
- New pipeline element: extend the builders in `pipeline.rs`.
- New diagnostics hint: extend `diagnostics::diagnose` and `overall_severity`.
- New domain module: add to `domain/mod.rs` and keep it pure (no I/O).
  A module that is not declared in a `mod.rs` is never compiled or tested —
  check for orphan files before assuming code is live.
