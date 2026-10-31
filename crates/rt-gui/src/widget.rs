//! Button widget: a pressable labeled rect rendered with embedded-graphics
//! primitives. State changes mark damage; a click is reported as an outcome
//! (the seam where a core Signal emission plugs in once ticket #5 lands).

use embedded_graphics::geometry::Point;
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::mono_font::ascii::FONT_8X13;
use embedded_graphics::pixelcolor::Rgb888;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
use embedded_graphics::text::Text;

use crate::input::InputEvent;

/// Result of feeding one [`InputEvent`] to a widget.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ButtonOutcome {
    /// Region needing repaint, if the visual state changed.
    pub damage: Option<Rectangle>,
    /// Press followed by release inside the button.
    pub clicked: bool,
}

impl ButtonOutcome {
    fn clean() -> Self {
        Self::default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Visual {
    Normal,
    Hovered,
    Pressed,
}

/// A labeled push button occupying a fixed rectangle (physical pixels).
#[derive(Debug)]
pub struct Button {
    rect: Rectangle,
    label: String,
    visual: Visual,
}

impl Button {
    pub fn new(rect: Rectangle, label: impl Into<String>) -> Self {
        Self {
            rect,
            label: label.into(),
            visual: Visual::Normal,
        }
    }

    pub fn rect(&self) -> Rectangle {
        self.rect
    }

    pub fn contains(&self, point: Point) -> bool {
        self.rect.contains(point)
    }

    /// Current body color (part of the rendering contract).
    pub fn fill_color(&self) -> Rgb888 {
        match self.visual {
            Visual::Normal => Rgb888::new(0xE0, 0xE0, 0xE0),
            Visual::Hovered => Rgb888::new(0xF5, 0xF5, 0xF5),
            Visual::Pressed => Rgb888::new(0xB8, 0xB8, 0xB8),
        }
    }

    /// Border color (part of the rendering contract).
    pub fn border_color(&self) -> Rgb888 {
        match self.visual {
            Visual::Pressed => Rgb888::new(0x30, 0x30, 0x30),
            _ => Rgb888::new(0x60, 0x60, 0x60),
        }
    }

    /// Feed one input event; returns damage and whether it counts as a click.
    pub fn handle(&mut self, event: &InputEvent) -> ButtonOutcome {
        match *event {
            InputEvent::MouseMoved { x, y } => {
                let next = if self.contains(Point::new(x, y)) {
                    Visual::Hovered
                } else {
                    Visual::Normal
                };
                self.set_visual(next)
            }
            InputEvent::MousePressed { x, y } => {
                if self.contains(Point::new(x, y)) {
                    self.set_visual(Visual::Pressed)
                } else {
                    ButtonOutcome::clean()
                }
            }
            InputEvent::MouseReleased { x, y } => {
                if self.visual != Visual::Pressed {
                    return ButtonOutcome::clean();
                }
                let inside = self.contains(Point::new(x, y));
                let outcome = self.set_visual(if inside {
                    Visual::Hovered
                } else {
                    Visual::Normal
                });
                ButtonOutcome {
                    clicked: inside,
                    ..outcome
                }
            }
            InputEvent::Resized { .. } | InputEvent::CloseRequested => ButtonOutcome::clean(),
        }
    }

    fn set_visual(&mut self, next: Visual) -> ButtonOutcome {
        if self.visual == next {
            return ButtonOutcome::clean();
        }
        self.visual = next;
        ButtonOutcome {
            damage: Some(self.rect),
            clicked: false,
        }
    }

    /// Paint the button: fill, 1px border, centered label.
    pub fn render<T: DrawTarget<Color = Rgb888>>(
        &self,
        target: &mut T,
    ) -> Result<(), T::Error> {
        self.rect
            .into_styled(PrimitiveStyle::with_fill(self.fill_color()))
            .draw(target)?;
        self.rect
            .into_styled(PrimitiveStyle::with_stroke(self.border_color(), 1))
            .draw(target)?;
        let glyph = FONT_8X13.character_size;
        let text_w = self.label.chars().count() as i32 * glyph.width as i32;
        let center = self.rect.center();
        let origin = Point::new(
            center.x - text_w / 2,
            center.y - glyph.height as i32 / 2,
        );
        Text::new(
            &self.label,
            origin,
            MonoTextStyle::new(&FONT_8X13, Rgb888::new(0x20, 0x20, 0x20)),
        )
        .draw(target)
        .map(|_| ())
    }
}
