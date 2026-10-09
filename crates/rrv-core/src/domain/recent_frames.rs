//! The last few seconds of a camera's full-size pictures, by timestamp, so a detection snapshot can
//! use the picture the model actually saw. The model answers a second or two after its frame was
//! taken; by then a walking person has moved and the box drawn on the *newest* picture lands on
//! empty pavement, which reads as a false positive.
//!
//! Generic over what is kept (the engine keeps `gst::Sample` references, so nothing is copied).
//! Pure: timestamps are the stream's own (nanoseconds), passed in.

use std::collections::VecDeque;

/// How far back pictures are kept. The model's answer arrives up to ~1.5 s after its frame
/// (a slow tick to submit, the inference, a slow tick to collect), so 4 s leaves room.
pub const KEEP_NS: u64 = 4_000_000_000;
/// One picture per 200 ms is close enough (a person moves a few pixels) and bounds memory:
/// 20 references per camera.
pub const SPACING_NS: u64 = 200_000_000;

#[derive(Debug, Clone)]
pub struct RecentFrames<T> {
    frames: VecDeque<(u64, T)>,
}

impl<T> Default for RecentFrames<T> {
    fn default() -> Self {
        Self {
            frames: VecDeque::new(),
        }
    }
}

impl<T: Clone> RecentFrames<T> {
    /// Keeps `item` taken at `pts` if it is at least [`SPACING_NS`] after the last one kept, and
    /// forgets what is older than [`KEEP_NS`]. A timestamp going backwards (the stream restarted)
    /// starts over.
    pub fn push(&mut self, pts: u64, item: T) {
        if let Some(&(last, _)) = self.frames.back() {
            if pts < last {
                self.frames.clear();
            } else if pts - last < SPACING_NS {
                return;
            }
        }
        self.frames.push_back((pts, item));
        while self
            .frames
            .front()
            .is_some_and(|&(t, _)| pts.saturating_sub(t) > KEEP_NS)
        {
            self.frames.pop_front();
        }
    }

    /// The picture closest to `pts`; the newest when `pts` is unknown.
    pub fn nearest(&self, pts: Option<u64>) -> Option<T> {
        let Some(pts) = pts else {
            return self.frames.back().map(|(_, f)| f.clone());
        };
        self.frames
            .iter()
            .min_by_key(|(t, _)| t.abs_diff(pts))
            .map(|(_, f)| f.clone())
    }

    pub fn clear(&mut self) {
        self.frames.clear();
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: u64 = 1_000_000;

    #[test]
    fn the_picture_closest_to_the_models_frame_is_picked() {
        let mut r = RecentFrames::default();
        // 20 fps durante 3 s: só um a cada 200 ms fica
        for i in 0..60u64 {
            r.push(i * 50 * MS, i);
        }
        assert_eq!(r.len(), 15);
        assert_eq!(r.nearest(Some(1_010 * MS)), Some(20), "1,0 s");
        assert_eq!(r.nearest(Some(2_390 * MS)), Some(48), "2,4 s");
        assert_eq!(r.nearest(None), Some(56), "sem tempo: o mais novo");
    }

    #[test]
    fn old_pictures_are_dropped_and_a_restart_starts_over() {
        let mut r = RecentFrames::default();
        for s in 0..10u64 {
            r.push(s * 1_000 * MS, s);
        }
        assert_eq!(r.nearest(Some(0)), Some(5), "só os últimos 4 s");
        r.push(0, 99);
        assert_eq!((r.len(), r.nearest(None)), (1, Some(99)));
        r.clear();
        assert!(r.is_empty() && r.nearest(None).is_none());
    }
}
