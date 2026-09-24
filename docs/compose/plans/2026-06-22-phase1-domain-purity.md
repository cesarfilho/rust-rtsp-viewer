# Phase 1: Domain Purity — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use compose:subagent (recommended) or compose:execute to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove all I/O and infrastructure dependencies from the domain layer, making it truly pure (no std::fs, no SystemTime, no config::CameraConfig).

**Architecture:** Extract I/O operations to infrastructure, replace infrastructure types with domain value objects, add tests for pure domain logic.

**Tech Stack:** Rust 2024, `gstreamer 0.20`, `gtk 0.18`, `glib 0.18`

---

### Task 1: Extract I/O from domain/recording.rs

**Covers:** [S3.1]

**Files:**
- Modify: `src/domain/recording.rs`
- Create: `src/infrastructure/recording_paths.rs`

- [ ] **Step 1: Create infrastructure module for recording paths**

```rust
// src/infrastructure/recording_paths.rs
use std::path::PathBuf;

pub fn ensure_recording_dir(dir: &str) -> Result<PathBuf, String> {
    let path = dirs_or_default(dir);
    std::fs::create_dir_all(&path)
        .map_err(|e| format!("Failed to create recording dir: {}", e))?;
    Ok(path)
}

fn dirs_or_default(dir: &str) -> PathBuf {
    if dir.starts_with("~/") {
        if let Some(home) = std::env::var("HOME").ok() {
            return PathBuf::from(home).join(&dir[2..]);
        }
    }
    PathBuf::from(dir)
}
```

- [ ] **Step 2: Wire into infrastructure/mod.rs**

Add `pub mod recording_paths;` to `src/infrastructure/mod.rs`.

- [ ] **Step 3: Update domain/recording.rs to remove I/O**

Remove `ensure_dir_exists()` function from `domain/recording.rs`. Remove `use std::fs;` and `use std::env;` imports.

The `generate_filename()` function should remain in domain but take a `base_dir: &Path` parameter instead of calling `std::env::var("HOME")`.

- [ ] **Step 4: Update callers to use infrastructure**

In `infrastructure/recording.rs`, replace calls to `domain::recording::ensure_dir_exists` with `infrastructure::recording_paths::ensure_recording_dir`.

- [ ] **Step 5: Verify compilation**

Run: `cargo build 2>&1 | head -20`
Expected: Compiles with no errors.

- [ ] **Step 6: Run tests**

Run: `cargo test 2>&1 | grep "test result"`
Expected: 282 passed.

- [ ] **Step 7: Commit**

```bash
git add src/domain/recording.rs src/infrastructure/recording_paths.rs src/infrastructure/mod.rs src/infrastructure/recording.rs
git commit -m "refactor(domain): extract I/O from recording.rs to infrastructure"
```

---

### Task 2: Replace config dependency in domain/source.rs

**Covers:** [S3.2]

**Files:**
- Modify: `src/domain/source.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Create SourceDescriptor value object**

```rust
// Add to src/domain/source.rs (or create src/domain/source_descriptor.rs)

#[derive(Debug, Clone)]
pub struct SourceDescriptor {
    pub kind: SourceKind,
    pub url: String,
    pub latency_ms: u32,
    pub cache_seconds: u32,
    pub decoder: String,
    pub use_uridecodebin: bool,
    pub do_retransmission: bool,
}

impl SourceDescriptor {
    pub fn from_url(url: &str) -> Self {
        let kind = if url.starts_with("http://") || url.starts_with("https://") {
            SourceKind::Hls
        } else if url.starts_with("rtsp://") || url.starts_with("rtsps://") {
            SourceKind::Rtsp
        } else {
            SourceKind::File
        };
        SourceDescriptor {
            kind,
            url: url.to_string(),
            latency_ms: 100,
            cache_seconds: 0,
            decoder: "decodebin".to_string(),
            use_uridecodebin: false,
            do_retransmission: true,
        }
    }
}
```

- [ ] **Step 2: Remove `use crate::config::CameraConfig` from domain/source.rs**

Replace the import with the new `SourceDescriptor`.

- [ ] **Step 3: Update SourceConfig to use SourceDescriptor**

Change `SourceConfig::from_camera_config` to `SourceConfig::from_descriptor`.

- [ ] **Step 4: Update callers in main.rs and infrastructure**

In `main.rs`, convert `CameraConfig` → `SourceDescriptor` before passing to domain.

- [ ] **Step 5: Verify compilation**

Run: `cargo build 2>&1 | head -20`

- [ ] **Step 6: Run tests**

Run: `cargo test 2>&1 | grep "test result"`

- [ ] **Step 7: Commit**

```bash
git add src/domain/source.rs src/main.rs
git commit -m "refactor(domain): replace config dependency with SourceDescriptor value object"
```

---

### Task 3: Add domain purity tests

**Covers:** [S3.3]

**Files:**
- Modify: `src/domain/recording.rs` (add tests)
- Modify: `src/domain/source.rs` (add tests)

- [ ] **Step 1: Add tests for recording domain logic**

```rust
// Add to src/domain/recording.rs #[cfg(test)] mod tests

#[test]
fn test_format_duration_seconds() {
    assert_eq!(format_duration(45), "00:00:45");
}

#[test]
fn test_format_duration_minutes() {
    assert_eq!(format_duration(125), "00:02:05");
}

#[test]
fn test_format_duration_hours() {
    assert_eq!(format_duration(3661), "01:01:01");
}

#[test]
fn test_recording_config_defaults() {
    let cfg = RecordingConfig::default();
    assert_eq!(cfg.max_segment_duration_secs, 600);
    assert_eq!(cfg.container, Container::Mkv);
}
```

- [ ] **Step 2: Add tests for SourceDescriptor**

```rust
// Add to src/domain/source.rs #[cfg(test)] mod tests

#[test]
fn test_source_kind_rtsp() {
    let desc = SourceDescriptor::from_url("rtsp://192.168.1.100:554/stream");
    assert_eq!(desc.kind, SourceKind::Rtsp);
}

#[test]
fn test_source_kind_hls() {
    let desc = SourceDescriptor::from_url("https://example.com/stream.m3u8");
    assert_eq!(desc.kind, SourceKind::Hls);
}

#[test]
fn test_source_kind_file() {
    let desc = SourceDescriptor::from_url("/tmp/test.mp4");
    assert_eq!(desc.kind, SourceKind::File);
}

#[test]
fn test_source_descriptor_defaults() {
    let desc = SourceDescriptor::from_url("rtsp://example.com/stream");
    assert_eq!(desc.latency_ms, 100);
    assert_eq!(desc.cache_seconds, 0);
    assert!(desc.do_retransmission);
}
```

- [ ] **Step 3: Run tests to verify they pass**

Run: `cargo test 2>&1 | grep "test result"`
Expected: 282+ passed (new tests added).

- [ ] **Step 4: Commit**

```bash
git add src/domain/recording.rs src/domain/source.rs
git commit -m "test(domain): add purity tests for recording and source modules"
```

---

### Task 4: Final verification

**Covers:** [S7]

**Files:** None (verification only)

- [ ] **Step 1: Full build**

Run: `cargo build 2>&1`
Expected: No errors.

- [ ] **Step 2: Full test suite**

Run: `cargo test 2>&1`
Expected: All tests pass (282+ including new ones).

- [ ] **Step 3: Verify domain purity**

Run: `grep -r "std::fs\|std::env\|SystemTime" src/domain/`
Expected: No matches (domain is pure).

- [ ] **Step 4: Commit if any cleanup needed**

```bash
git add -u
git commit -m "refactor(domain): complete domain purity phase"
```

---

## Summary

| Task | What | Lines changed | Risk |
|------|------|:-------------:|:----:|
| 1 | Extract I/O from recording.rs | ~30 | Low |
| 2 | Replace config dependency | ~40 | Medium |
| 3 | Add domain purity tests | ~50 | Low |
| 4 | Final verification | 0 | None |

**Net result:** Domain layer becomes truly pure — no I/O, no infrastructure dependencies, fully testable.
