//! Damage tracking: widgets mark invalidated regions; the repaint step takes
//! the coalesced (enclosing) region once per paint.

use embedded_graphics::geometry::Size;
use embedded_graphics::geometry::Point;
use embedded_graphics::primitives::Rectangle;

/// Coalesced dirty region. Marks accumulate into the enclosing rectangle;
/// `take` returns it and clears the damage.
#[derive(Debug, Default)]
pub struct Damage {
    rect: Option<Rectangle>,
}

impl Damage {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mark `rect` as needing repaint. Zero-area rects are ignored.
    pub fn mark(&mut self, rect: Rectangle) {
        let Some(rect) = non_empty(rect) else { return };
        self.rect = Some(match self.rect {
            Some(existing) => enclosing(existing, rect),
            None => rect,
        });
    }

    /// Mark the full `size` area as needing repaint.
    pub fn mark_all(&mut self, size: Size) {
        self.mark(Rectangle::new(
            embedded_graphics::geometry::Point::zero(),
            size,
        ));
    }

    /// Whether any region is dirty.
    pub fn is_dirty(&self) -> bool {
        self.rect.is_some()
    }

    /// Take the coalesced dirty region, clearing the damage.
    pub fn take(&mut self) -> Option<Rectangle> {
        self.rect.take()
    }
}

fn non_empty(rect: Rectangle) -> Option<Rectangle> {
    rect.bottom_right().map(|_| rect)
}

/// Smallest rectangle containing both inputs (both non-empty).
fn enclosing(a: Rectangle, b: Rectangle) -> Rectangle {
    let (a_br, b_br) = (a.bottom_right().expect("non-empty"), b.bottom_right().expect("non-empty"));
    let top_left = Point::new(a.top_left.x.min(b.top_left.x), a.top_left.y.min(b.top_left.y));
    let bottom_right = Point::new(a_br.x.max(b_br.x), a_br.y.max(b_br.y));
    Rectangle::with_corners(top_left, bottom_right)
}
