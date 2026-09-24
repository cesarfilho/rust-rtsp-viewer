use super::app::App;
use super::message::Message;

/// Frame tick period. Also the burst-snapshot interval, which is why it
/// matches `domain::snapshot::BURST_INTERVAL_MS`.
pub const TICK_MS: u64 = 100;

pub fn subscription(_state: &App) -> iced::Subscription<Message> {
    let resize = iced::window::resize_events().map(|(_id, size)| Message::WindowResized(size));

    // Key presses are forwarded raw and resolved in `update`, where the
    // current focus state is known. Capturing state in this closure would not
    // work: iced identifies a subscription by the closure's type, so a
    // captured flag would go stale the moment it changed.
    let keyboard = iced::keyboard::on_key_press(|key, modifiers| {
        Some(Message::KeyPressed(key, modifiers))
    });

    let frame_tick =
        iced::time::every(std::time::Duration::from_millis(TICK_MS)).map(|_| Message::FrameUpdate);

    // In immersive / spotlight mode the toolbar is hidden; nudging the pointer
    // to the very top edge brings back a floating reveal rail. Filtering to
    // `y <= 4.0` here keeps this from firing on every cursor move.
    let pointer_top = iced::event::listen_with(|event, _status, _window| match event {
        iced::Event::Mouse(iced::mouse::Event::CursorMoved { position })
            if position.y <= 4.0 =>
        {
            Some(Message::RevealChrome)
        }
        _ => None,
    });

    iced::Subscription::batch(vec![resize, keyboard, frame_tick, pointer_top])
}
