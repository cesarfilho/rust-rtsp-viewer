//! Reprodução de gravações (plano 3.5): controles de uma `GStreamerBridge` aberta com
//! [`GStreamerBridge::start_file`] — posição, duração, seek, velocidade, pausa e quadro
//! a quadro. O vídeo sai pelo mesmo caminho dos quadros ao vivo (`read_frame`), então a
//! janela mostra uma gravação com o mesmo widget de uma câmera.

use gstreamer as gst;
use gstreamer::prelude::*;

use super::bridge::GStreamerBridge;

/// Velocidades que a janela oferece.
pub const PLAYBACK_RATES: [f64; 5] = [0.5, 1.0, 2.0, 4.0, 8.0];

impl GStreamerBridge {
    /// Posição atual em milissegundos desde o início do arquivo.
    pub fn playback_position_ms(&self) -> Option<u64> {
        let p = self.pipeline.as_ref()?;
        p.query_position::<gst::ClockTime>().map(|t| t.mseconds())
    }

    /// Duração do arquivo em milissegundos (`None` até o arquivo ser lido).
    pub fn playback_duration_ms(&self) -> Option<u64> {
        let p = self.pipeline.as_ref()?;
        p.query_duration::<gst::ClockTime>().map(|t| t.mseconds())
    }

    /// Vai para `ms`, decodificando a partir do keyframe anterior mas mostrando só a
    /// partir do instante pedido (exato).
    pub fn seek_ms(&self, ms: u64) -> Result<(), String> {
        self.seek_with(ms, 1.0)
    }

    /// Muda a velocidade (> 0) a partir da posição atual.
    pub fn set_playback_rate(&self, rate: f64) -> Result<(), String> {
        if !(rate.is_finite() && rate > 0.0) {
            return Err(format!("velocidade inválida: {rate}"));
        }
        let at = self.playback_position_ms().unwrap_or(0);
        self.seek_with(at, rate)
    }

    fn seek_with(&self, ms: u64, rate: f64) -> Result<(), String> {
        let p = self.pipeline.as_ref().ok_or("sem reprodução ativa")?;
        p.seek(
            rate,
            gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
            gst::SeekType::Set,
            gst::ClockTime::from_mseconds(ms),
            gst::SeekType::None,
            gst::ClockTime::NONE,
        )
        .map_err(|e| format!("seek falhou: {e}"))
    }

    /// Pausa (`true`) ou retoma (`false`).
    pub fn set_playback_paused(&self, paused: bool) -> Result<(), String> {
        let p = self.pipeline.as_ref().ok_or("sem reprodução ativa")?;
        p.set_state(if paused {
            gst::State::Paused
        } else {
            gst::State::Playing
        })
        .map(|_| ())
        .map_err(|e| format!("estado: {e:?}"))
    }

    /// Avança um quadro (pausa se estiver tocando).
    pub fn step_frame(&self) -> Result<(), String> {
        let p = self.pipeline.as_ref().ok_or("sem reprodução ativa")?;
        self.set_playback_paused(true)?;
        let ev = gst::event::Step::new(gst::format::Buffers::from_u64(1), 1.0, true, false);
        if p.send_event(ev) {
            Ok(())
        } else {
            Err("o pipeline recusou o passo".into())
        }
    }

    /// `true` quando a posição chegou ao fim do arquivo.
    pub fn playback_ended(&self) -> bool {
        match (self.playback_position_ms(), self.playback_duration_ms()) {
            (Some(pos), Some(dur)) => dur > 0 && pos + 100 >= dur,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// Um mkv de 3 s a 30 fps, feito na hora.
    fn make_clip(name: &str) -> std::path::PathBuf {
        let _ = gst::init();
        let path = std::env::temp_dir().join(format!("rrv-play-{name}-{}.mkv", std::process::id()));
        let desc = format!(
            "videotestsrc num-buffers=90 ! video/x-raw,format=I420,width=320,height=240,framerate=30/1 \
             ! x264enc tune=zerolatency key-int-max=15 ! h264parse ! matroskamux \
             ! filesink location={}",
            crate::infrastructure::launch::quote_launch_value(&path.to_string_lossy())
        );
        let p = gst::parse::launch(&desc).unwrap();
        p.set_state(gst::State::Playing).unwrap();
        let bus = p.bus().unwrap();
        bus.timed_pop_filtered(
            gst::ClockTime::from_seconds(20),
            &[gst::MessageType::Eos, gst::MessageType::Error],
        )
        .expect("o clipe de teste não terminou");
        p.set_state(gst::State::Null).unwrap();
        path
    }

    fn wait_for(mut f: impl FnMut() -> bool) -> bool {
        let end = Instant::now() + Duration::from_secs(10);
        while Instant::now() < end {
            if f() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }

    #[test]
    fn a_recording_plays_seeks_changes_speed_and_steps() {
        let clip = make_clip("controls");
        let mut b = GStreamerBridge::new(320, 240).unwrap();
        b.start_file(&clip.to_string_lossy()).unwrap();

        assert!(
            wait_for(|| b.playback_duration_ms().is_some()),
            "sem duração"
        );
        let dur = b.playback_duration_ms().unwrap();
        assert!((2800..=3300).contains(&dur), "duração {dur} ms");
        assert!(wait_for(|| b.read_frame().3 > 0), "nenhum quadro");

        // seek exato para 2 s: a posição passa a ser >= 2 s
        b.seek_ms(2000).unwrap();
        assert!(
            wait_for(|| b.playback_position_ms().is_some_and(|p| p >= 1900)),
            "posição depois do seek: {:?}",
            b.playback_position_ms()
        );

        // pausa: a posição para
        b.set_playback_paused(true).unwrap();
        std::thread::sleep(Duration::from_millis(300));
        let p1 = b.playback_position_ms().unwrap();
        std::thread::sleep(Duration::from_millis(400));
        assert_eq!(p1, b.playback_position_ms().unwrap(), "andou pausado");

        // um quadro: a geração do quadro sobe e a posição avança ~33 ms
        let gen_before = b.read_frame().3;
        b.step_frame().unwrap();
        assert!(
            wait_for(|| b.read_frame().3 > gen_before),
            "o passo não mostrou quadro"
        );

        // velocidade 4x acaba antes que 1 s de relógio para 1 s de vídeo
        b.seek_ms(0).unwrap();
        b.set_playback_rate(4.0).unwrap();
        b.set_playback_paused(false).unwrap();
        let t = Instant::now();
        assert!(wait_for(|| b.playback_ended()), "não chegou ao fim a 4x");
        assert!(
            t.elapsed() < Duration::from_millis(2500),
            "4x demorou {:?}",
            t.elapsed()
        );

        assert!(b.set_playback_rate(0.0).is_err());
        assert!(b.set_playback_rate(f64::NAN).is_err());
        b.stop();
        let _ = std::fs::remove_file(&clip);
    }

    #[test]
    fn controls_without_a_pipeline_fail_cleanly() {
        let b = GStreamerBridge::new(320, 240).unwrap();
        assert!(b.playback_position_ms().is_none());
        assert!(b.seek_ms(10).is_err());
        assert!(b.step_frame().is_err());
        assert!(!b.playback_ended());
    }
}
