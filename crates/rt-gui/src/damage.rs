//! Damage tracking: widgets mark invalidated regions; the repaint step takes
//! the coalesced (enclosing) region once per paint.

use vello_cpu::kurbo::Rect;

/// Coalesced dirty region. Marks accumulate into the enclosing rectangle;
/// `take` returns it and clears the damage.
#[derive(Debug, Default)]
pub struct Damage {
    rect: Option<Rect>,
}

impl Damage {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mark `rect` as needing repaint. Zero-area rects are ignored.
    pub fn mark(&mut self, rect: Rect) {
        let Some(rect) = non_empty(rect) else { return };
        self.rect = Some(match self.rect {
            Some(existing) => existing.union(rect),
            None => rect,
        });
    }

    /// Mark the full `width x height` area as needing repaint.
    pub fn mark_all(&mut self, width: u32, height: u32) {
        self.mark(Rect::new(
            0.0,
            0.0,
            f64::from(width),
            f64::from(height),
        ));
    }

    /// Whether any region is dirty.
    pub fn is_dirty(&self) -> bool {
        self.rect.is_some()
    }

    /// Take the coalesced dirty region, clearing the damage.
    pub fn take(&mut self) -> Option<Rect> {
        self.rect.take()
    }
}

fn non_empty(rect: Rect) -> Option<Rect> {
    (rect.width() > 0.0 && rect.height() > 0.0).then_some(rect)
}
