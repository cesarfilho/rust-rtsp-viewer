use serde::Deserialize;

/// Configuration for smart streaming — pausing decode on
/// static scenes to save CPU.
#[derive(Debug, Clone)]
pub struct SmartStreamingConfig {
    /// Whether smart streaming is enabled.
    pub enabled: bool,
    /// How many seconds the scene must be static before
    /// we pause decoding. Range 5..=3600.
    pub pause_after_secs: u32,
    /// How many frames to keep decoding after motion
    /// resumes (to let the scene stabilise). Range 1..=30.
    pub warmup_frames: u32,
}

impl Default for SmartStreamingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            pause_after_secs: 30,
            warmup_frames: 5,
        }
    }
}

/// TOML mirror for `[smart_streaming]` section.
#[derive(Debug, Deserialize, Clone, Default)]
pub struct SmartStreamingConfigFile {
    pub enabled: Option<bool>,
    pub pause_after_secs: Option<u32>,
    pub warmup_frames: Option<u32>,
}

impl SmartStreamingConfigFile {
    pub fn into_config(self) -> SmartStreamingConfig {
        let mut config = SmartStreamingConfig::default();
        if let Some(e) = self.enabled {
            config.enabled = e;
        }
        if let Some(p) = self.pause_after_secs {
            config.pause_after_secs = p.clamp(5, 3600);
        }
        if let Some(w) = self.warmup_frames {
            config.warmup_frames = w.clamp(1, 30);
        }
        config
    }
}

/// Runtime state for smart streaming, tracked per-camera.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamingDecision {
    /// Keep decoding normally.
    Continue,
    /// Pause the decoder (scene is static).
    PauseDecoder,
    /// Resume the decoder (motion just started or warmup in progress).
    ResumeDecoder,
}

/// Track whether we should pause or resume decoding.
///
/// Call this every tick (250 ms) with the current `motion_active`
/// flag. Returns a `StreamingDecision` the pipeline can act on.
pub fn evaluate_streaming(
    state: &mut StreamingState,
    motion_active: bool,
    config: &SmartStreamingConfig,
    frames_since_motion: u32,
) -> StreamingDecision {
    if !config.enabled {
        return StreamingDecision::Continue;
    }

    match state.phase {
        Phase::Decoding => {
            if motion_active {
                // Scene is active — keep decoding, reset timer.
                state.static_secs = 0;
                StreamingDecision::Continue
            } else {
                state.static_secs += 1;
                if state.static_secs >= config.pause_after_secs {
                    state.phase = Phase::Paused;
                    StreamingDecision::PauseDecoder
                } else {
                    StreamingDecision::Continue
                }
            }
        }
        Phase::Paused => {
            if motion_active {
                state.phase = Phase::WarmingUp;
                state.warmup_remaining = config.warmup_frames;
                StreamingDecision::ResumeDecoder
            } else {
                StreamingDecision::PauseDecoder
            }
        }
        Phase::WarmingUp => {
            if frames_since_motion < config.warmup_frames {
                // Still in warmup — keep decoding.
                StreamingDecision::ResumeDecoder
            } else {
                // Warmup done — switch to normal decoding.
                state.phase = Phase::Decoding;
                state.static_secs = 0;
                StreamingDecision::Continue
            }
        }
    }
}

/// Current phase of the smart streaming state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Normal decoding — scene is active or timer hasn't expired.
    Decoding,
    /// Decoder is paused — waiting for motion to resume.
    Paused,
    /// Motion detected — keep decoding for a few frames to
    /// let the scene stabilise before normal operation.
    WarmingUp,
}

/// Persistent state for smart streaming, tracked per-camera.
#[derive(Debug, Clone)]
pub struct StreamingState {
    phase: Phase,
    /// Seconds the scene has been static (reset on motion).
    static_secs: u32,
    /// Frames remaining in the warmup phase.
    warmup_remaining: u32,
}

impl Default for StreamingState {
    fn default() -> Self {
        Self {
            phase: Phase::Decoding,
            static_secs: 0,
            warmup_remaining: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(pause: u32, warmup: u32) -> SmartStreamingConfig {
        SmartStreamingConfig {
            enabled: true,
            pause_after_secs: pause,
            warmup_frames: warmup,
        }
    }

    #[test]
    fn disabled_config_always_continues() {
        let mut state = StreamingState::default();
        let config = SmartStreamingConfig {
            enabled: false,
            ..Default::default()
        };
        assert_eq!(
            evaluate_streaming(&mut state, true, &config, 0),
            StreamingDecision::Continue
        );
        assert_eq!(
            evaluate_streaming(&mut state, false, &config, 0),
            StreamingDecision::Continue
        );
    }

    #[test]
    fn motion_resets_static_timer() {
        let mut state = StreamingState::default();
        let config = cfg(10, 5);
        // Static for 5 ticks
        for _ in 0..5 {
            evaluate_streaming(&mut state, false, &config, 0);
        }
        // Motion resets timer
        evaluate_streaming(&mut state, true, &config, 0);
        // Should still be decoding (timer reset)
        for _ in 0..9 {
            assert_eq!(
                evaluate_streaming(&mut state, false, &config, 0),
                StreamingDecision::Continue
            );
        }
    }

    #[test]
    fn pauses_after_timeout() {
        let mut state = StreamingState::default();
        let config = cfg(3, 5);
        for _ in 0..2 {
            assert_eq!(
                evaluate_streaming(&mut state, false, &config, 0),
                StreamingDecision::Continue
            );
        }
        assert_eq!(
            evaluate_streaming(&mut state, false, &config, 0),
            StreamingDecision::PauseDecoder
        );
    }

    #[test]
    fn motion_resumes_from_paused() {
        let mut state = StreamingState::default();
        let config = cfg(2, 3);
        // Pause
        evaluate_streaming(&mut state, false, &config, 0);
        evaluate_streaming(&mut state, false, &config, 0);
        assert_eq!(state.phase, Phase::Paused);
        // Motion resumes
        assert_eq!(
            evaluate_streaming(&mut state, true, &config, 0),
            StreamingDecision::ResumeDecoder
        );
        assert_eq!(state.phase, Phase::WarmingUp);
    }

    #[test]
    fn warmup_transitions_to_decoding() {
        let mut state = StreamingState::default();
        let config = cfg(2, 3);
        // Pause then resume
        evaluate_streaming(&mut state, false, &config, 0);
        evaluate_streaming(&mut state, false, &config, 0);
        evaluate_streaming(&mut state, true, &config, 0);
        // Warmup: frames 0, 1 still warming
        assert_eq!(
            evaluate_streaming(&mut state, false, &config, 0),
            StreamingDecision::ResumeDecoder
        );
        assert_eq!(
            evaluate_streaming(&mut state, false, &config, 1),
            StreamingDecision::ResumeDecoder
        );
        // Frame 3 = warmup done → Continue
        assert_eq!(
            evaluate_streaming(&mut state, false, &config, 3),
            StreamingDecision::Continue
        );
    }

    #[test]
    fn paused_without_motion_stays_paused() {
        let mut state = StreamingState::default();
        let config = cfg(1, 5);
        evaluate_streaming(&mut state, false, &config, 0);
        assert_eq!(state.phase, Phase::Paused);
        assert_eq!(
            evaluate_streaming(&mut state, false, &config, 0),
            StreamingDecision::PauseDecoder
        );
    }

    #[test]
    fn config_file_clamps_values() {
        let f = SmartStreamingConfigFile {
            enabled: Some(true),
            pause_after_secs: Some(0), // below min
            warmup_frames: Some(100),  // above max
        };
        let c = f.into_config();
        assert_eq!(c.pause_after_secs, 5);
        assert_eq!(c.warmup_frames, 30);
    }
}
