//! The paint engine seam: a [`Painter`] owns the vello CPU render context,
//! the glyph resources, and the parley text stacks. Widgets paint into it
//! (vello's own representation — no custom display list, no renderer trait
//! matrix), and one call rasterizes everything into the `u32` frame that
//! softbuffer presents.
//!
//! Rendering is single-threaded and deterministic: the same paint sequence
//! produces the same pixel output, run to run.

use parley::layout::PositionedLayoutItem;
use parley::{Alignment, AlignmentOptions, FontFamily, GenericFamily, StyleProperty};
use vello_cpu::Glyph;
use vello_cpu::PixmapMut;
use vello_cpu::kurbo::{Point as KPoint, Rect as KRect};
use vello_cpu::peniko::{Color, Fill};
use vello_cpu::{RasterizerSettings, RenderContext, Resources, TargetInit};

use crate::pixel::premul_rgba8_to_frame;

/// The default label font size (matches the retired 8x13 bitmap metrics).
pub(crate) const LABEL_FONT_SIZE: f32 = 13.0;
/// The default label/text color (part of the rendering contract).
pub(crate) const LABEL_COLOR: Color = Color::from_rgba8(0x20, 0x20, 0x20, 0xFF);

/// Opaque black background (matches the zeroed `u32` frame).
const BACKGROUND: Color = Color::from_rgba8(0x00, 0x00, 0x00, 0xFF);

/// Owns the vello CPU paint state for one surface.
pub struct Painter {
    width: u32,
    height: u32,
    rc: RenderContext,
    resources: Resources,
    rgba: Vec<u8>,
    fonts: parley::FontContext,
    layout: parley::LayoutContext<[u8; 4]>,
}

impl Painter {
    pub fn new(width: u32, height: u32) -> Self {
        let mut painter = Self {
            width: 0,
            height: 0,
            rc: RenderContext::new(0, 0),
            resources: Resources::new(),
            rgba: Vec::new(),
            fonts: parley::FontContext::default(),
            layout: parley::LayoutContext::new(),
        };
        painter.resize(width, height);
        painter
    }

    /// Resize the paint surface (frame reallocation stays the caller's job).
    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
        self.rgba = vec![0; (width * height * 4) as usize];
        self.rc.reset_and_resize(
            u16::try_from(width).expect("surface width over u16"),
            u16::try_from(height).expect("surface height over u16"),
        );
    }

    /// Reset paint state for a fresh frame (transforms, layers, clips).
    pub fn begin_frame(&mut self) {
        self.rc.reset();
    }

    /// Fill `rect` with an opaque color.
    pub fn fill_rect(&mut self, rect: KRect, color: Color) {
        self.rc.set_paint(color);
        self.rc.set_fill_rule(Fill::NonZero);
        self.rc.fill_rect(&rect);
    }

    /// Draw a single-style text label with its line box top-left at `origin`.
    /// Minimal by design: one style, no wrapping (single line); full shaping
    /// quality is deferred to the later text spec.
    pub fn draw_label(&mut self, text: &str, origin: KPoint, color: Color) {
        if text.is_empty() {
            return;
        }
        let scale = 1.0;
        let mut builder = self
            .layout
            .ranged_builder(&mut self.fonts, text, scale, true);
        builder.push_default(StyleProperty::FontFamily(FontFamily::from(
            GenericFamily::SystemUi,
        )));
        builder.push_default(StyleProperty::FontSize(LABEL_FONT_SIZE));
        let mut layout = builder.build(text);
        layout.break_all_lines(None);
        layout.align(Alignment::Start, AlignmentOptions::default());

        for line in layout.lines() {
            let baseline = origin.y as f32 + line.metrics().baseline;
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(run) = item else {
                    continue;
                };
                let inner = run.run();
                let font = inner.font().clone();
                let font_size = inner.font_size();
                let run_offset = run.offset();
                let glyphs: Vec<Glyph> = run
                    .glyphs()
                    .map(|g| Glyph {
                        id: g.id,
                        x: run_offset + g.x,
                        y: baseline + g.y,
                    })
                    .collect();
                self.rc.set_paint(color);
                let _ = self
                    .rc
                    .glyph_run(&mut self.resources, &font)
                    .font_size(font_size)
                    .fill_glyphs(glyphs.into_iter());
            }
        }
    }

    /// Rasterize the accumulated paint into `frame` (row-major `u32`,
    /// softbuffer layout). The target is initialized to opaque black each
    /// frame, so callers do not need to clear it first.
    pub fn render_into(&mut self, frame: &mut [u32]) {
        debug_assert_eq!(frame.len(), (self.width * self.height) as usize);
        let settings = RasterizerSettings {
            target_init: TargetInit::Clear(BACKGROUND),
            ..RasterizerSettings::default()
        };
        let rgba = &mut self.rgba;
        let (width, height) = (self.width as u16, self.height as u16);
        let pixmap = PixmapMut::new(width, height, rgba).expect("scratch matches surface");
        self.rc.render_with(pixmap, &mut self.resources, settings);
        premul_rgba8_to_frame(&self.rgba, frame);
    }

    /// The surface size, for widgets that lay out relative to the surface.
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

/// Convenience: measurement of a single-style label, for centering.
/// Paints nothing; mirrors `draw_label`'s layout pass.
pub(crate) fn measure_label(painter: &mut Painter, text: &str) -> f64 {
    if text.is_empty() {
        return 0.0;
    }
    let mut builder = painter
        .layout
        .ranged_builder(&mut painter.fonts, text, 1.0, true);
    builder.push_default(StyleProperty::FontFamily(FontFamily::from(
        GenericFamily::SystemUi,
    )));
    builder.push_default(StyleProperty::FontSize(LABEL_FONT_SIZE));
    let mut layout = builder.build(text);
    layout.break_all_lines(None);
    layout.align(Alignment::Start, AlignmentOptions::default());
    f64::from(layout.width())
}
