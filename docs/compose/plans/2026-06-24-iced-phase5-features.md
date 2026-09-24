# Phase 5: Quick Actions, Audio Indicator, Keyboard Shortcuts

> **For agentic workers:** Use compose:subagent or compose:execute to implement task-by-task.

**Goal:** Add quick actions bar, audio visual indicator, keyboard shortcuts, and status bar.

**Architecture:** Quick actions bar appears when camera selected. Audio glow applied via container style. Keyboard events handled via subscription.

**Tech Stack:** Iced 0.13, existing theme system

## Tasks

### Task 1: Quick actions bar

**Files:**
- Modify: `src/iced_app/mod.rs`

**Steps:**
1. Add `quick_actions_visible: bool` to `App`
2. Add `Message::Snapshot`, `Message::ToggleRecording`, `Message::ToggleAudio` variants
3. Build quick actions bar with 3 buttons
4. Show when camera selected, hide otherwise
5. Style with theme colors

### Task 2: Audio glow indicator

**Files:**
- Modify: `src/iced_app/mod.rs`

**Steps:**
1. Add `audio_active: Vec<bool>` to track which cameras have audio
2. Add `Message::AudioToggled(usize)` handler
3. Apply green glow (`box-shadow`) to cells with active audio
4. Add ♪ badge overlay on audio-active cells

### Task 3: Keyboard shortcuts

**Files:**
- Modify: `src/iced_app/mod.rs`

**Steps:**
1. Add `iced::keyboard::on_key_press` subscription
2. Handle: Space (select/deselect), s/F12 (snapshot), r (record), m (mute), 1-9 (select camera)
3. Map shortcuts to existing messages

### Task 4: Status bar

**Files:**
- Modify: `src/iced_app/mod.rs`

**Steps:**
1. Add status bar at bottom of window
2. Show: camera count, online count, uptime, key hints
3. Style with theme colors

### Task 5: Integration test

**Steps:**
1. Build and verify
2. Run all tests
3. Commit
