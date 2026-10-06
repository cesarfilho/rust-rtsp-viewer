use std::sync::{Arc, Mutex};

use iced::{Element, Length};

use crate::ui::bridge::GStreamerBridge;
use crate::ui::video_shader::{self, Frame, VideoProgram};

#[derive(Debug, Clone)]
pub enum Message {
    FrameUpdate,
}

/// Mostra o vídeo de uma `GStreamerBridge` (câmera ao vivo ou gravação). O desenho é do widget
/// `shader` em `video_shader`: uma textura na GPU por vídeo, atualizada no lugar a cada quadro
/// novo (sem um `Handle` de imagem por quadro, que no iced 0.14 faz o vídeo piscar).
pub struct VideoWidget {
    bridge: Arc<Mutex<GStreamerBridge>>,
    /// Identifica a textura deste vídeo na GPU.
    id: u64,
    /// Vive enquanto este widget vive: a GPU libera a textura quando ele morre.
    alive: Arc<()>,
}

impl VideoWidget {
    pub fn new(bridge: Arc<Mutex<GStreamerBridge>>) -> Self {
        Self {
            bridge,
            id: video_shader::next_video_id(),
            alive: video_shader::new_alive_token(),
        }
    }

    pub fn view(&self) -> Element<'static, Message> {
        let (rgba, width, height, generation) = self
            .bridge
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .read_frame();
        let frame = rgba.map(|rgba| Frame {
            rgba,
            width,
            height,
            generation,
        });
        iced::widget::shader(VideoProgram {
            id: self.id,
            frame,
            alive: Arc::downgrade(&self.alive),
        })
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bridge(w: u32, h: u32) -> Arc<Mutex<GStreamerBridge>> {
        Arc::new(Mutex::new(GStreamerBridge::new(w, h).unwrap()))
    }

    #[test]
    fn each_widget_has_its_own_gpu_identity() {
        let a = VideoWidget::new(bridge(640, 480));
        let b = VideoWidget::new(bridge(640, 480));
        assert_ne!(a.id, b.id, "duas câmeras nunca dividem a mesma textura");
    }

    #[test]
    fn the_view_builds_with_and_without_a_frame() {
        // sem quadro ainda: o widget existe e não desenha nada
        let w = VideoWidget::new(bridge(320, 240));
        let _ = w.view();
        let _ = w.view();
    }

    #[test]
    fn the_gpu_texture_is_released_when_the_widget_dies() {
        let w = VideoWidget::new(bridge(320, 240));
        let weak = Arc::downgrade(&w.alive);
        assert!(weak.upgrade().is_some());
        drop(w);
        assert!(
            weak.upgrade().is_none(),
            "o token morre com o widget: a GPU recolhe a textura no próximo quadro"
        );
    }
}
