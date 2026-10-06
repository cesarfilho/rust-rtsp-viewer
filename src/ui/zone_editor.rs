use iced::mouse;
use iced::widget::canvas;
use iced::{Color, Element, Length, Point, Rectangle, Renderer, Size, Theme};

use crate::domain::zones::{MotionZone, Point as ZonePoint};

/// Message emitted by the zone editor.
#[derive(Debug, Clone)]
pub enum ZoneEditorMessage {
    /// A vertex was added at normalized coordinates.
    VertexAdded(f64, f64),
    /// The first vertex was clicked again: close the polygon.
    CloseRequested,
}

/// How close (px) a click must land to the first vertex to close the polygon.
const CLOSE_RADIUS: f32 = 10.0;

/// Visual zone editor overlay for the video area.
#[derive(Debug)]
pub struct ZoneEditorProgram {
    /// Current zones to display.
    pub zones: Vec<MotionZone>,
    /// Whether the editor is in "adding" mode.
    pub adding_mode: bool,
    /// Temporary vertices being placed (not yet committed).
    pub temp_vertices: Vec<ZonePoint>,
    /// Video dimensions. The picture is letterboxed (`ContentFit::Contain`),
    /// so normalized zone coordinates map onto the fitted rect, not the
    /// whole canvas.
    pub video_width: f32,
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
        cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let area = fitted_rect(bounds.size(), self.video_width, self.video_height);

        // Draw existing zones as semi-transparent filled polygons
        for zone in &self.zones {
            if !zone.enabled || zone.vertices.len() < 3 {
                continue;
            }
            let path = zone_to_path(&zone.vertices, area);
            frame.fill(&path, Color::from_rgba(0.2, 0.6, 1.0, 0.15));
            frame.stroke(
                &path,
                canvas::Stroke::default()
                    .with_color(Color::from_rgba(0.2, 0.6, 1.0, 0.8))
                    .with_width(2.0),
            );
        }

        // Polygon being drawn: an open polyline that follows the cursor, a
        // fill preview once it has 3+ corners, and a ring on the first corner
        // that lights up when a click there would close the shape.
        if !self.temp_vertices.is_empty() {
            let orange = |a: f32| Color::from_rgba(1.0, 0.5, 0.0, a);
            let pts: Vec<Point> = self
                .temp_vertices
                .iter()
                .map(|v| normalized_to_pixel(*v, area))
                .collect();
            let hover = cursor.position_in(bounds).filter(|p| area.contains(*p));

            if self.temp_vertices.len() >= 3 {
                frame.fill(&zone_to_path(&self.temp_vertices, area), orange(0.15));
            }
            let mut line = canvas::path::Builder::new();
            line.move_to(pts[0]);
            for p in &pts[1..] {
                line.line_to(*p);
            }
            if let Some(h) = hover {
                line.line_to(h);
            }
            frame.stroke(
                &line.build(),
                canvas::Stroke::default()
                    .with_color(orange(0.9))
                    .with_width(2.0),
            );

            let closable = self.temp_vertices.len() >= 3
                && hover.is_some_and(|h| h.distance(pts[0]) <= CLOSE_RADIUS);
            for (i, pt) in pts.iter().enumerate() {
                let r = if i == 0 && closable { 7.0 } else { 4.0 };
                frame.fill(&canvas::Path::circle(*pt, r), orange(0.95));
            }
            if closable {
                frame.stroke(
                    &canvas::Path::circle(pts[0], CLOSE_RADIUS),
                    canvas::Stroke::default()
                        .with_color(Color::WHITE)
                        .with_width(1.5),
                );
            }
        }

        // Draw zone labels
        for zone in &self.zones {
            if !zone.enabled || zone.vertices.is_empty() {
                continue;
            }
            let cx: f64 =
                zone.vertices.iter().map(|v| v.x).sum::<f64>() / zone.vertices.len() as f64;
            let cy: f64 =
                zone.vertices.iter().map(|v| v.y).sum::<f64>() / zone.vertices.len() as f64;
            let pt = normalized_to_pixel(ZonePoint::new(cx, cy), area);
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
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<ZoneEditorMessage>> {
        match event {
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let area = fitted_rect(bounds.size(), self.video_width, self.video_height);
                if let Some(cursor_pos) = cursor.position_in(bounds)
                    && area.contains(cursor_pos)
                {
                    if self.temp_vertices.len() >= 3
                        && cursor_pos.distance(normalized_to_pixel(self.temp_vertices[0], area))
                            <= CLOSE_RADIUS
                    {
                        return Some(
                            canvas::Action::publish(ZoneEditorMessage::CloseRequested)
                                .and_capture(),
                        );
                    }
                    let normalized = pixel_to_normalized(cursor_pos, area);
                    return Some(
                        canvas::Action::publish(ZoneEditorMessage::VertexAdded(
                            normalized.x,
                            normalized.y,
                        ))
                        .and_capture(),
                    );
                }
                None
            }
            _ => None,
        }
    }

    fn mouse_interaction(
        &self,
        _state: &(),
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        let area = fitted_rect(bounds.size(), self.video_width, self.video_height);
        if cursor.position_in(bounds).is_some_and(|p| area.contains(p)) {
            mouse::Interaction::Crosshair
        } else {
            mouse::Interaction::default()
        }
    }
}

/// Where a `video_w × video_h` picture lands inside `size` under `Contain`.
pub(crate) fn fitted_rect(size: Size, video_w: f32, video_h: f32) -> Rectangle {
    if video_w <= 0.0 || video_h <= 0.0 || size.width <= 0.0 || size.height <= 0.0 {
        return Rectangle::new(Point::ORIGIN, size);
    }
    let scale = (size.width / video_w).min(size.height / video_h);
    let (w, h) = (video_w * scale, video_h * scale);
    Rectangle::new(
        Point::new((size.width - w) / 2.0, (size.height - h) / 2.0),
        Size::new(w, h),
    )
}

fn zone_to_path(vertices: &[ZonePoint], size: Rectangle) -> canvas::Path {
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

fn normalized_to_pixel(p: ZonePoint, area: Rectangle) -> Point {
    Point::new(
        area.x + p.x as f32 * area.width,
        area.y + p.y as f32 * area.height,
    )
}

fn pixel_to_normalized(p: Point, area: Rectangle) -> ZonePoint {
    ZonePoint::new(
        ((p.x - area.x) / area.width).clamp(0.0, 1.0) as f64,
        ((p.y - area.y) / area.height).clamp(0.0, 1.0) as f64,
    )
}

/// Create the zone editor canvas widget as an Element.
///
/// Emits [`ZoneEditorMessage`] rather than the application's own `Message`,
/// so the caller decides how to fold it in (`.map(...)`) and this module
/// stays independent of the app's message enum.
pub fn zone_editor_widget_element(
    program: ZoneEditorProgram,
) -> Element<'static, ZoneEditorMessage> {
    iced::widget::canvas(program)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fitted_rect_letterboxes_wide_canvas() {
        // 16:9 video in a square canvas → full width, centred vertically.
        let r = fitted_rect(Size::new(800.0, 800.0), 1600.0, 900.0);
        assert_eq!(r.width, 800.0);
        assert_eq!(r.height, 450.0);
        assert_eq!(r.y, 175.0);
    }

    #[test]
    fn pixel_and_normalized_round_trip_inside_fitted_rect() {
        let area = fitted_rect(Size::new(1000.0, 500.0), 400.0, 400.0);
        let n = pixel_to_normalized(normalized_to_pixel(ZonePoint::new(0.25, 0.75), area), area);
        assert!((n.x - 0.25).abs() < 1e-6 && (n.y - 0.75).abs() < 1e-6);
    }
}
