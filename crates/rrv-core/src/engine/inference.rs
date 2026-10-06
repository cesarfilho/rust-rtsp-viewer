//! The inference thread: frames go in, detections come out, and nothing ever waits for the network.
//!
//! The engine (and the window's tick) hand a frame to [`InferenceWorker::submit`] and move on — it
//! never blocks, whatever the model costs. The worker keeps **at most one waiting frame per camera**
//! (a newer one replaces the older: an old picture is worth nothing to a detector) and at most
//! `capacity` in all (the oldest goes first). Whatever is replaced or evicted is counted, so the
//! metrics show when the model cannot keep up.
//!
//! The model is behind [`Infer`], so the queue is tested with a fake that is slow on purpose;
//! `infrastructure::detector::Detector` implements it for the real thing (feature `detect`).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;

use bytes::Bytes;

use crate::domain::detect::Detection;

/// What runs the network. One call per frame, on the worker thread only.
pub trait Infer: Send + 'static {
    fn infer(&mut self, rgba: &[u8], width: u32, height: u32) -> Result<Vec<Detection>, String>;
}

/// A frame to analyse, tagged with the camera it came from.
#[derive(Debug, Clone)]
pub struct InferenceInput {
    pub camera: usize,
    pub rgba: Bytes,
    pub width: u32,
    pub height: u32,
}

/// What came out of one frame.
#[derive(Debug, Clone)]
pub struct InferenceResult {
    pub camera: usize,
    pub detections: Vec<Detection>,
    /// Time the network took.
    pub infer_ms: f32,
    /// Time the frame waited in the queue.
    pub queued_ms: f32,
    /// `Err` text of a failed inference (detections are empty then).
    pub error: Option<String>,
}

/// Counters for the inspector and the logs.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct InferenceStats {
    pub inferred: u64,
    /// Frames replaced or evicted before the network got to them.
    pub dropped: u64,
    pub errors: u64,
    pub queue_len: usize,
    pub last_ms: f32,
    /// Moving average of the network time.
    pub avg_ms: f32,
}

struct Queued {
    input: InferenceInput,
    at: Instant,
}

#[derive(Default)]
struct Shared {
    queue: Mutex<VecDeque<Queued>>,
    wake: Condvar,
    results: Mutex<VecDeque<InferenceResult>>,
    stop: AtomicBool,
    inferred: AtomicU64,
    dropped: AtomicU64,
    errors: AtomicU64,
    /// `f32` bits.
    last_ms: AtomicU64,
    avg_ms: AtomicU64,
}

/// Results kept for the host to collect; a host that never collects does not grow memory.
const MAX_PENDING_RESULTS: usize = 256;

pub struct InferenceWorker {
    shared: Arc<Shared>,
    capacity: usize,
    thread: Option<JoinHandle<()>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl InferenceWorker {
    /// Starts the thread. `capacity` bounds the waiting frames (at least 1).
    pub fn start(mut model: impl Infer, capacity: usize) -> Self {
        let shared = Arc::new(Shared::default());
        let worker = shared.clone();
        let thread = std::thread::Builder::new()
            .name("rrv-inference".into())
            .spawn(move || run(&worker, &mut model))
            .expect("thread de inferência");
        Self {
            shared,
            capacity: capacity.max(1),
            thread: Some(thread),
        }
    }

    /// Queues a frame. Never blocks on the model: a waiting frame of the same camera is replaced,
    /// and when the queue is full the oldest frame is evicted.
    pub fn submit(&self, input: InferenceInput) {
        let mut queue = lock(&self.shared.queue);
        let item = Queued {
            input,
            at: Instant::now(),
        };
        if let Some(slot) = queue
            .iter_mut()
            .find(|q| q.input.camera == item.input.camera)
        {
            *slot = item;
            self.shared.dropped.fetch_add(1, Ordering::Relaxed);
        } else {
            queue.push_back(item);
            if queue.len() > self.capacity {
                let _ = queue.pop_front();
                self.shared.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
        drop(queue);
        self.shared.wake.notify_one();
    }

    /// Everything finished since the last call, oldest first.
    pub fn take_results(&self) -> Vec<InferenceResult> {
        lock(&self.shared.results).drain(..).collect()
    }

    pub fn stats(&self) -> InferenceStats {
        let s = &self.shared;
        InferenceStats {
            inferred: s.inferred.load(Ordering::Relaxed),
            dropped: s.dropped.load(Ordering::Relaxed),
            errors: s.errors.load(Ordering::Relaxed),
            queue_len: lock(&s.queue).len(),
            last_ms: f32::from_bits(s.last_ms.load(Ordering::Relaxed) as u32),
            avg_ms: f32::from_bits(s.avg_ms.load(Ordering::Relaxed) as u32),
        }
    }
}

impl Drop for InferenceWorker {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        self.shared.wake.notify_all();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn run(shared: &Shared, model: &mut impl Infer) {
    loop {
        let job = {
            let mut queue = lock(&shared.queue);
            loop {
                if shared.stop.load(Ordering::Relaxed) {
                    return;
                }
                if let Some(job) = queue.pop_front() {
                    break job;
                }
                queue = shared.wake.wait(queue).unwrap_or_else(|e| e.into_inner());
            }
        };
        let queued_ms = job.at.elapsed().as_secs_f32() * 1000.0;
        let started = Instant::now();
        let outcome = model.infer(&job.input.rgba, job.input.width, job.input.height);
        let infer_ms = started.elapsed().as_secs_f32() * 1000.0;

        let (detections, error) = match outcome {
            Ok(d) => {
                shared.inferred.fetch_add(1, Ordering::Relaxed);
                (d, None)
            }
            Err(e) => {
                shared.errors.fetch_add(1, Ordering::Relaxed);
                (Vec::new(), Some(e))
            }
        };
        shared
            .last_ms
            .store(u64::from(infer_ms.to_bits()), Ordering::Relaxed);
        let prev = f32::from_bits(shared.avg_ms.load(Ordering::Relaxed) as u32);
        let avg = if prev == 0.0 {
            infer_ms
        } else {
            prev * 0.8 + infer_ms * 0.2
        };
        shared
            .avg_ms
            .store(u64::from(avg.to_bits()), Ordering::Relaxed);

        let mut results = lock(&shared.results);
        results.push_back(InferenceResult {
            camera: job.input.camera,
            detections,
            infer_ms,
            queued_ms,
            error,
        });
        while results.len() > MAX_PENDING_RESULTS {
            let _ = results.pop_front();
        }
    }
}

#[cfg(feature = "detect")]
impl Infer for crate::infrastructure::detector::Detector {
    fn infer(&mut self, rgba: &[u8], width: u32, height: u32) -> Result<Vec<Detection>, String> {
        // thresholds fixed here until the config (C4) carries them
        self.detect_rgba(rgba, width, height, 0.25, 0.45)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// A model that takes `delay` per frame and answers with one box whose score is the first byte.
    struct Slow {
        delay: Duration,
        fail_on: Option<u8>,
    }

    impl Infer for Slow {
        fn infer(&mut self, rgba: &[u8], _w: u32, _h: u32) -> Result<Vec<Detection>, String> {
            std::thread::sleep(self.delay);
            if self.fail_on == Some(rgba[0]) {
                return Err("modelo falhou".into());
            }
            Ok(vec![Detection {
                class: 0,
                score: f32::from(rgba[0]) / 255.0,
                x: 0.0,
                y: 0.0,
                w: 1.0,
                h: 1.0,
            }])
        }
    }

    fn frame(camera: usize, tag: u8) -> InferenceInput {
        InferenceInput {
            camera,
            rgba: Bytes::from(vec![tag; 16]),
            width: 2,
            height: 2,
        }
    }

    fn wait_until(mut cond: impl FnMut() -> bool) -> bool {
        let end = Instant::now() + Duration::from_secs(5);
        while Instant::now() < end {
            if cond() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    }

    fn slow(ms: u64) -> Slow {
        Slow {
            delay: Duration::from_millis(ms),
            fail_on: None,
        }
    }

    #[test]
    fn submitting_never_waits_for_a_slow_model() {
        let w = InferenceWorker::start(slow(200), 4);
        let t = Instant::now();
        for i in 0..200 {
            w.submit(frame(i % 8, 1));
        }
        assert!(
            t.elapsed() < Duration::from_millis(100),
            "200 envios levaram {:?} com um modelo de 200 ms",
            t.elapsed()
        );
        assert!(w.stats().queue_len <= 4, "a fila respeita o limite");
        assert!(w.stats().dropped > 0);
    }

    #[test]
    fn a_newer_frame_of_the_same_camera_replaces_the_waiting_one() {
        let w = InferenceWorker::start(slow(100), 8);
        w.submit(frame(0, 10)); // o worker pega este e fica ocupado
        std::thread::sleep(Duration::from_millis(20));
        w.submit(frame(1, 20));
        w.submit(frame(1, 30)); // substitui o 20
        w.submit(frame(1, 40)); // substitui o 30
        assert!(wait_until(|| w.stats().inferred >= 2));
        std::thread::sleep(Duration::from_millis(150));
        let tags: Vec<u8> = w
            .take_results()
            .iter()
            .map(|r| (r.detections[0].score * 255.0).round() as u8)
            .collect();
        assert_eq!(
            tags,
            vec![10, 40],
            "só o mais novo da câmera 1 foi analisado"
        );
        assert_eq!(w.stats().dropped, 2);
    }

    #[test]
    fn when_full_the_oldest_camera_is_evicted() {
        let w = InferenceWorker::start(slow(100), 2);
        w.submit(frame(0, 1)); // ocupa o worker
        std::thread::sleep(Duration::from_millis(20));
        w.submit(frame(1, 2));
        w.submit(frame(2, 3));
        w.submit(frame(3, 4)); // cheia: sai a câmera 1
        assert!(wait_until(|| w.stats().inferred >= 3));
        std::thread::sleep(Duration::from_millis(120));
        let cams: Vec<usize> = w.take_results().iter().map(|r| r.camera).collect();
        assert_eq!(cams, vec![0, 2, 3]);
        assert_eq!(w.stats().dropped, 1);
    }

    #[test]
    fn results_carry_timings_and_stats_follow() {
        let w = InferenceWorker::start(slow(30), 4);
        w.submit(frame(5, 99));
        assert!(wait_until(|| w.stats().inferred == 1));
        let r = &w.take_results()[0];
        assert_eq!(r.camera, 5);
        assert!(r.infer_ms >= 25.0, "{}", r.infer_ms);
        assert!(r.error.is_none());
        let s = w.stats();
        assert!(s.last_ms >= 25.0 && s.avg_ms >= 25.0, "{s:?}");
        assert_eq!((s.dropped, s.errors, s.queue_len), (0, 0, 0));
        assert!(w.take_results().is_empty(), "já coletado");
    }

    #[test]
    fn a_failing_inference_is_reported_and_the_worker_goes_on() {
        let w = InferenceWorker::start(
            Slow {
                delay: Duration::ZERO,
                fail_on: Some(7),
            },
            4,
        );
        w.submit(frame(0, 7));
        assert!(wait_until(|| w.stats().errors == 1));
        w.submit(frame(0, 8));
        assert!(wait_until(|| w.stats().inferred == 1));
        let r = w.take_results();
        assert_eq!(r[0].error.as_deref(), Some("modelo falhou"));
        assert!(r[0].detections.is_empty());
        assert!(r[1].error.is_none());
    }

    #[test]
    fn dropping_the_worker_stops_the_thread_even_mid_queue() {
        let w = InferenceWorker::start(slow(50), 4);
        for i in 0..4 {
            w.submit(frame(i, 1));
        }
        let t = Instant::now();
        drop(w);
        assert!(
            t.elapsed() < Duration::from_secs(1),
            "o encerramento esperou a fila toda: {:?}",
            t.elapsed()
        );
    }

    #[test]
    fn unread_results_do_not_grow_without_bound() {
        let w = InferenceWorker::start(slow(0), 1);
        for i in 0..(MAX_PENDING_RESULTS + 100) {
            w.submit(frame(0, 1));
            if i % 50 == 0 {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        assert!(wait_until(|| w.stats().queue_len == 0));
        std::thread::sleep(Duration::from_millis(30));
        assert!(w.take_results().len() <= MAX_PENDING_RESULTS);
    }
}
