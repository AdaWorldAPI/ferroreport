//! Draws print items into a bitmap, for exporters that need a picture of an element (charts and
//! barcodes in spreadsheets).

use crate::fonts::FontSet;
use crate::image::Pixels;
use crate::model::{Color, LineStyle, Pen};
use crate::print::{PathOp, PrintItem};
use tiny_skia::{FillRule, IntSize, LineCap, Paint, PathBuilder, Pixmap, PixmapPaint, Rect, Stroke, StrokeDash, Transform};

/// Draws the items of an area of a page (`x, y, w, h` in points) at `scale` pixels per point
/// and returns it as a PNG with a transparent background.
pub(crate) fn render_png(items: &[PrintItem], fonts: &FontSet, area: [f32; 4], scale: f32) -> Option<Vec<u8>> {
    let [x, y, w, h] = area;
    let (pw, ph) = ((w * scale).round().max(1.0) as u32, (h * scale).round().max(1.0) as u32);
    let mut pixmap = Pixmap::new(pw, ph)?;
    let base = Transform::from_row(scale, 0.0, 0.0, scale, -x * scale, -y * scale);
    let mut r = Raster { pixmap: &mut pixmap, fonts };
    r.items(items, base);
    encode(&pixmap)
}

struct Raster<'a> {
    pixmap: &'a mut Pixmap,
    fonts: &'a FontSet,
}

fn paint(c: Color) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color_rgba8(c.r, c.g, c.b, 255);
    p.anti_alias = true;
    p
}

fn path(ops: &[PathOp]) -> Option<tiny_skia::Path> {
    let mut b = PathBuilder::new();
    for op in ops {
        match *op {
            PathOp::Move(a, c) => b.move_to(a as f32, c as f32),
            PathOp::Line(a, c) => b.line_to(a as f32, c as f32),
            PathOp::Quad(a, c, d, e) => b.quad_to(a as f32, c as f32, d as f32, e as f32),
            PathOp::Cubic(a, c, d, e, f, g) => b.cubic_to(a as f32, c as f32, d as f32, e as f32, f as f32, g as f32),
            PathOp::Close => b.close(),
        }
    }
    b.finish()
}

fn pen_stroke(pen: &Pen) -> Stroke {
    let mut s = Stroke { width: pen.width, line_cap: LineCap::Butt, ..Stroke::default() };
    let w = pen.width.max(0.5);
    s.dash = match pen.style {
        LineStyle::Dashed => StrokeDash::new(vec![5.0 * w, 3.0 * w], 0.0),
        LineStyle::Dotted => StrokeDash::new(vec![w, w], 0.0),
        _ => None,
    };
    s
}

impl Raster<'_> {
    fn items(&mut self, items: &[PrintItem], t: Transform) {
        for item in items {
            self.item(item, t);
        }
    }

    fn fill(&mut self, p: &tiny_skia::Path, color: Color, t: Transform) {
        self.pixmap.fill_path(p, &paint(color), FillRule::Winding, t, None);
    }

    fn stroke(&mut self, p: &tiny_skia::Path, color: Color, stroke: &Stroke, t: Transform) {
        if stroke.width > 0.0 {
            self.pixmap.stroke_path(p, &paint(color), stroke, t, None);
        }
    }

    fn item(&mut self, item: &PrintItem, t: Transform) {
        match item {
            PrintItem::Cell(_, inner) => self.item(inner, t),
            PrintItem::Group(items) => self.items(items, t),
            PrintItem::Transform([a, b, c, d, e, f], items) => {
                self.items(items, t.pre_concat(Transform::from_row(*a, *b, *c, *d, *e, *f)))
            }
            PrintItem::Element(_) => {}
            PrintItem::Path { ops, fill, stroke, dash } => {
                let Some(p) = path(ops) else { return };
                if let Some(color) = fill {
                    self.fill(&p, *color, t);
                }
                if let Some((color, width)) = stroke {
                    let mut s = Stroke { width: *width, line_cap: LineCap::Butt, ..Stroke::default() };
                    s.dash = dash.and_then(|d| StrokeDash::new(d.to_vec(), 0.0));
                    self.stroke(&p, *color, &s, t);
                }
            }
            PrintItem::Rect { x, y, w, h, fill, stroke, radius, .. } => {
                let p = if *radius > 0.0 {
                    rounded(*x, *y, *w, *h, *radius)
                } else {
                    Rect::from_xywh(*x, *y, w.max(0.01), h.max(0.01)).map(PathBuilder::from_rect)
                };
                let Some(p) = p else { return };
                if let Some(color) = fill {
                    self.fill(&p, *color, t);
                }
                if let Some(pen) = stroke {
                    self.stroke(&p, pen.color, &pen_stroke(pen), t);
                }
            }
            PrintItem::Ellipse { x, y, w, h, fill, stroke } => {
                let Some(p) = Rect::from_xywh(*x, *y, w.max(0.01), h.max(0.01)).and_then(PathBuilder::from_oval) else {
                    return;
                };
                if let Some(color) = fill {
                    self.fill(&p, *color, t);
                }
                if let Some(pen) = stroke {
                    self.stroke(&p, pen.color, &pen_stroke(pen), t);
                }
            }
            PrintItem::Line { x1, y1, x2, y2, pen } => {
                let mut b = PathBuilder::new();
                b.move_to(*x1, *y1);
                b.line_to(*x2, *y2);
                if let Some(p) = b.finish() {
                    self.stroke(&p, pen.color, &pen_stroke(pen), t);
                }
            }
            PrintItem::Shapes { x, y, scale, color, rects, path: outline } => {
                let s = *scale as f32;
                let local = t.pre_concat(Transform::from_row(s, 0.0, 0.0, s, *x, *y));
                let mut b = PathBuilder::new();
                for [rx, ry, rw, rh] in rects.iter() {
                    if let Some(r) = Rect::from_xywh(*rx as f32, *ry as f32, *rw as f32, *rh as f32) {
                        b.push_rect(r);
                    }
                }
                if let Some(p) = b.finish() {
                    self.fill(&p, *color, local);
                }
                if let Some(p) = path(outline) {
                    self.fill(&p, *color, local);
                }
            }
            PrintItem::Text(text) => {
                let face = self.fonts.face(text.face);
                let size = text.size as f64;
                for line in &text.lines {
                    let mut ops = Vec::new();
                    let mut pen_x = line.x as f64;
                    for c in line.text.chars() {
                        face.glyph_outline(c, size, pen_x, line.baseline as f64, &mut ops);
                        pen_x += face.char_width(c, text.size) as f64;
                        if c == ' ' {
                            pen_x += line.word_spacing as f64;
                        }
                    }
                    if let Some(p) = path(&ops) {
                        self.fill(&p, text.color, t);
                    }
                    let thick = text.size / 18.0;
                    for (on, offset) in [(text.underline, text.size / 12.0), (text.strike, -text.size / 3.0)] {
                        if on && line.width > 0.0 {
                            if let Some(r) = Rect::from_xywh(line.x, line.baseline + offset - thick / 2.0, line.width, thick) {
                                self.fill(&PathBuilder::from_rect(r), text.color, t);
                            }
                        }
                    }
                }
            }
            PrintItem::Image { x, y, w, h, clip, image } => {
                // Decoded images only (charts do not hold JPEG files).
                let Pixels::Raw { samples, gray, alpha } = &image.pixels else { return };
                let Some(size) = IntSize::from_wh(image.width, image.height) else { return };
                let channels = if *gray { 1 } else { 3 };
                let mut rgba = Vec::with_capacity((image.width * image.height * 4) as usize);
                for (i, px) in samples.chunks(channels).enumerate() {
                    let a = alpha.as_ref().and_then(|a| a.get(i).copied()).unwrap_or(255);
                    let (r, g, b) = if *gray { (px[0], px[0], px[0]) } else { (px[0], px[1], px[2]) };
                    let pm = |v: u8| ((v as u16 * a as u16 + 127) / 255) as u8;
                    rgba.extend_from_slice(&[pm(r), pm(g), pm(b), a]);
                }
                let Some(src) = Pixmap::from_vec(rgba, size) else { return };
                let place = t.pre_concat(Transform::from_row(w / image.width as f32, 0.0, 0.0, h / image.height as f32, *x, *y));
                let mask = clip.and_then(|[cx, cy, cw, ch]| {
                    let mut m = tiny_skia::Mask::new(self.pixmap.width(), self.pixmap.height())?;
                    let r = PathBuilder::from_rect(Rect::from_xywh(cx, cy, cw, ch)?);
                    m.fill_path(&r, FillRule::Winding, true, t);
                    Some(m)
                });
                self.pixmap.draw_pixmap(0, 0, src.as_ref(), &PixmapPaint::default(), place, mask.as_ref());
            }
        }
    }
}

/// A rectangle with rounded corners.
fn rounded(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<tiny_skia::Path> {
    let r = r.min(w / 2.0).min(h / 2.0);
    let k = 0.552_284_8 * r;
    let mut b = PathBuilder::new();
    b.move_to(x + r, y);
    b.line_to(x + w - r, y);
    b.cubic_to(x + w - r + k, y, x + w, y + r - k, x + w, y + r);
    b.line_to(x + w, y + h - r);
    b.cubic_to(x + w, y + h - r + k, x + w - r + k, y + h, x + w - r, y + h);
    b.line_to(x + r, y + h);
    b.cubic_to(x + r - k, y + h, x, y + h - r + k, x, y + h - r);
    b.line_to(x, y + r);
    b.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
    b.close();
    b.finish()
}

/// A pixmap as an RGBA PNG (tiny-skia keeps premultiplied colors).
fn encode(pixmap: &Pixmap) -> Option<Vec<u8>> {
    let mut data = Vec::with_capacity(pixmap.data().len());
    for p in pixmap.pixels() {
        let c = p.demultiply();
        data.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, pixmap.width(), pixmap.height());
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::Best);
        let mut w = enc.write_header().ok()?;
        w.write_image_data(&data).ok()?;
    }
    Some(out)
}
