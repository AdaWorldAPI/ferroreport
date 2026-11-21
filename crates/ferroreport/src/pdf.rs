//! PDF exporter.
//!
//! Fonts are embedded as subsetted CID fonts (Identity-H) with a ToUnicode map, so text stays
//! selectable and searchable. Content streams are Flate-compressed.

use crate::error::{Error, Result};
use crate::fonts::{FaceId, FontSet};
use crate::image::{ImageData, Pixels};
use crate::model::{Color, LineStyle, Pen};
use crate::print::{PathOp, PrintDocument, PrintItem};
use pdf_writer::types::{CidFontType, FontFlags, LineCapStyle, SystemInfo};
use pdf_writer::{Content, Filter, Finish, Name, Pdf, Rect, Ref, Str, TextStr};
use std::collections::{BTreeMap, BTreeSet};
use subsetter::GlyphRemapper;

/// Document metadata written to the PDF info dictionary.
#[derive(Debug, Clone, Default)]
pub struct PdfOptions {
    pub title: Option<String>,
    pub compress: bool,
}

/// Level 3 is about twice as fast as the default level 6 and only ~8% larger (still a
/// quarter of the size of JasperReports' output).
fn deflate(data: &[u8]) -> Vec<u8> {
    miniz_oxide::deflate::compress_to_vec_zlib(data, 3)
}

struct Ids {
    next: i32,
}

impl Ids {
    fn next(&mut self) -> Ref {
        let r = Ref::new(self.next);
        self.next += 1;
        r
    }
}

/// iText (and so JasperReports) writes coordinates with two decimals; rounding the same way
/// keeps glyphs on the same pixel grid when rasterized.
fn r2(v: f32) -> f32 {
    (v * 100.0).round() / 100.0
}

fn rgb(c: Color) -> (f32, f32, f32) {
    (c.r as f32 / 255.0, c.g as f32 / 255.0, c.b as f32 / 255.0)
}

/// Sets stroke color, width and dash pattern. Lines and borders use JasperReports' line
/// patterns; rectangles and ellipses (`shape`) use its graphic-element patterns.
fn apply_pen(content: &mut Content, pen: &Pen, shape: bool) {
    let (r, g, b) = rgb(pen.color);
    content.set_stroke_rgb(r, g, b);
    content.set_line_width(pen.width);
    let w = pen.width.max(0.1);
    content.set_line_cap(LineCapStyle::ButtCap);
    match (pen.style, shape) {
        (LineStyle::Dashed, false) => content.set_dash_pattern([5.0 * w, 3.0 * w], 0.0),
        (LineStyle::Dotted, false) => content.set_dash_pattern([w, w], 0.0),
        (LineStyle::Dashed, true) => content.set_dash_pattern([4.0 * w, 4.0 * w], 0.0),
        (LineStyle::Dotted, true) => {
            content.set_line_cap(LineCapStyle::RoundCap);
            content.set_dash_pattern([0.0, 2.0 * w], 0.0)
        }
        _ => content.set_dash_pattern([], 0.0),
    };
}

/// Appends a rounded-rectangle path (PDF coordinates, y up).
fn rounded_rect(content: &mut Content, x: f32, y: f32, w: f32, h: f32, r: f32) {
    let r = r.min(w / 2.0).min(h / 2.0);
    let k = 0.552_284_8 * r;
    content.move_to(x + r, y);
    content.line_to(x + w - r, y);
    content.cubic_to(x + w - r + k, y, x + w, y + r - k, x + w, y + r);
    content.line_to(x + w, y + h - r);
    content.cubic_to(x + w, y + h - r + k, x + w - r + k, y + h, x + w - r, y + h);
    content.line_to(x + r, y + h);
    content.cubic_to(x + r - k, y + h, x, y + h - r + k, x, y + h - r);
    content.line_to(x, y + r);
    content.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
    content.close_path();
}

/// A rectangle as iText's `roundRectangle` writes it with radius 0: lines with zero-length
/// curves at the corners, not closed.
fn itext_rect(content: &mut Content, x: f32, y: f32, w: f32, h: f32) {
    content.move_to(x, y);
    content.line_to(x + w, y);
    content.cubic_to(x + w, y, x + w, y, x + w, y);
    content.line_to(x + w, y + h);
    content.cubic_to(x + w, y + h, x + w, y + h, x + w, y + h);
    content.line_to(x, y + h);
    content.cubic_to(x, y + h, x, y + h, x, y + h);
    content.line_to(x, y);
    content.cubic_to(x, y, x, y, x, y);
}

fn ellipse(content: &mut Content, x: f32, y: f32, w: f32, h: f32) {
    let (rx, ry) = (w / 2.0, h / 2.0);
    let (cx, cy) = (x + rx, y + ry);
    let (kx, ky) = (0.552_284_8 * rx, 0.552_284_8 * ry);
    content.move_to(cx + rx, cy);
    content.cubic_to(cx + rx, cy + ky, cx + kx, cy + ry, cx, cy + ry);
    content.cubic_to(cx - kx, cy + ry, cx - rx, cy + ky, cx - rx, cy);
    content.cubic_to(cx - rx, cy - ky, cx - kx, cy - ry, cx, cy - ry);
    content.cubic_to(cx + kx, cy - ry, cx + rx, cy - ky, cx + rx, cy);
    content.close_path();
}

/// Six-letter subset tag derived from the glyph set, as required for subset font names.
fn subset_tag(face: FaceId, glyphs: &BTreeSet<u16>) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325 ^ face as u64;
    for g in glyphs {
        h = (h ^ *g as u64).wrapping_mul(0x0100_0000_01b3);
    }
    (0..6).map(|i| (b'A' + ((h >> (i * 5)) % 26) as u8) as char).collect()
}

/// Writes an image XObject (plus its soft mask for transparency).
fn write_image(pdf: &mut Pdf, ids: &mut Ids, r: Ref, image: &ImageData) {
    match &image.pixels {
        Pixels::Jpeg { data, components } => {
            let mut x = pdf.image_xobject(r, data);
            x.filter(Filter::DctDecode);
            x.width(image.width as i32).height(image.height as i32).bits_per_component(8);
            match components {
                1 => x.color_space().device_gray(),
                4 => x.color_space().device_cmyk(),
                _ => x.color_space().device_rgb(),
            };
        }
        Pixels::Raw { samples, gray, alpha } => {
            let mask = alpha.as_ref().map(|a| {
                let mr = ids.next();
                let compressed = deflate(a);
                let mut m = pdf.image_xobject(mr, &compressed);
                m.filter(Filter::FlateDecode);
                m.width(image.width as i32).height(image.height as i32).bits_per_component(8);
                m.color_space().device_gray();
                mr
            });
            let compressed = deflate(samples);
            let mut x = pdf.image_xobject(r, &compressed);
            x.filter(Filter::FlateDecode);
            x.width(image.width as i32).height(image.height as i32).bits_per_component(8);
            if *gray {
                x.color_space().device_gray();
            } else {
                x.color_space().device_rgb();
            }
            if let Some(mr) = mask {
                x.s_mask(mr);
            }
        }
    }
}

/// One drawing step: an item, or entering/leaving a transformed coordinate system.
enum Draw<'a> {
    Item(&'a PrintItem),
    Push([f32; 6]),
    Pop,
}

/// Items in drawing order with groups expanded.
fn flatten<'a>(items: &'a [PrintItem], out: &mut Vec<Draw<'a>>) {
    for item in items {
        match item {
            PrintItem::Group(inner) => flatten(inner, out),
            PrintItem::Cell(_, inner) => flatten(std::slice::from_ref(inner), out),
            PrintItem::Transform(m, inner) => {
                out.push(Draw::Push(*m));
                flatten(inner, out);
                out.push(Draw::Pop);
            }
            other => out.push(Draw::Item(other)),
        }
    }
}

/// Serialises a filled document to PDF bytes.
pub fn write_pdf(doc: &PrintDocument, fonts: &FontSet, options: &PdfOptions) -> Result<Vec<u8>> {
    let flat: Vec<Vec<Draw>> = doc
        .pages
        .iter()
        .map(|p| {
            let mut v = Vec::with_capacity(p.items.len());
            flatten(&p.items, &mut v);
            v
        })
        .collect();
    // 1. Which glyphs of which faces are used (and which characters they stand for).
    let mut used: BTreeMap<FaceId, BTreeMap<u16, char>> = BTreeMap::new();
    for page in &flat {
        for draw in page {
            if let Draw::Item(PrintItem::Text(t)) = draw {
                let face = fonts.face(t.face);
                let entry = used.entry(t.face).or_default();
                for line in &t.lines {
                    for c in line.text.chars() {
                        entry.entry(face.glyph(c)).or_insert(c);
                    }
                }
            }
        }
    }

    let mut ids = Ids { next: 1 };
    let catalog_id = ids.next();
    let tree_id = ids.next();
    let info_id = ids.next();
    let mut pdf = Pdf::new();

    // 2. Fonts.
    let mut remappers: BTreeMap<FaceId, (GlyphRemapper, String)> = BTreeMap::new();
    let mut font_refs: Vec<(String, Ref)> = Vec::new();
    for (i, (&face_id, glyph_map)) in used.iter().enumerate() {
        let face = fonts.face(face_id);
        let mut remapper = GlyphRemapper::new();
        for &g in glyph_map.keys() {
            remapper.remap(g);
        }
        let subset = subsetter::subset(&face.data, 0, &remapper).map_err(|e| Error::Font(format!("{}: {e}", face.family)))?;
        let glyph_set: BTreeSet<u16> = glyph_map.keys().copied().collect();
        let base_name = format!("{}+{}", subset_tag(face_id, &glyph_set), face.postscript_name);
        let resource = format!("F{}", i + 1);

        let type0 = ids.next();
        let cid = ids.next();
        let descriptor = ids.next();
        let cmap_ref = ids.next();
        let file = ids.next();
        let scale = 1000.0 / face.units_per_em;

        pdf.type0_font(type0)
            .base_font(Name(base_name.as_bytes()))
            .encoding_predefined(Name(b"Identity-H"))
            .descendant_font(cid)
            .to_unicode(cmap_ref);

        let system_info = SystemInfo { registry: Str(b"Adobe"), ordering: Str(b"Identity"), supplement: 0 };
        // iText (used by JasperReports) truncates glyph widths to whole units; viewers advance
        // glyphs by these widths, so we do the same to match its output.
        let widths: Vec<f32> = remapper.remapped_gids().map(|old| (face.advance(old) * scale).trunc()).collect();
        let mut cid_font = pdf.cid_font(cid);
        cid_font
            .subtype(CidFontType::Type2)
            .base_font(Name(base_name.as_bytes()))
            .system_info(system_info)
            .font_descriptor(descriptor)
            .default_width(0.0)
            .cid_to_gid_map_predefined(Name(b"Identity"));
        cid_font.widths().consecutive(0, widths);
        cid_font.finish();

        let mut flags = FontFlags::NON_SYMBOLIC;
        if face.family.contains("Serif") {
            flags |= FontFlags::SERIF;
        }
        if face.family.contains("Mono") {
            flags |= FontFlags::FIXED_PITCH;
        }
        if face.italic {
            flags |= FontFlags::ITALIC;
        }
        let b = face.bbox;
        pdf.font_descriptor(descriptor)
            .name(Name(base_name.as_bytes()))
            .flags(flags)
            .bbox(Rect::new(b[0] * scale, b[1] * scale, b[2] * scale, b[3] * scale))
            .italic_angle(face.italic_angle)
            .ascent(face.ascender * scale)
            .descent(-face.descender * scale)
            .cap_height(face.cap_height * scale)
            .stem_v(if face.bold { 120.0 } else { 80.0 })
            .font_file2(file);

        let mut cmap = pdf_writer::types::UnicodeCmap::new(Name(b"Custom"), system_info);
        for (&old, &c) in glyph_map {
            if let Some(new) = remapper.get(old) {
                cmap.pair(new, c);
            }
        }
        pdf.cmap(cmap_ref, &cmap.finish());
        let compressed = deflate(&subset);
        pdf.stream(file, &compressed).filter(Filter::FlateDecode).pair(Name(b"Length1"), subset.len() as i32);

        font_refs.push((resource.clone(), type0));
        remappers.insert(face_id, (remapper, resource));
    }

    // 3. Images, written once each.
    let mut image_refs: BTreeMap<usize, (String, Ref)> = BTreeMap::new();
    for page in &flat {
        for draw in page {
            if let Draw::Item(PrintItem::Image { image, .. }) = draw {
                if image_refs.contains_key(&image.id) {
                    continue;
                }
                let r = ids.next();
                let name = format!("Im{}", image_refs.len() + 1);
                write_image(&mut pdf, &mut ids, r, image);
                image_refs.insert(image.id, (name, r));
            }
        }
    }

    // 4. Pages.
    let mut page_ids = Vec::new();
    let ph = doc.page_height;
    let empty: Vec<Draw> = Vec::new();
    let pages: Vec<&Vec<Draw>> = if flat.is_empty() { vec![&empty] } else { flat.iter().collect() };
    for page in pages {
        let page_id = ids.next();
        let content_id = ids.next();
        page_ids.push(page_id);
        let mut c = Content::new();
        c.set_line_cap(LineCapStyle::ButtCap);
        for draw in page.iter() {
            let item = match draw {
                Draw::Item(item) => *item,
                Draw::Push([a, b, cc, d, e, f]) => {
                    // Local items are written as if on the page; map them into place.
                    c.save_state();
                    c.transform([*a, -*b, -*cc, *d, *cc * ph + *e, ph - *d * ph - *f]);
                    continue;
                }
                Draw::Pop => {
                    c.restore_state();
                    continue;
                }
            };
            match item {
                PrintItem::Group(_) | PrintItem::Transform(..) | PrintItem::Cell(..) | PrintItem::Element(_) => {}
                PrintItem::Path { ops, fill, stroke, dash } => {
                    if ops.is_empty() || (fill.is_none() && stroke.is_none()) {
                        continue;
                    }
                    let y = |v: f64| ph - v as f32;
                    let mut cur = (0.0f64, 0.0f64);
                    for op in ops.iter() {
                        match *op {
                            PathOp::Move(a, b) => {
                                c.move_to(a as f32, y(b));
                                cur = (a, b);
                            }
                            PathOp::Line(a, b) => {
                                c.line_to(a as f32, y(b));
                                cur = (a, b);
                            }
                            PathOp::Quad(qx, qy, ex, ey) => {
                                // Exact cubic form of the quadratic.
                                let (c1x, c1y) = (cur.0 + 2.0 / 3.0 * (qx - cur.0), cur.1 + 2.0 / 3.0 * (qy - cur.1));
                                let (c2x, c2y) = (ex + 2.0 / 3.0 * (qx - ex), ey + 2.0 / 3.0 * (qy - ey));
                                c.cubic_to(c1x as f32, y(c1y), c2x as f32, y(c2y), ex as f32, y(ey));
                                cur = (ex, ey);
                            }
                            PathOp::Cubic(a, b, cc, d, e, f) => {
                                c.cubic_to(a as f32, y(b), cc as f32, y(d), e as f32, y(f));
                                cur = (e, f);
                            }
                            PathOp::Close => {
                                c.close_path();
                            }
                        }
                    }
                    if let Some(color) = fill {
                        let (r, g, b) = rgb(*color);
                        c.set_fill_rgb(r, g, b);
                    }
                    if let Some((color, width)) = stroke {
                        let (r, g, b) = rgb(*color);
                        c.set_stroke_rgb(r, g, b);
                        c.set_line_width(*width);
                        c.set_line_cap(LineCapStyle::ButtCap);
                        match dash {
                            Some(d) => c.set_dash_pattern(*d, 0.0),
                            None => c.set_dash_pattern([], 0.0),
                        };
                    }
                    match (fill.is_some(), stroke.is_some()) {
                        (true, true) => c.fill_nonzero_and_stroke(),
                        (true, false) => c.fill_nonzero(),
                        _ => c.stroke(),
                    };
                }
                PrintItem::Rect { x, y, w, h, fill, stroke, radius, path } => {
                    let (px, py) = (*x, ph - y - h);
                    let shape = |c: &mut Content| {
                        if *radius > 0.0 {
                            rounded_rect(c, px, py, *w, *h, *radius);
                        } else if *path {
                            itext_rect(c, px, py, *w, *h);
                        } else {
                            c.rect(px, py, *w, *h);
                        }
                    };
                    if let Some(color) = fill {
                        let (r, g, b) = rgb(*color);
                        c.set_fill_rgb(r, g, b);
                        shape(&mut c);
                        c.fill_nonzero();
                    }
                    if let Some(pen) = stroke {
                        apply_pen(&mut c, pen, true);
                        shape(&mut c);
                        c.stroke();
                    }
                }
                PrintItem::Shapes { x, y, scale, color, rects, path } => {
                    let (r, g, b) = rgb(*color);
                    c.save_state();
                    c.set_fill_rgb(r, g, b);
                    let s = *scale as f32;
                    c.transform([s, 0.0, 0.0, -s, *x, ph - y]);
                    for [rx, ry, rw, rh] in rects.iter() {
                        c.rect(*rx as f32, *ry as f32, *rw as f32, *rh as f32);
                        c.fill_nonzero();
                    }
                    if !path.is_empty() {
                        for op in path.iter() {
                            match *op {
                                PathOp::Move(a, b) => c.move_to(a as f32, b as f32),
                                PathOp::Line(a, b) => c.line_to(a as f32, b as f32),
                                PathOp::Quad(a, b, x3, y3) => c.cubic_to_initial(a as f32, b as f32, x3 as f32, y3 as f32),
                                PathOp::Cubic(a, b, x2, y2, x3, y3) => {
                                    c.cubic_to(a as f32, b as f32, x2 as f32, y2 as f32, x3 as f32, y3 as f32)
                                }
                                PathOp::Close => c.close_path(),
                            };
                        }
                        c.fill_nonzero();
                    }
                    c.restore_state();
                }
                PrintItem::Ellipse { x, y, w, h, fill, stroke } => {
                    let (px, py) = (*x, ph - y - h);
                    if let Some(color) = fill {
                        let (r, g, b) = rgb(*color);
                        c.set_fill_rgb(r, g, b);
                        ellipse(&mut c, px, py, *w, *h);
                        c.fill_nonzero();
                    }
                    if let Some(pen) = stroke {
                        apply_pen(&mut c, pen, true);
                        ellipse(&mut c, px, py, *w, *h);
                        c.stroke();
                    }
                }
                PrintItem::Image { x, y, w, h, clip, image } => {
                    let Some((name, _)) = image_refs.get(&image.id) else { continue };
                    c.save_state();
                    if let Some([cx, cy, cw, ch]) = clip {
                        c.rect(*cx, ph - cy - ch, *cw, *ch);
                        c.clip_nonzero();
                        c.end_path();
                    }
                    c.transform([*w, 0.0, 0.0, *h, *x, ph - y - h]);
                    c.x_object(Name(name.as_bytes()));
                    c.restore_state();
                }
                PrintItem::Line { x1, y1, x2, y2, pen } => {
                    apply_pen(&mut c, pen, false);
                    if pen.style == LineStyle::Double {
                        // Two thin strokes on either side of the center line.
                        let (dx, dy) = if (y1 - y2).abs() < 1e-3 { (0.0, pen.width / 3.0) } else { (pen.width / 3.0, 0.0) };
                        c.set_line_width(pen.width / 3.0);
                        for s in [-1.0f32, 1.0] {
                            c.move_to(x1 + s * dx, ph - y1 - s * dy);
                            c.line_to(x2 + s * dx, ph - y2 - s * dy);
                        }
                    } else {
                        c.move_to(*x1, ph - y1);
                        c.line_to(*x2, ph - y2);
                    }
                    c.stroke();
                }
                PrintItem::Text(t) => {
                    if t.lines.iter().all(|l| l.text.is_empty()) {
                        continue;
                    }
                    let Some((remapper, resource)) = remappers.get(&t.face) else { continue };
                    let face = fonts.face(t.face);
                    let (r, g, b) = rgb(t.color);
                    c.set_fill_rgb(r, g, b);
                    c.begin_text();
                    c.set_font(Name(resource.as_bytes()), t.size);
                    for line in &t.lines {
                        if line.text.is_empty() {
                            continue;
                        }
                        let encode = |s: &str| -> Vec<u8> {
                            s.chars().flat_map(|ch| remapper.get(face.glyph(ch)).unwrap_or(0).to_be_bytes()).collect()
                        };
                        c.set_text_matrix([1.0, 0.0, 0.0, 1.0, r2(line.x), r2(ph - line.baseline)]);
                        if line.word_spacing.abs() > 1e-4 {
                            // Justified: move each word by the extra spacing after every space
                            // (Tw does not apply to two-byte fonts).
                            let adjust = -line.word_spacing * 1000.0 / t.size;
                            let segments: Vec<&str> = line.text.split_inclusive(' ').collect();
                            let mut positioned = c.show_positioned();
                            let mut items = positioned.items();
                            for (i, seg) in segments.iter().enumerate() {
                                items.show(Str(&encode(seg)));
                                if i + 1 < segments.len() && seg.ends_with(' ') {
                                    items.adjust(adjust);
                                }
                            }
                        } else {
                            c.show(Str(&encode(&line.text)));
                        }
                    }
                    c.end_text();
                    if t.underline || t.strike {
                        // Same geometry as iText/JasperReports: thickness size/18, underline at
                        // size/12 below the baseline, strike-through at size/3 above it.
                        let thick = t.size / 18.0;
                        for line in &t.lines {
                            for (on, offset) in [(t.underline, -t.size / 12.0), (t.strike, t.size / 3.0)] {
                                if on && line.width > 0.0 {
                                    let y = ph - line.baseline + offset;
                                    c.set_stroke_rgb(r, g, b);
                                    c.set_line_width(thick);
                                    c.set_dash_pattern([], 0.0);
                                    c.move_to(line.x, y);
                                    c.line_to(line.x + line.width, y);
                                    c.stroke();
                                }
                            }
                        }
                    }
                }
            }
        }
        let raw = c.finish();
        if options.compress {
            pdf.stream(content_id, &deflate(&raw)).filter(Filter::FlateDecode);
        } else {
            pdf.stream(content_id, &raw);
        }
        let mut p = pdf.page(page_id);
        p.parent(tree_id).media_box(Rect::new(0.0, 0.0, doc.page_width, doc.page_height)).contents(content_id);
        let mut res = p.resources();
        let mut fonts_dict = res.fonts();
        for (name, r) in &font_refs {
            fonts_dict.pair(Name(name.as_bytes()), *r);
        }
        fonts_dict.finish();
        if !image_refs.is_empty() {
            let mut xobjects = res.x_objects();
            for (name, r) in image_refs.values() {
                xobjects.pair(Name(name.as_bytes()), *r);
            }
            xobjects.finish();
        }
        res.finish();
        p.finish();
    }

    pdf.pages(tree_id).kids(page_ids.iter().copied()).count(page_ids.len() as i32);
    pdf.catalog(catalog_id).pages(tree_id);
    let mut info = pdf.document_info(info_id);
    info.producer(TextStr(concat!("ferroreport ", env!("CARGO_PKG_VERSION"))));
    if let Some(t) = &options.title {
        info.title(TextStr(t));
    }
    info.finish();
    Ok(pdf.finish())
}
