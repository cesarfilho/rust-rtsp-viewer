use iced::mouse;
use iced::widget::canvas;
use iced::{Color, Element, Length, Point, Rectangle, Renderer, Size, Theme};

use crate::domain::zones::{MotionZone, Point as ZonePoint};

/// Message emitted by the zone editor.
#[derive(Debug, Clone)]
pub enum ZoneEditorMessage {
    /// A vertex was added at normalized coordinates.
    VertexAdded(f64, f64),
    /// The editor was clicked (for finishing a zone).
    #[allow(dead_code)]
    Clicked,
}

/// Visual zone editor overlay for the video area.
#[derive(Debug)]
pub struct ZoneEditorProgram {
    /// Current zones to display.
    pub zones: Vec<MotionZone>,
    /// Whether the editor is in "adding" mode.
    pub adding_mode: bool,
    /// Temporary vertices being placed (not yet committed).
    pub temp_vertices: Vec<ZonePoint>,
    /// Video dimensions (for coordinate conversion).
    #[allow(dead_code)]
    pub video_width: f32,
    #[allow(dead_code)]
    pub video_height: f32,
}

impl Default for ZoneEditorProgram {
    fn default() -> Self {
        Self {
            zones: Vec::new(),
            adding_mode: false,
            temp_vertices: Vec::new(),
            video_width: 640.0,
            video_height: 480.0,
        }
    }
}

impl canvas::Program<ZoneEditorMessage> for ZoneEditorProgram {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());

        // Draw existing zones as semi-transparent filled polygons
        for zone in &self.zones {
            if !zone.enabled || zone.vertices.len() < 3 {
                continue;
            }
            let path = zone_to_path(&zone.vertices, bounds.size());
            frame.fill(&path, Color::from_rgba(0.2, 0.6, 1.0, 0.15));
            frame.stroke(
                &path,
                canvas::Stroke::default()
                    .with_color(Color::from_rgba(0.2, 0.6, 1.0, 0.8))
                    .with_width(2.0),
            );
        }

        // Draw temp vertices being placed
        if !self.temp_vertices.is_empty() {
            if self.temp_vertices.len() >= 3 {
                let path = zone_to_path(&self.temp_vertices, bounds.size());
                frame.fill(&path, Color::from_rgba(1.0, 0.5, 0.0, 0.15));
                frame.stroke(
                    &path,
                    canvas::Stroke::default()
                        .with_color(Color::from_rgba(1.0, 0.5, 0.0, 0.8))
                        .with_width(2.0),
                );
            }
            // Draw vertices as dots
            for v in &self.temp_vertices {
                let pt = normalized_to_pixel(*v, bounds.size());
                let dot = canvas::Path::circle(pt, 4.0);
                frame.fill(&dot, Color::from_rgba(1.0, 0.5, 0.0, 0.9));
            }
        }

        // Draw zone labels
        for zone in &self.zones {
            if !zone.enabled || zone.vertices.is_empty() {
                continue;
            }
            let cx: f64 = zone.vertices.iter().map(|v| v.x).sum::<f64>() / zone.vertices.len() as f64;
            let cy: f64 = zone.vertices.iter().map(|v| v.y).sum::<f64>() / zone.vertices.len() as f64;
            let pt = normalized_to_pixel(ZonePoint::new(cx, cy), bounds.size());
            frame.fill_text(canvas::Text {
                content: zone.name.clone(),
                position: pt,
                color: Color::WHITE,
                size: iced::Pixels(12.0),
                ..Default::default()
            });
        }

        vec![frame.into_geometry()]
    }

    fn update(
        &self,
        _state: &mut (),
        event: canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> (canvas::event::Status, Option<ZoneEditorMessage>) {
        match event {
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                if let Some(cursor_pos) = cursor.position_in(bounds) {
                    let normalized = pixel_to_normalized(cursor_pos, bounds.size());
                    return (
                        canvas::event::Status::Captured,
                        Some(ZoneEditorMessage::VertexAdded(normalized.x, normalized.y)),
                    );
                }
                (canvas::event::Status::Ignored, None)
            }
            _ => (canvas::event::Status::Ignored, None),
        }
    }
}

fn zone_to_path(vertices: &[ZonePoint], size: Size) -> canvas::Path {
    let mut builder = canvas::path::Builder::new();
    if let Some(first) = vertices.first() {
        let pt = normalized_to_pixel(*first, size);
        builder.move_to(pt);
        for v in vertices.iter().skip(1) {
            let pt = normalized_to_pixel(*v, size);
            builder.line_to(pt);
        }
        builder.close();
    }
    builder.build()
}

fn normalized_to_pixel(p: ZonePoint, size: Size) -> Point {
    Point::new(
        p.x as f32 * size.width,
        p.y as f32 * size.height,
    )
}

fn pixel_to_normalized(p: Point, size: Size) -> ZonePoint {
    ZonePoint::new(
        (p.x / size.width).clamp(0.0, 1.0) as f64,
        (p.y / size.height).clamp(0.0, 1.0) as f64,
    )
}

/// Create the zone editor canvas widget as an Element.
///
/// Emits [`ZoneEditorMessage`] rather than the application's own `Message`,
/// so the caller decides how to fold it in (`.map(...)`) and this module
/// stays independent of the app's message enum.
pub fn zone_editor_widget_element(program: &ZoneEditorProgram) -> Element<'_, ZoneEditorMessage> {
    iced::widget::canvas(program)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}
