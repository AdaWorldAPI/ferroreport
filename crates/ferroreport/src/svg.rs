//! SVG images: parsed with usvg (text turned into outlines with the bundled fonts), drawn as
//! vector graphics in PDFs and rasterized with resvg for pictures.
//!
//! Like JasperReports (Batik), one SVG pixel is one point (and an inch 96 pixels). Nothing
//! outside the SVG is read: images inside it must be `data:` URLs.

use crate::error::{Error, Result};
use resvg::tiny_skia;
use resvg::usvg;
use std::sync::{Arc, OnceLock};

/// A parsed SVG and the file it came from.
pub struct SvgImage {
    pub tree: usvg::Tree,
    pub source: Arc<[u8]>,
}

impl std::fmt::Debug for SvgImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SvgImage({}x{})", self.tree.size().width(), self.tree.size().height())
    }
}

/// Whether `bytes` look like an SVG document (or a gzip-compressed one).
pub fn sniff(bytes: &[u8]) -> bool {
    if bytes.starts_with(&[0x1f, 0x8b]) {
        return true;
    }
    let head = &bytes[..bytes.len().min(1024)];
    let text = String::from_utf8_lossy(head);
    let t = text.trim_start_matches('\u{feff}').trim_start();
    (t.starts_with("<?xml") || t.starts_with("<svg") || t.starts_with("<!--") || t.starts_with("<!DOCTYPE svg"))
        && text.contains("<svg")
}

fn fontdb() -> Arc<usvg::fontdb::Database> {
    static DB: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = usvg::fontdb::Database::new();
        for file in crate::fonts::builtin_files() {
            db.load_font_data(file.to_vec());
        }
        db.set_sans_serif_family("DejaVu Sans");
        db.set_serif_family("DejaVu Serif");
        db.set_monospace_family("DejaVu Sans Mono");
        Arc::new(db)
    })
    .clone()
}

/// Parses an SVG document.
pub fn parse(bytes: &[u8]) -> Result<SvgImage> {
    let options = usvg::Options {
        // As Batik in JasperReports: lengths in inches or millimetres at 96 pixels to the
        // inch, and then one pixel drawn as one point.
        dpi: 96.0,
        font_family: "DejaVu Sans".into(),
        fontdb: fontdb(),
        resources_dir: None,
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: usvg::ImageHrefResolver::default_data_resolver(),
            // Never a file or a URL.
            resolve_string: Box::new(|_, _| None),
        },
        ..Default::default()
    };
    let tree = usvg::Tree::from_data(bytes, &options).map_err(|e| Error::Data(format!("SVG: {e}")))?;
    Ok(SvgImage { tree, source: Arc::from(bytes) })
}

/// The SVG drawn into a `width`×`height` pixmap, stretched to fill it.
pub fn rasterize(tree: &usvg::Tree, width: u32, height: u32) -> Option<tiny_skia::Pixmap> {
    let mut pixmap = tiny_skia::Pixmap::new(width.max(1), height.max(1))?;
    let size = tree.size();
    let t = tiny_skia::Transform::from_scale(width as f32 / size.width(), height as f32 / size.height());
    resvg::render(tree, t, &mut pixmap.as_mut());
    Some(pixmap)
}

/// The SVG as a PNG file of `width`×`height` pixels.
pub fn png(tree: &usvg::Tree, width: u32, height: u32) -> Option<Vec<u8>> {
    let pixmap = rasterize(tree, width, height)?;
    let mut rgba = Vec::with_capacity(pixmap.data().len());
    for px in pixmap.pixels() {
        let c = px.demultiply();
        rgba.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }
    let mut file = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut file, pixmap.width(), pixmap.height());
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().ok()?;
        writer.write_image_data(&rgba).ok()?;
    }
    Some(file)
}

/// Pixels for a picture of the SVG that keeps it sharp: two to a point, at most 4096 a side.
pub fn picture_size(tree: &usvg::Tree) -> (u32, u32) {
    let size = tree.size();
    let k = (2.0f32).min(4096.0 / size.width().max(size.height()).max(1.0));
    (((size.width() * k).round() as u32).max(1), ((size.height() * k).round() as u32).max(1))
}

/// A gradient fill, as a PDF shading pattern needs it.
#[derive(Debug, Clone)]
pub struct Gradient {
    /// Axial: `[x1, y1, x2, y2]`; radial: `[fx, fy, 0, cx, cy, r]`.
    pub coords: Vec<f32>,
    pub radial: bool,
    /// Offsets from 0 to 1 and RGB colors, from 0 at the start to 1 at the end.
    pub stops: Vec<(f32, [f32; 3])>,
    /// From the gradient's space to the SVG's.
    pub matrix: [f32; 6],
}

/// What a PDF form drawing an SVG is made of: its content, and the resources the content
/// names (`G<n>` graphics states, `P<n>` patterns, `X<n>` images).
#[derive(Debug, Default)]
pub struct Form {
    pub content: Vec<u8>,
    pub width: f32,
    pub height: f32,
    /// Fill and stroke opacities.
    pub alphas: Vec<(f32, f32)>,
    pub gradients: Vec<Gradient>,
    pub images: Vec<Arc<crate::image::ImageData>>,
}

/// Whether the SVG uses something drawn only by rasterizing it (filters, masks, patterns).
pub fn needs_raster(tree: &usvg::Tree) -> bool {
    !tree.filters().is_empty() || !tree.masks().is_empty() || !tree.patterns().is_empty()
}

/// The SVG as a PDF form: vector paths, gradients, opacity, clipping, outlined text and
/// embedded pictures, in the SVG's own coordinates (y down).
pub fn form(tree: &usvg::Tree) -> Form {
    let mut f = Form { width: tree.size().width(), height: tree.size().height(), ..Default::default() };
    let mut c = pdf_writer::Content::new();
    group(&mut c, tree.root(), &mut f, 1.0);
    f.content = c.finish().to_vec();
    f
}

fn matrix(t: tiny_skia::Transform) -> [f32; 6] {
    [t.sx, t.ky, t.kx, t.sy, t.tx, t.ty]
}

fn group(c: &mut pdf_writer::Content, g: &usvg::Group, f: &mut Form, opacity: f32) {
    c.save_state();
    let t = g.transform();
    if !t.is_identity() {
        c.transform(matrix(t));
    }
    if let Some(cp) = g.clip_path() {
        clip(c, cp);
    }
    // JasperReports draws SVGs with Batik onto iText, which draws an element or group with
    // an `opacity` as an opaque picture of it (fill and stroke opacities are kept); so the
    // `opacity` is left out here too.
    let _ = g.opacity();
    for node in g.children() {
        match node {
            usvg::Node::Group(g) => group(c, g, f, opacity),
            usvg::Node::Path(p) => path(c, p, f, opacity),
            usvg::Node::Image(i) => image(c, i, f, opacity),
            usvg::Node::Text(t) => group(c, t.flattened(), f, opacity),
        }
    }
    c.restore_state();
}

fn segments(c: &mut pdf_writer::Content, data: &tiny_skia::Path) {
    use tiny_skia::PathSegment;
    let mut last = (0.0f32, 0.0f32);
    for seg in data.segments() {
        match seg {
            PathSegment::MoveTo(p) => {
                c.move_to(p.x, p.y);
                last = (p.x, p.y);
            }
            PathSegment::LineTo(p) => {
                c.line_to(p.x, p.y);
                last = (p.x, p.y);
            }
            PathSegment::QuadTo(q, p) => {
                // A quadratic curve as the cubic one it is.
                let c1 = (last.0 + 2.0 / 3.0 * (q.x - last.0), last.1 + 2.0 / 3.0 * (q.y - last.1));
                let c2 = (p.x + 2.0 / 3.0 * (q.x - p.x), p.y + 2.0 / 3.0 * (q.y - p.y));
                c.cubic_to(c1.0, c1.1, c2.0, c2.1, p.x, p.y);
                last = (p.x, p.y);
            }
            PathSegment::CubicTo(a, b, p) => {
                c.cubic_to(a.x, a.y, b.x, b.y, p.x, p.y);
                last = (p.x, p.y);
            }
            PathSegment::Close => {
                c.close_path();
            }
        }
    }
}

/// Clips to a clip path: the outlines of its shapes, in the current coordinates.
fn clip(c: &mut pdf_writer::Content, cp: &usvg::ClipPath) {
    if let Some(inner) = cp.clip_path() {
        clip(c, inner);
    }
    let mut even_odd = true;
    let mut any = false;
    collect_clip(c, cp.root(), cp.transform(), &mut even_odd, &mut any);
    if !any {
        // Nothing to show through: an empty clip.
        c.rect(0.0, 0.0, 0.0, 0.0);
    }
    if even_odd && any {
        c.clip_even_odd();
    } else {
        c.clip_nonzero();
    }
    c.end_path();
}

fn collect_clip(c: &mut pdf_writer::Content, g: &usvg::Group, t: tiny_skia::Transform, even_odd: &mut bool, any: &mut bool) {
    let t = t.pre_concat(g.transform());
    for node in g.children() {
        match node {
            usvg::Node::Group(g) => collect_clip(c, g, t, even_odd, any),
            usvg::Node::Text(text) => collect_clip(c, text.flattened(), t, even_odd, any),
            usvg::Node::Path(p) => {
                if let Some(data) = p.data().clone().transform(t) {
                    segments(c, &data);
                    *any = true;
                    if p.fill().map(|f| f.rule()) != Some(usvg::FillRule::EvenOdd) {
                        *even_odd = false;
                    }
                }
            }
            usvg::Node::Image(_) => {}
        }
    }
}

fn alpha(f: &mut Form, fill: f32, stroke: f32) -> String {
    let key = ((fill * 1000.0).round() / 1000.0, (stroke * 1000.0).round() / 1000.0);
    let i = f.alphas.iter().position(|a| *a == key).unwrap_or_else(|| {
        f.alphas.push(key);
        f.alphas.len() - 1
    });
    format!("G{}", i + 1)
}

/// Sets the paint of a fill or a stroke; returns the opacity it adds (gradient stops').
fn paint(c: &mut pdf_writer::Content, p: &usvg::Paint, stroke: bool, at: tiny_skia::Transform, f: &mut Form) -> f32 {
    use pdf_writer::types::ColorSpaceOperand;
    let rgb = |col: usvg::Color| [col.red as f32 / 255.0, col.green as f32 / 255.0, col.blue as f32 / 255.0];
    let gradient = |base: &usvg::BaseGradient, coords: Vec<f32>, radial: bool, f: &mut Form| {
        let mut stops: Vec<(f32, [f32; 3])> = base.stops().iter().map(|s| (s.offset().get(), rgb(s.color()))).collect();
        if stops.is_empty() {
            stops.push((0.0, [0.0; 3]));
        }
        let opacity = base.stops().iter().map(|s| s.opacity().get()).sum::<f32>() / base.stops().len().max(1) as f32;
        f.gradients.push(Gradient { coords, radial, stops, matrix: matrix(at.pre_concat(base.transform())) });
        (format!("P{}", f.gradients.len()), opacity)
    };
    match p {
        usvg::Paint::Color(col) => {
            let [r, g, b] = rgb(*col);
            if stroke {
                c.set_stroke_rgb(r, g, b);
            } else {
                c.set_fill_rgb(r, g, b);
            }
            1.0
        }
        usvg::Paint::LinearGradient(lg) => {
            let (name, opacity) = gradient(lg, vec![lg.x1(), lg.y1(), lg.x2(), lg.y2()], false, f);
            set_pattern(c, stroke, &name);
            opacity
        }
        usvg::Paint::RadialGradient(rg) => {
            let (name, opacity) = gradient(rg, vec![rg.fx(), rg.fy(), 0.0, rg.cx(), rg.cy(), rg.r().get()], true, f);
            set_pattern(c, stroke, &name);
            opacity
        }
        usvg::Paint::Pattern(_) => {
            // Drawn by rasterizing the whole SVG (see `needs_raster`); never reached.
            let _ = ColorSpaceOperand::DeviceGray;
            if stroke {
                c.set_stroke_gray(0.0);
            } else {
                c.set_fill_gray(0.0);
            }
            1.0
        }
    }
}

fn set_pattern(c: &mut pdf_writer::Content, stroke: bool, name: &str) {
    use pdf_writer::types::ColorSpaceOperand;
    use pdf_writer::Name;
    if stroke {
        c.set_stroke_color_space(ColorSpaceOperand::Pattern);
        c.set_stroke_pattern(None, Name(name.as_bytes()));
    } else {
        c.set_fill_color_space(ColorSpaceOperand::Pattern);
        c.set_fill_pattern(None, Name(name.as_bytes()));
    }
}

fn path(c: &mut pdf_writer::Content, p: &usvg::Path, f: &mut Form, opacity: f32) {
    use pdf_writer::types::{LineCapStyle, LineJoinStyle};
    use pdf_writer::Name;
    if !p.is_visible() {
        return;
    }
    let at = p.abs_transform();
    let fill = |c: &mut pdf_writer::Content, f: &mut Form| {
        let Some(fill) = p.fill() else { return };
        c.save_state();
        let extra = paint(c, fill.paint(), false, at, f);
        let a = opacity * fill.opacity().get() * extra;
        if a < 1.0 {
            let g = alpha(f, a, 1.0);
            c.set_parameters(Name(g.as_bytes()));
        }
        segments(c, p.data());
        if fill.rule() == usvg::FillRule::EvenOdd {
            c.fill_even_odd();
        } else {
            c.fill_nonzero();
        }
        c.restore_state();
    };
    let stroke = |c: &mut pdf_writer::Content, f: &mut Form| {
        let Some(s) = p.stroke() else { return };
        c.save_state();
        let extra = paint(c, s.paint(), true, at, f);
        let a = opacity * s.opacity().get() * extra;
        if a < 1.0 {
            let g = alpha(f, 1.0, a);
            c.set_parameters(Name(g.as_bytes()));
        }
        c.set_line_width(s.width().get());
        c.set_line_cap(match s.linecap() {
            usvg::LineCap::Butt => LineCapStyle::ButtCap,
            usvg::LineCap::Round => LineCapStyle::RoundCap,
            usvg::LineCap::Square => LineCapStyle::ProjectingSquareCap,
        });
        c.set_line_join(match s.linejoin() {
            usvg::LineJoin::Round => LineJoinStyle::RoundJoin,
            usvg::LineJoin::Bevel => LineJoinStyle::BevelJoin,
            _ => LineJoinStyle::MiterJoin,
        });
        c.set_miter_limit(s.miterlimit().get());
        if let Some(dash) = s.dasharray() {
            c.set_dash_pattern(dash.iter().copied(), s.dashoffset());
        }
        segments(c, p.data());
        c.stroke();
        c.restore_state();
    };
    match p.paint_order() {
        usvg::PaintOrder::FillAndStroke => {
            fill(c, f);
            stroke(c, f);
        }
        usvg::PaintOrder::StrokeAndFill => {
            stroke(c, f);
            fill(c, f);
        }
    }
}

fn image(c: &mut pdf_writer::Content, i: &usvg::Image, f: &mut Form, opacity: f32) {
    use pdf_writer::Name;
    if !i.is_visible() {
        return;
    }
    let size = i.size();
    match i.kind() {
        usvg::ImageKind::SVG(inner) => {
            c.save_state();
            let s = inner.size();
            c.transform([size.width() / s.width(), 0.0, 0.0, size.height() / s.height(), 0.0, 0.0]);
            group(c, inner.root(), f, opacity);
            c.restore_state();
        }
        usvg::ImageKind::PNG(data) | usvg::ImageKind::JPEG(data) | usvg::ImageKind::GIF(data) => {
            let Ok(img) = crate::image::decode_embedded(data) else { return };
            f.images.push(img);
            let name = format!("X{}", f.images.len());
            c.save_state();
            if opacity < 1.0 {
                let g = alpha(f, opacity, opacity);
                c.set_parameters(Name(g.as_bytes()));
            }
            // The picture's top row at the top of its box (y goes down here).
            c.transform([size.width(), 0.0, 0.0, -size.height(), 0.0, size.height()]);
            c.x_object(Name(name.as_bytes()));
            c.restore_state();
        }
        usvg::ImageKind::WEBP(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_shapes_gradients_and_text() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100" viewBox="0 0 400 200">
            <defs><linearGradient id="g"><stop offset="0" stop-color="#f00"/><stop offset="1" stop-color="#00f"/></linearGradient></defs>
            <rect x="10" y="10" width="180" height="80" fill="url(#g)" stroke="black" stroke-width="4" fill-opacity="0.5"/>
            <circle cx="300" cy="100" r="50" fill="#0a0"/>
            <text x="20" y="180" font-size="30">Hello</text>
        </svg>"##;
        assert!(sniff(svg));
        let img = parse(svg).unwrap();
        assert_eq!((img.tree.size().width(), img.tree.size().height()), (200.0, 100.0));
        let f = form(&img.tree);
        assert_eq!(f.gradients.len(), 1);
        assert!(!f.alphas.is_empty());
        let text = String::from_utf8_lossy(&f.content);
        // The text drawn as outlines: many curves.
        assert!(text.matches(" c\n").count() > 20, "{text}");
        let px = rasterize(&img.tree, 200, 100).unwrap();
        assert!(px.data().iter().any(|&b| b != 0));
    }

    #[test]
    fn reads_nothing_outside_the_file() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="10" height="10">
            <image xlink:href="/etc/passwd" width="10" height="10"/></svg>"#;
        let img = parse(svg).unwrap();
        assert!(form(&img.tree).images.is_empty());
    }
}
