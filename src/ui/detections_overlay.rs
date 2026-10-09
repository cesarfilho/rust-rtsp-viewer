//! Caixas dos objetos reconhecidos (plano C5), desenhadas sobre o vídeo no spotlight.
//!
//! O daemon manda as caixas normalizadas ao quadro (`WireBox`); o vídeo aparece com *letterbox*, então
//! elas se apoiam no retângulo em que a imagem de fato cai (`zone_editor::fitted_rect`), não no widget
//! inteiro. Sobre vídeo as cores são fixas (como o resto do que se desenha sobre ele): um tom por classe,
//! escolhido numa paleta de alto contraste.

use iced::mouse;
use iced::widget::canvas;
use iced::{Color, Element, Length, Point, Rectangle, Renderer, Size, Theme};

use crate::domain::detect::label_pt;
use crate::ipc::protocol::WireBox;

/// Onde uma caixa cai dentro de `area` (o retângulo do vídeo).
pub fn pixel_rect(b: &WireBox, area: Rectangle) -> Rectangle {
    Rectangle::new(
        Point::new(area.x + b.x * area.width, area.y + b.y * area.height),
        Size::new(b.w * area.width, b.h * area.height),
    )
}

/// "pessoa 86%": o nome em português para as classes comuns e a confiança.
pub fn chip_text(b: &WireBox) -> String {
    format!("{} {:.0}%", label_pt(&b.label), b.score * 100.0)
}

/// A mesma cor do JPEG das detecções (`domain::detection_snapshot::box_colour`): a classe decide, não a
/// ordem em que os objetos aparecem.
pub fn color_for(label: &str) -> Color {
    let [r, g, b] = crate::domain::detection_snapshot::box_colour(label);
    Color::from_rgb8(r, g, b)
}

pub struct DetectionsProgram {
    pub boxes: Vec<WireBox>,
    pub video_width: f32,
    pub video_height: f32,
}

impl<Message> canvas::Program<Message> for DetectionsProgram {
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
        let area =
            super::zone_editor::fitted_rect(bounds.size(), self.video_width, self.video_height);
        for b in &self.boxes {
            let r = pixel_rect(b, area);
            let color = color_for(&b.label);
            frame.stroke(
                &canvas::Path::rectangle(r.position(), r.size()),
                canvas::Stroke::default().with_color(color).with_width(2.0),
            );
            // etiqueta colada ao canto de cima da caixa (dentro dela se não houver espaço acima)
            let text = chip_text(b);
            let chip_w = text.chars().count() as f32 * 7.0 + 8.0;
            let chip_h = 17.0;
            let y = if r.y >= chip_h + 1.0 {
                r.y - chip_h
            } else {
                r.y
            };
            frame.fill_rectangle(Point::new(r.x, y), Size::new(chip_w, chip_h), color);
            frame.fill_text(canvas::Text {
                content: text,
                position: Point::new(r.x + 4.0, y + 2.0),
                color: Color::BLACK,
                size: iced::Pixels(12.0),
                ..canvas::Text::default()
            });
        }
        vec![frame.into_geometry()]
    }
}

/// A camada pronta para empilhar sobre o vídeo; não captura cliques.
pub fn layer<'a, Message: 'a>(
    boxes: Vec<WireBox>,
    video_width: f32,
    video_height: f32,
) -> Element<'a, Message> {
    canvas(DetectionsProgram {
        boxes,
        video_width,
        video_height,
    })
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wb(label: &str, x: f32, y: f32, w: f32, h: f32) -> WireBox {
        WireBox {
            label: label.into(),
            score: 0.867,
            x,
            y,
            w,
            h,
        }
    }

    #[test]
    fn a_box_lands_inside_the_letterboxed_picture_not_the_whole_widget() {
        // vídeo 16:9 num widget quadrado de 800: a imagem ocupa y de 175 a 625
        let area = super::super::zone_editor::fitted_rect(Size::new(800.0, 800.0), 1600.0, 900.0);
        let r = pixel_rect(&wb("person", 0.5, 0.0, 0.25, 0.5), area);
        assert_eq!((r.x, r.width), (400.0, 200.0));
        assert!(
            (r.y - 175.0).abs() < 1e-3 && (r.height - 225.0).abs() < 1e-3,
            "{r:?}"
        );
        // a imagem toda
        let all = pixel_rect(&wb("x", 0.0, 0.0, 1.0, 1.0), area);
        assert_eq!(all, area);
    }

    #[test]
    fn the_chip_names_the_object_in_portuguese_with_the_confidence() {
        assert_eq!(chip_text(&wb("person", 0.0, 0.0, 0.1, 0.1)), "pessoa 87%");
        assert_eq!(chip_text(&wb("kite", 0.0, 0.0, 0.1, 0.1)), "kite 87%");
    }

    #[test]
    fn a_class_always_gets_the_same_colour_and_common_ones_differ() {
        assert_eq!(color_for("person"), color_for("person"));
        let distinct: std::collections::HashSet<_> = ["person", "car", "dog", "bus", "truck"]
            .iter()
            .map(|l| format!("{:?}", color_for(l)))
            .collect();
        assert!(distinct.len() >= 4, "cores demais iguais: {distinct:?}");
    }

    #[test]
    fn the_layer_builds() {
        let _: Element<'_, ()> = layer(vec![wb("person", 0.1, 0.1, 0.2, 0.2)], 1920.0, 1080.0);
    }

    #[test]
    fn the_black_label_text_is_readable_on_every_palette_colour() {
        for [r, g, b] in crate::domain::detection_snapshot::BOX_PALETTE {
            let c =
                super::super::theme::Theme::contrast_ratio(Color::from_rgb8(r, g, b), Color::BLACK);
            assert!(c >= 4.5, "({r},{g},{b}) só tem {c:.1}:1 com texto preto");
        }
    }
}
