use std::cell::{Cell, RefCell};
use std::sync::{Arc, Mutex};

use iced::{Element, Length};
use iced::widget::image::Handle;

use crate::ui::bridge::GStreamerBridge;

#[derive(Debug, Clone)]
pub enum Message {
    FrameUpdate,
}

pub struct VideoWidget {
    bridge: Arc<Mutex<GStreamerBridge>>,
    last_gen: Cell<u64>,
    cached_handle: RefCell<Option<Handle>>,
    width: u32,
    height: u32,
}

impl VideoWidget {
    pub fn new(bridge: Arc<Mutex<GStreamerBridge>>) -> Self {
        let (_, w, h, _) = bridge
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .read_frame();
        Self {
            bridge,
            last_gen: Cell::new(0),
            cached_handle: RefCell::new(None),
            width: w,
            height: h,
        }
    }

    pub fn view(&self) -> Element<'static, Message> {
        let (handle_opt, w, h, frame_gen) = self
            .bridge
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .read_frame();

        if frame_gen != self.last_gen.get() {
            self.last_gen.set(frame_gen);
            if let Some(handle) = handle_opt
                && w > 0 && h > 0 {
                    *self.cached_handle.borrow_mut() = Some(handle);
                }
        }

        let handle = self.cached_handle.borrow().clone().unwrap_or_else(|| {
            let size = (self.width * self.height * 4) as usize;
            Handle::from_rgba(self.width, self.height, vec![20u8; size])
        });

        iced::widget::image(handle)
            .width(Length::Fill)
            .height(Length::Fill)
            .content_fit(iced::ContentFit::Contain)
            .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_widget_creation() {
        let bridge = Arc::new(Mutex::new(
            GStreamerBridge::new(640, 480).unwrap(),
        ));
        let widget = VideoWidget::new(bridge);
        assert_eq!(widget.width, 640);
        assert_eq!(widget.height, 480);
    }

    #[test]
    fn video_widget_view_no_frame() {
        let bridge = Arc::new(Mutex::new(
            GStreamerBridge::new(320, 240).unwrap(),
        ));
        let widget = VideoWidget::new(bridge);
        let _ = widget.view();
    }

    #[test]
    fn video_widget_sizes() {
        let bridge = Arc::new(Mutex::new(
            GStreamerBridge::new(1920, 1080).unwrap(),
        ));
        let widget = VideoWidget::new(bridge);
        assert_eq!(widget.width, 1920);
        assert_eq!(widget.height, 1080);
    }
}
