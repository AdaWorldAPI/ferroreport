//! PowerPoint export (PPTX), laid out like JasperReports' `JRPptxExporter`: each page of the
//! report is a slide of the report's page size, and each element is a shape where it is on the
//! page.
//!
//! Texts are text boxes with their fonts, sizes, styles, colors, alignment, padding,
//! background and rotation; rectangles, ellipses and lines are shapes; images, charts and
//! barcodes are pictures; frames are rectangles under their elements. A box whose borders are
//! the same on every side is the shape's outline; other borders are drawn as lines (JasperReports
//! leaves them out). Links lead to web pages, other files, and the slides of this document that
//! hold their page or anchor. Slides and pictures are written as the pages are filled.

use crate::fonts::FontSet;
use crate::grid::{self, Node, What};
use crate::model::{BoxStyle, Color, HAlign, LineStyle, Pen, Rotation, VAlign};
use crate::ooxml::{self, Media};
use crate::print::{CellInfo, LinkInfo, LinkTarget, PrintItem, Shape};
use crate::xlsx::escape;
use crate::zip::ZipStream;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Arc;

pub(crate) const MIMETYPE: &str = "application/vnd.openxmlformats-officedocument.presentationml.presentation";

/// Pixels per point of the pictures drawn for charts and barcodes.
const PICTURE_SCALE: f32 = 2.0;

/// English Metric Units in a point.
const EMU: f32 = 12700.0;

/// The smallest and largest slide PowerPoint accepts (1 and 56 inches).
const MIN_SLIDE: i64 = 914_400;
const MAX_SLIDE: i64 = 51_206_400;

const SLIDE_REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide";
const HYPERLINK_REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink";
const LAYOUT_REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout";

fn io(e: std::io::Error) -> crate::Error {
    crate::Error::Io(e.to_string())
}

fn emu(pt: f32) -> i64 {
    (pt * EMU).round() as i64
}

fn hex(c: Color) -> String {
    format!("{:02X}{:02X}{:02X}", c.r, c.g, c.b)
}

/// Where a link of a slide leads, resolved when the document ends (an anchor may be on a later
/// page).
#[derive(PartialEq)]
enum Link {
    External(String),
    Slide(u32),
    Anchor(String),
}

/// The relationships of a slide beyond its layout: pictures and links, by identifier.
#[derive(Default)]
struct SlideRels {
    pictures: Vec<String>,
    links: Vec<Link>,
}

/// Writes a presentation slide by slide.
pub struct PptxStream<W: std::io::Write> {
    zip: ZipStream<W>,
    width: i64,
    height: i64,
    title: String,
    media: Media,
    /// The relationships of every slide written so far.
    slides: Vec<SlideRels>,
    /// The slide each anchor is on (from 1).
    anchors: HashMap<String, u32>,
    /// The links and anchors of the page being written, by the box of their element.
    links: HashMap<(i32, i32, i32, i32), Arc<LinkInfo>>,
    /// The line spacing of the page's texts that are not single-spaced, by text.
    spacing: HashMap<usize, f32>,
    /// The slide being written: its XML, its relationships and its next shape identifier.
    buf: String,
    rels: SlideRels,
    next_id: u32,
}

impl<W: std::io::Write> PptxStream<W> {
    pub fn new(out: W, title: &str, page_width: f32, page_height: f32) -> crate::Result<PptxStream<W>> {
        Ok(PptxStream {
            zip: ZipStream::new(out),
            width: emu(page_width).clamp(MIN_SLIDE, MAX_SLIDE),
            height: emu(page_height).clamp(MIN_SLIDE, MAX_SLIDE),
            title: title.to_string(),
            media: Media::default(),
            slides: Vec::new(),
            anchors: HashMap::new(),
            links: HashMap::new(),
            spacing: HashMap::new(),
            buf: String::new(),
            rels: SlideRels::default(),
            next_id: 2,
        })
    }

    /// Ends the presentation and writes the rest of the package.
    pub fn finish(mut self) -> crate::Result<W> {
        let count = self.slides.len();
        for (i, slide) in self.slides.iter().enumerate() {
            let mut rels = String::from(ooxml::RELATIONSHIPS_HEAD);
            ooxml::relationship(&mut rels, "rIdLayout", LAYOUT_REL, "../slideLayouts/slideLayout1.xml", false);
            for name in &slide.pictures {
                ooxml::relationship(&mut rels, &picture_id(name), ooxml::IMAGE, &format!("../media/{name}"), false);
            }
            for (n, link) in slide.links.iter().enumerate() {
                let id = format!("rIdLink{n}");
                // A link to a page or an anchor the document does not have stays on its slide.
                let here = i as u32 + 1;
                match link {
                    Link::External(target) => ooxml::relationship(&mut rels, &id, HYPERLINK_REL, target, true),
                    Link::Slide(p) => {
                        let p = if (1..=count as u32).contains(p) { *p } else { here };
                        ooxml::relationship(&mut rels, &id, SLIDE_REL, &format!("slide{p}.xml"), false)
                    }
                    Link::Anchor(a) => {
                        let p = self.anchors.get(a).copied().unwrap_or(here);
                        ooxml::relationship(&mut rels, &id, SLIDE_REL, &format!("slide{p}.xml"), false)
                    }
                }
            }
            rels.push_str("\n</Relationships>\n");
            self.zip.add(&format!("ppt/slides/_rels/slide{}.xml.rels", i + 1), rels.as_bytes()).map_err(io)?;
        }

        let mut pres = String::from(PRESENTATION_HEAD);
        if count > 0 {
            pres.push_str("<p:sldIdLst>");
            for i in 0..count {
                let _ = write!(pres, "<p:sldId id=\"{}\" r:id=\"rIdSlide{}\"/>", 256 + i, i + 1);
            }
            pres.push_str("</p:sldIdLst>");
        }
        let _ = writeln!(
            pres,
            "<p:sldSz cx=\"{}\" cy=\"{}\"/><p:notesSz cx=\"6858000\" cy=\"9144000\"/></p:presentation>",
            self.width, self.height
        );
        self.zip.add("ppt/presentation.xml", pres.as_bytes()).map_err(io)?;
        let mut rels = String::from(ooxml::RELATIONSHIPS_HEAD);
        ooxml::relationship(
            &mut rels,
            "rIdMaster",
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster",
            "slideMasters/slideMaster1.xml",
            false,
        );
        ooxml::relationship(
            &mut rels,
            "rIdTheme",
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme",
            "theme/theme1.xml",
            false,
        );
        ooxml::relationship(
            &mut rels,
            "rIdTableStyles",
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/tableStyles",
            "tableStyles.xml",
            false,
        );
        for i in 1..=count {
            ooxml::relationship(&mut rels, &format!("rIdSlide{i}"), SLIDE_REL, &format!("slides/slide{i}.xml"), false);
        }
        rels.push_str("\n</Relationships>\n");
        self.zip.add("ppt/_rels/presentation.xml.rels", rels.as_bytes()).map_err(io)?;

        let master = SLIDE_MASTER.replace("{tree}", EMPTY_TREE);
        self.zip.add("ppt/slideMasters/slideMaster1.xml", master.as_bytes()).map_err(io)?;
        self.zip.add("ppt/slideMasters/_rels/slideMaster1.xml.rels", MASTER_RELS.as_bytes()).map_err(io)?;
        let layout = SLIDE_LAYOUT.replace("{tree}", EMPTY_TREE);
        self.zip.add("ppt/slideLayouts/slideLayout1.xml", layout.as_bytes()).map_err(io)?;
        self.zip.add("ppt/slideLayouts/_rels/slideLayout1.xml.rels", LAYOUT_RELS.as_bytes()).map_err(io)?;
        self.zip.add("ppt/theme/theme1.xml", THEME.as_bytes()).map_err(io)?;
        self.zip.add("ppt/tableStyles.xml", TABLE_STYLES.as_bytes()).map_err(io)?;

        let mut types = String::from(CONTENT_TYPES_HEAD);
        for i in 1..=count {
            let _ = write!(
                types,
                "<Override PartName=\"/ppt/slides/slide{i}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slide+xml\"/>"
            );
        }
        types.push_str("</Types>\n");
        self.zip.add("[Content_Types].xml", types.as_bytes()).map_err(io)?;
        self.zip.add("_rels/.rels", ooxml::root_rels("ppt/presentation.xml").as_bytes()).map_err(io)?;
        self.zip.add("docProps/core.xml", ooxml::core_xml(&self.title).as_bytes()).map_err(io)?;
        let app = APP.replace("{slides}", &count.to_string());
        self.zip.add("docProps/app.xml", app.as_bytes()).map_err(io)?;
        self.zip.finish().map_err(io)
    }

    fn slide_xml(&mut self, items: &[PrintItem], fonts: &FontSet) {
        let nodes = grid::nodes(items);
        let mut found = Vec::new();
        crate::xlsx::links(items, &mut found);
        let page = self.slides.len() as u32 + 1;
        for l in &found {
            if let Some(a) = &l.anchor {
                self.anchors.entry(a.clone()).or_insert(page);
            }
        }
        self.links = found.into_iter().map(|l| ((r(l.x), r(l.y), r(l.w), r(l.h)), l)).collect();
        // Texts are on the page itself or among the items of a frame that follow it.
        self.spacing.clear();
        for item in items {
            if let PrintItem::Cell(c, _) = item {
                if let Some(k) = line_spacing(item, fonts) {
                    self.spacing.insert(Arc::as_ptr(c) as usize, k);
                }
            }
        }
        self.next_id = 2;
        self.rels = SlideRels::default();
        let mut out = std::mem::take(&mut self.buf);
        out.clear();
        out.push_str(SLIDE_HEAD);
        out.push_str(TREE_HEAD);
        self.nodes(&mut out, &nodes, fonts);
        out.push_str("</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>\n");
        self.buf = out;
    }

    fn nodes(&mut self, out: &mut String, nodes: &[Node<'_>], fonts: &FontSet) {
        for node in nodes {
            match &node.what {
                What::Text(t) => self.text(out, node, t),
                What::Shape(e, children, drawn) => {
                    let click = self.click(node);
                    let area = [e.x, e.y, e.w, e.h];
                    match &e.shape {
                        // Its background, its elements, then its borders over them (as the PDF).
                        Shape::Frame => {
                            let boxed = e.boxed.unwrap_or(BoxStyle::plain(e.fore));
                            self.rectangle(out, "Frame", area, e.back, None, 0.0, click.as_deref());
                            self.nodes(out, children, fonts);
                            self.rectangle(out, "Frame", area, None, outline(&boxed), 0.0, None);
                            self.borders(out, area, &boxed);
                        }
                        Shape::Rectangle(pen) => {
                            let (fill, radius) = drawn
                                .iter()
                                .find_map(|i| match i {
                                    PrintItem::Rect { fill, radius, .. } => Some((*fill, *radius)),
                                    _ => None,
                                })
                                .unwrap_or((e.back, 0.0));
                            self.rectangle(out, "Rectangle", area, fill, *pen, radius, click.as_deref());
                        }
                        Shape::Ellipse(pen) => {
                            let fill = drawn
                                .iter()
                                .find_map(|i| match i {
                                    PrintItem::Ellipse { fill, .. } => Some(*fill),
                                    _ => None,
                                })
                                .unwrap_or(e.back);
                            let id = self.id();
                            shape_head(out, id, "Ellipse", click.as_deref());
                            xfrm(out, area, 0, false, false);
                            out.push_str("<a:prstGeom prst=\"ellipse\"><a:avLst/></a:prstGeom>");
                            fill_and_line(out, fill, *pen);
                            out.push_str("</p:spPr>");
                            empty_body(out);
                            out.push_str("</p:sp>");
                        }
                        Shape::Line { pen, .. } => {
                            // Where the PDF draws it (thin lines along the middle of their box).
                            if let Some(PrintItem::Line { x1, y1, x2, y2, pen }) =
                                drawn.iter().find(|i| matches!(i, PrintItem::Line { .. }))
                            {
                                self.line(out, (*x1, *y1, *x2, *y2), pen, click.as_deref());
                            } else if pen.width > 0.0 {
                                self.line(out, (e.x, e.y, e.x + e.w, e.y + e.h), pen, click.as_deref());
                            }
                        }
                        Shape::Image { .. } => {
                            let boxed = e.boxed.unwrap_or(BoxStyle::plain(e.fore));
                            self.rectangle(out, "Image", area, e.back, None, 0.0, None);
                            if let Some(PrintItem::Image { x, y, w, h, clip, image }) =
                                drawn.iter().find(|i| matches!(i, PrintItem::Image { .. }))
                            {
                                if let Some(name) = self.media.image(image) {
                                    self.picture(out, &name, [*x, *y, *w, *h], *clip, click.as_deref());
                                }
                            }
                            self.rectangle(out, "Image", area, None, outline(&boxed), 0.0, None);
                            self.borders(out, area, &boxed);
                        }
                        Shape::Picture { .. } => {
                            // A chart or a barcode: a picture of what the PDF draws there.
                            if e.w > 0.0 && e.h > 0.0 {
                                if let Some(png) = crate::raster::render_png(drawn, fonts, area, PICTURE_SCALE) {
                                    let name = self.media.png(png);
                                    self.picture(out, &name, area, None, click.as_deref());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    fn id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// The `<a:hlinkClick>` of an element with a link.
    fn click(&mut self, node: &Node<'_>) -> Option<String> {
        let link = self.links.get(&(node.x, node.y, node.w, node.h))?.clone();
        let target = link.target.as_ref()?;
        let (to, jump) = match target {
            LinkTarget::Url(url) => (Link::External(url.clone()), false),
            LinkTarget::RemoteAnchor { file, anchor } => (Link::External(format!("{file}#{anchor}")), false),
            LinkTarget::RemotePage { file, page } => (Link::External(format!("{file}#JR_PAGE_ANCHOR_0_{page}")), false),
            LinkTarget::Page(p) => (Link::Slide(*p), true),
            LinkTarget::Anchor(a) => (Link::Anchor(a.clone()), true),
        };
        // Links of a slide that lead to the same place share their relationship.
        let n = match self.rels.links.iter().position(|l| *l == to) {
            Some(n) => n,
            None => {
                self.rels.links.push(to);
                self.rels.links.len() - 1
            }
        };
        let id = format!("rIdLink{n}");
        let mut s = format!("<a:hlinkClick r:id=\"{id}\"");
        if jump {
            s.push_str(" action=\"ppaction://hlinksldjump\"");
        }
        if let Some(t) = &link.tooltip {
            let _ = write!(s, " tooltip=\"{}\"", escape(t));
        }
        s.push_str("/>");
        Some(s)
    }

    #[allow(clippy::too_many_arguments)]
    fn rectangle(
        &mut self,
        out: &mut String,
        name: &str,
        area: [f32; 4],
        fill: Option<Color>,
        pen: Option<Pen>,
        radius: f32,
        click: Option<&str>,
    ) {
        if fill.is_none() && pen.is_none() && click.is_none() {
            return;
        }
        let id = self.id();
        shape_head(out, id, name, click);
        xfrm(out, area, 0, false, false);
        let short = area[2].min(area[3]);
        if radius > 0.0 && short > 0.0 {
            let adj = ((radius / short * 100_000.0).round() as i64).min(50_000);
            let _ = write!(
                out,
                "<a:prstGeom prst=\"roundRect\"><a:avLst><a:gd name=\"adj\" fmla=\"val {adj}\"/></a:avLst></a:prstGeom>"
            );
        } else {
            out.push_str("<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom>");
        }
        fill_and_line(out, fill, pen);
        out.push_str("</p:spPr>");
        empty_body(out);
        out.push_str("</p:sp>");
    }

    /// The borders of a box that are not the same on every side, each as a line.
    fn borders(&mut self, out: &mut String, [x, y, w, h]: [f32; 4], boxed: &BoxStyle) {
        if outline(boxed).is_some() {
            return;
        }
        let sides = [
            (&boxed.top, (x, y, x + w, y)),
            (&boxed.left, (x, y, x, y + h)),
            (&boxed.bottom, (x, y + h, x + w, y + h)),
            (&boxed.right, (x + w, y, x + w, y + h)),
        ];
        for (pen, at) in sides {
            if pen.width > 0.0 {
                self.line(out, at, pen, None);
            }
        }
    }

    fn line(&mut self, out: &mut String, (x1, y1, x2, y2): (f32, f32, f32, f32), pen: &Pen, click: Option<&str>) {
        if pen.width <= 0.0 {
            return;
        }
        let id = self.id();
        let _ = write!(out, "<p:cxnSp><p:nvCxnSpPr><p:cNvPr id=\"{id}\" name=\"Line {id}\">");
        if let Some(c) = click {
            out.push_str(c);
        }
        out.push_str("</p:cNvPr><p:cNvCxnSpPr/><p:nvPr/></p:nvCxnSpPr><p:spPr>");
        // A line from the bottom left to the top right is a box flipped vertically.
        let area = [x1.min(x2), y1.min(y2), (x2 - x1).abs(), (y2 - y1).abs()];
        let flip_h = x2 < x1;
        let flip_v = (y2 < y1) != flip_h;
        xfrm(out, area, 0, flip_h && flip_v, flip_v && !flip_h);
        out.push_str("<a:prstGeom prst=\"line\"><a:avLst/></a:prstGeom>");
        stroke(out, pen);
        out.push_str("</p:spPr></p:cxnSp>");
    }

    /// A picture shown in `at` (x, y, w, h), cut to `clip`.
    fn picture(&mut self, out: &mut String, name: &str, at: [f32; 4], clip: Option<[f32; 4]>, click: Option<&str>) {
        let [x, y, w, h] = at;
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        // What shows of the picture, and how much of it each side cuts (thousandths of a
        // percent of the picture).
        let (shown, cut) = match clip {
            Some([cx, cy, cw, ch]) => {
                let (l, t) = (x.max(cx), y.max(cy));
                let (r, b) = ((x + w).min(cx + cw), (y + h).min(cy + ch));
                if r <= l || b <= t {
                    return;
                }
                let k = |v: f32, size: f32| (v / size * 100_000.0).round() as i64;
                ([l, t, r - l, b - t], [k(l - x, w), k(t - y, h), k(x + w - r, w), k(y + h - b, h)])
            }
            None => (at, [0; 4]),
        };
        if !self.rels.pictures.iter().any(|n| n == name) {
            self.rels.pictures.push(name.to_string());
        }
        let id = self.id();
        let _ = write!(out, "<p:pic><p:nvPicPr><p:cNvPr id=\"{id}\" name=\"Picture {id}\">");
        if let Some(c) = click {
            out.push_str(c);
        }
        let _ = write!(
            out,
            "</p:cNvPr><p:cNvPicPr><a:picLocks noChangeAspect=\"1\"/></p:cNvPicPr><p:nvPr/></p:nvPicPr><p:blipFill><a:blip r:embed=\"{}\"/>",
            picture_id(name)
        );
        if cut != [0; 4] {
            let _ = write!(out, "<a:srcRect l=\"{}\" t=\"{}\" r=\"{}\" b=\"{}\"/>", cut[0], cut[1], cut[2], cut[3]);
        }
        out.push_str("<a:stretch><a:fillRect/></a:stretch></p:blipFill><p:spPr>");
        xfrm(out, shown, 0, false, false);
        out.push_str("<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></p:spPr></p:pic>");
    }

    fn text(&mut self, out: &mut String, node: &Node<'_>, t: &CellInfo) {
        let click = self.click(node);
        let id = self.id();
        shape_head(out, id, "Text", click.as_deref());
        let area = [t.x, t.y, t.w, t.h];
        // A turned text box turns about its center: its unturned box is as wide as the
        // element is tall.
        let (rot, shape_area) = match t.rotation {
            Rotation::None => (0, area),
            Rotation::UpsideDown => (10_800_000, area),
            Rotation::Left | Rotation::Right => {
                let (cx, cy) = (t.x + t.w / 2.0, t.y + t.h / 2.0);
                let rot = if t.rotation == Rotation::Left { 16_200_000 } else { 5_400_000 };
                (rot, [cx - t.h / 2.0, cy - t.w / 2.0, t.h, t.w])
            }
        };
        xfrm(out, shape_area, rot, false, false);
        out.push_str("<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom>");
        fill_and_line(out, t.back, outline(&t.boxed));
        out.push_str("</p:spPr>");
        // The padding, on the sides of the turned box they end up on.
        let p = t.boxed.padding;
        let [l, top, rt, b] = match t.rotation {
            Rotation::None => [p.left, p.top, p.right, p.bottom],
            Rotation::Left => [p.bottom, p.left, p.top, p.right],
            Rotation::Right => [p.top, p.right, p.bottom, p.left],
            Rotation::UpsideDown => [p.right, p.bottom, p.left, p.top],
        };
        let anchor = match t.valign {
            VAlign::Top => "t",
            VAlign::Middle => "ctr",
            VAlign::Bottom => "b",
        };
        let _ = write!(
            out,
            "<p:txBody><a:bodyPr wrap=\"square\" lIns=\"{}\" tIns=\"{}\" rIns=\"{}\" bIns=\"{}\" rtlCol=\"0\" anchor=\"{anchor}\"><a:noAutofit/></a:bodyPr><a:lstStyle/>",
            emu(l),
            emu(top),
            emu(rt),
            emu(b)
        );
        let spacing = self.spacing.get(&(t as *const CellInfo as usize)).copied();
        paragraphs(out, t, spacing);
        out.push_str("</p:txBody></p:sp>");
        self.borders(out, area, &t.boxed);
    }
}

impl<W: std::io::Write> crate::fill::PageSink for PptxStream<W> {
    fn page(&mut self, page: crate::print::PrintPage, fonts: &FontSet) -> crate::Result<()> {
        self.slide_xml(&page.items, fonts);
        let n = self.slides.len() + 1;
        let buf = std::mem::take(&mut self.buf);
        let r = self.zip.add(&format!("ppt/slides/slide{n}.xml"), buf.as_bytes()).map_err(io);
        self.buf = buf;
        r?;
        for (name, bytes) in std::mem::take(&mut self.media.pending) {
            self.zip.add(&format!("ppt/media/{name}"), &bytes).map_err(io)?;
        }
        let rels = std::mem::take(&mut self.rels);
        self.slides.push(rels);
        Ok(())
    }
}

/// Exports a filled document to a PowerPoint presentation.
pub fn write_pptx(doc: &crate::print::PrintDocument, fonts: &FontSet) -> crate::Result<Vec<u8>> {
    use crate::fill::PageSink;
    let mut sink = PptxStream::new(Vec::new(), &doc.name, doc.page_width, doc.page_height)?;
    for page in &doc.pages {
        sink.page(page.clone(), fonts)?;
    }
    sink.finish()
}

fn r(v: f32) -> i32 {
    v.round() as i32
}

fn picture_id(name: &str) -> String {
    format!("rId_{}", name.replace('.', "_"))
}

/// The pen of a box whose four borders are the same (the shape's outline).
fn outline(b: &BoxStyle) -> Option<Pen> {
    (b.top.width > 0.0 && b.top == b.left && b.top == b.bottom && b.top == b.right).then_some(b.top)
}

fn shape_head(out: &mut String, id: u32, name: &str, click: Option<&str>) {
    let _ = write!(out, "<p:sp><p:nvSpPr><p:cNvPr id=\"{id}\" name=\"{name} {id}\">");
    if let Some(c) = click {
        out.push_str(c);
    }
    out.push_str("</p:cNvPr><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr>");
}

fn xfrm(out: &mut String, [x, y, w, h]: [f32; 4], rot: i64, flip_h: bool, flip_v: bool) {
    out.push_str("<a:xfrm");
    if rot != 0 {
        let _ = write!(out, " rot=\"{rot}\"");
    }
    if flip_h {
        out.push_str(" flipH=\"1\"");
    }
    if flip_v {
        out.push_str(" flipV=\"1\"");
    }
    let _ = write!(
        out,
        "><a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"{}\" cy=\"{}\"/></a:xfrm>",
        emu(x),
        emu(y),
        emu(w).max(0),
        emu(h).max(0)
    );
}

fn fill_and_line(out: &mut String, fill: Option<Color>, pen: Option<Pen>) {
    match fill {
        Some(c) => {
            let _ = write!(out, "<a:solidFill><a:srgbClr val=\"{}\"/></a:solidFill>", hex(c));
        }
        None => out.push_str("<a:noFill/>"),
    }
    match pen {
        Some(p) if p.width > 0.0 => stroke(out, &p),
        _ => out.push_str("<a:ln><a:noFill/></a:ln>"),
    }
}

fn stroke(out: &mut String, pen: &Pen) {
    let double = if pen.style == LineStyle::Double { " cmpd=\"dbl\"" } else { "" };
    let _ = write!(
        out,
        "<a:ln w=\"{}\"{double}><a:solidFill><a:srgbClr val=\"{}\"/></a:solidFill>",
        emu(pen.width).max(1),
        hex(pen.color)
    );
    let dash = match pen.style {
        LineStyle::Dashed => "dash",
        LineStyle::Dotted => "sysDot",
        _ => "solid",
    };
    let _ = write!(out, "<a:prstDash val=\"{dash}\"/></a:ln>");
}

/// The body of a shape without text.
fn empty_body(out: &mut String) {
    out.push_str("<p:txBody><a:bodyPr rtlCol=\"0\" anchor=\"ctr\"/><a:lstStyle/><a:p><a:endParaRPr lang=\"en-US\" dirty=\"0\"/></a:p></p:txBody>");
}

/// The line spacing of a text, as a percentage of the font's own (`None`: the font's), from the
/// distance between the first lines the PDF draws.
fn line_spacing(item: &PrintItem, fonts: &FontSet) -> Option<f32> {
    fn first_text(item: &PrintItem) -> Option<&crate::print::PrintText> {
        match item {
            PrintItem::Text(t) if t.lines.len() > 1 => Some(t),
            PrintItem::Cell(_, inner) => first_text(inner),
            PrintItem::Group(items) | PrintItem::Transform(_, items) | PrintItem::Paragraphs(_, items) => {
                items.iter().find_map(first_text)
            }
            _ => None,
        }
    }
    let t = first_text(item)?;
    let pitch = t.lines[1].baseline - t.lines[0].baseline;
    let natural = fonts.face(t.face).line_height(t.size);
    let k = pitch / natural;
    (natural > 0.0 && k.is_finite() && (k - 1.0).abs() > 0.02 && k > 0.0).then_some(k)
}

/// The paragraphs of a text: one per line of its text, each with the runs of its styles.
fn paragraphs(out: &mut String, t: &CellInfo, spacing: Option<f32>) {
    let rtl = crate::shape::is_rtl(&t.text);
    let algn = match t.halign {
        HAlign::Left => "l",
        HAlign::Center => "ctr",
        HAlign::Right => "r",
        HAlign::Justified => "just",
    };
    let mut ppr = format!("<a:pPr algn=\"{algn}\"");
    if rtl {
        ppr.push_str(" rtl=\"1\"");
    }
    ppr.push('>');
    let pct = spacing.map_or(100_000, |k| (k * 100_000.0).round() as i64);
    let _ = write!(ppr, "<a:lnSpc><a:spcPct val=\"{pct}\"/></a:lnSpc><a:spcBef><a:spcPts val=\"0\"/></a:spcBef><a:spcAft><a:spcPts val=\"0\"/></a:spcAft></a:pPr>");
    // The styled pieces of the text.
    let plain = crate::rich::RunStyle { font: t.font.clone(), color: t.color, back: None, script: crate::rich::Script::Normal };
    let pieces: Vec<(&str, &crate::rich::RunStyle)> = if t.runs.is_empty() {
        vec![(t.text.as_str(), &plain)]
    } else {
        t.runs.iter().filter_map(|run| Some((t.text.get(run.start..run.end)?, &run.style))).collect()
    };
    out.push_str("<a:p>");
    out.push_str(&ppr);
    let mut last = &plain;
    for (piece, style) in pieces {
        for (i, line) in piece.split('\n').enumerate() {
            if i > 0 {
                end_paragraph(out, style);
                out.push_str("<a:p>");
                out.push_str(&ppr);
            }
            let line = line.trim_end_matches('\r');
            if !line.is_empty() {
                out.push_str("<a:r>");
                run_props(out, style, "a:rPr");
                let _ = write!(out, "<a:t>{}</a:t></a:r>", escape(line));
            }
        }
        last = style;
    }
    end_paragraph(out, last);
}

fn end_paragraph(out: &mut String, style: &crate::rich::RunStyle) {
    run_props(out, style, "a:endParaRPr");
    out.push_str("</a:p>");
}

fn run_props(out: &mut String, style: &crate::rich::RunStyle, tag: &str) {
    let font = &style.font;
    let size = ((font.size * 100.0).round() as i64).clamp(100, 400_000);
    let _ = write!(out, "<{tag} lang=\"en-US\" sz=\"{size}\" b=\"{}\" i=\"{}\"", font.bold as u8, font.italic as u8);
    if font.underline {
        out.push_str(" u=\"sng\"");
    }
    if font.strike {
        out.push_str(" strike=\"sngStrike\"");
    }
    match style.script {
        crate::rich::Script::Super => out.push_str(" baseline=\"30000\""),
        crate::rich::Script::Sub => out.push_str(" baseline=\"-25000\""),
        crate::rich::Script::Normal => {}
    }
    let _ = write!(out, " dirty=\"0\"><a:solidFill><a:srgbClr val=\"{}\"/></a:solidFill>", hex(style.color));
    if let Some(b) = style.back {
        let _ = write!(out, "<a:highlight><a:srgbClr val=\"{}\"/></a:highlight>", hex(b));
    }
    let name = escape(&font.name);
    let _ = write!(out, "<a:latin typeface=\"{name}\"/><a:ea typeface=\"{name}\"/><a:cs typeface=\"{name}\"/></{tag}>");
}

const PRESENTATION_HEAD: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<p:presentation xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\" saveSubsetFonts=\"1\"><p:sldMasterIdLst><p:sldMasterId id=\"2147483648\" r:id=\"rIdMaster\"/></p:sldMasterIdLst>";

const SLIDE_HEAD: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<p:sld xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\"><p:cSld>";

const TREE_HEAD: &str = "<p:spTree><p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/><a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr>";

const EMPTY_TREE: &str = "<p:spTree><p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/><a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr></p:spTree>";

const SLIDE_MASTER: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<p:sldMaster xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\"><p:cSld><p:bg><p:bgPr><a:solidFill><a:srgbClr val=\"FFFFFF\"/></a:solidFill><a:effectLst/></p:bgPr></p:bg>{tree}</p:cSld><p:clrMap bg1=\"lt1\" tx1=\"dk1\" bg2=\"lt2\" tx2=\"dk2\" accent1=\"accent1\" accent2=\"accent2\" accent3=\"accent3\" accent4=\"accent4\" accent5=\"accent5\" accent6=\"accent6\" hlink=\"hlink\" folHlink=\"folHlink\"/><p:sldLayoutIdLst><p:sldLayoutId id=\"2147483649\" r:id=\"rIdLayout\"/></p:sldLayoutIdLst></p:sldMaster>\n";

const MASTER_RELS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rIdLayout\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout\" Target=\"../slideLayouts/slideLayout1.xml\"/><Relationship Id=\"rIdTheme\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme\" Target=\"../theme/theme1.xml\"/></Relationships>\n";

const SLIDE_LAYOUT: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<p:sldLayout xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\" type=\"blank\" preserve=\"1\"><p:cSld name=\"Blank\">{tree}</p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>\n";

const LAYOUT_RELS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rIdMaster\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster\" Target=\"../slideMasters/slideMaster1.xml\"/></Relationships>\n";

/// A theme of plain colors and the DejaVu fonts (a slide master needs one).
const THEME: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<a:theme xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" name=\"ferroreport\"><a:themeElements><a:clrScheme name=\"ferroreport\"><a:dk1><a:srgbClr val=\"000000\"/></a:dk1><a:lt1><a:srgbClr val=\"FFFFFF\"/></a:lt1><a:dk2><a:srgbClr val=\"333333\"/></a:dk2><a:lt2><a:srgbClr val=\"EEEEEE\"/></a:lt2><a:accent1><a:srgbClr val=\"2B4C7E\"/></a:accent1><a:accent2><a:srgbClr val=\"C0392B\"/></a:accent2><a:accent3><a:srgbClr val=\"1E8449\"/></a:accent3><a:accent4><a:srgbClr val=\"7D3C98\"/></a:accent4><a:accent5><a:srgbClr val=\"2E86C1\"/></a:accent5><a:accent6><a:srgbClr val=\"D68910\"/></a:accent6><a:hlink><a:srgbClr val=\"0000FF\"/></a:hlink><a:folHlink><a:srgbClr val=\"800080\"/></a:folHlink></a:clrScheme><a:fontScheme name=\"ferroreport\"><a:majorFont><a:latin typeface=\"DejaVu Sans\"/><a:ea typeface=\"\"/><a:cs typeface=\"\"/></a:majorFont><a:minorFont><a:latin typeface=\"DejaVu Sans\"/><a:ea typeface=\"\"/><a:cs typeface=\"\"/></a:minorFont></a:fontScheme><a:fmtScheme name=\"ferroreport\"><a:fillStyleLst><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill></a:fillStyleLst><a:lnStyleLst><a:ln w=\"9525\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill></a:ln><a:ln w=\"12700\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill></a:ln><a:ln w=\"19050\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill></a:ln></a:lnStyleLst><a:effectStyleLst><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle></a:effectStyleLst><a:bgFillStyleLst><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill></a:bgFillStyleLst></a:fmtScheme></a:themeElements><a:objectDefaults/><a:extraClrSchemeLst/></a:theme>\n";

const TABLE_STYLES: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<a:tblStyleLst xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" def=\"{5C22544A-7EE6-4342-B048-85BDC9FD1C3A}\"/>\n";

const CONTENT_TYPES_HEAD: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Default Extension=\"png\" ContentType=\"image/png\"/><Default Extension=\"jpg\" ContentType=\"image/jpeg\"/><Override PartName=\"/ppt/presentation.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml\"/><Override PartName=\"/ppt/slideMasters/slideMaster1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml\"/><Override PartName=\"/ppt/slideLayouts/slideLayout1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml\"/><Override PartName=\"/ppt/theme/theme1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.theme+xml\"/><Override PartName=\"/ppt/tableStyles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.tableStyles+xml\"/><Override PartName=\"/docProps/core.xml\" ContentType=\"application/vnd.openxmlformats-package.core-properties+xml\"/><Override PartName=\"/docProps/app.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.extended-properties+xml\"/>";

const APP: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/extended-properties\"><Application>ferroreport</Application><Slides>{slides}</Slides></Properties>\n";

#[cfg(test)]
mod tests {
    fn render(name: &str, rows: Option<serde_json::Value>) -> (Vec<(String, String)>, usize) {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures").join(name);
        let read = |f: &str| std::fs::read_to_string(dir.join(f));
        // A report without data reads no rows.
        let rows =
            rows.unwrap_or_else(|| read("data.json").map(|t| serde_json::from_str(&t).unwrap()).unwrap_or(serde_json::json!([])));
        let params: serde_json::Value =
            read("params.json").map(|t| serde_json::from_str(&t).unwrap()).unwrap_or(serde_json::json!({}));
        let mut r = crate::Renderer::new().resource_dir(&dir);
        let report = r.parse(&read("report.jrxml").unwrap()).unwrap();
        let records = crate::data::records_from_json(&report, &rows).unwrap();
        let p = crate::params_from_json(&report, &params).unwrap();
        let mut pptx = Vec::new();
        let stats = r.render_to(&report, Box::new(records.into_iter().map(Ok)), p, crate::Format::Pptx, &mut pptx).unwrap();
        let parts =
            crate::zip::tests::read(&pptx).into_iter().map(|(n, d)| (n, String::from_utf8_lossy(&d).into_owned())).collect();
        (parts, stats.pages)
    }

    fn part<'a>(parts: &'a [(String, String)], name: &str) -> &'a str {
        parts.iter().find(|(n, _)| n == name).map(|(_, d)| d.as_str()).unwrap_or_else(|| panic!("{name}"))
    }

    /// Every part the package needs, and every relationship of a part leads to a part.
    fn check_package(parts: &[(String, String)], slides: usize) {
        for name in [
            "[Content_Types].xml",
            "_rels/.rels",
            "docProps/core.xml",
            "docProps/app.xml",
            "ppt/presentation.xml",
            "ppt/_rels/presentation.xml.rels",
            "ppt/slideMasters/slideMaster1.xml",
            "ppt/slideLayouts/slideLayout1.xml",
            "ppt/theme/theme1.xml",
        ] {
            part(parts, name);
        }
        let types = part(parts, "[Content_Types].xml");
        let pres = part(parts, "ppt/presentation.xml");
        assert_eq!(pres.matches("<p:sldId ").count(), slides);
        for i in 1..=slides {
            part(parts, &format!("ppt/slides/slide{i}.xml"));
            assert!(types.contains(&format!("PartName=\"/ppt/slides/slide{i}.xml\"")), "slide {i}");
        }
        assert!(!parts.iter().any(|(n, _)| n == &format!("ppt/slides/slide{}.xml", slides + 1)));
        for (name, rels) in parts.iter().filter(|(n, _)| n.ends_with(".rels")) {
            let dir = std::path::Path::new(name).parent().unwrap().parent().unwrap();
            for target in rels.split("Target=\"").skip(1).filter_map(|t| t.split('"').next()) {
                if rels.contains(&format!("Target=\"{target}\" TargetMode=\"External\"")) {
                    continue;
                }
                let mut path = dir.to_path_buf();
                for piece in target.split('/') {
                    match piece {
                        ".." => {
                            path.pop();
                        }
                        p => path.push(p),
                    }
                }
                let path = path.to_string_lossy().replace('\\', "/");
                assert!(parts.iter().any(|(n, _)| *n == path), "{name}: {target}");
            }
        }
    }

    /// A slide per page, of the page's size, with the texts as text boxes (late values like
    /// "page 2 of 5" included) and the shapes and lines of the report.
    #[test]
    fn writes_a_slide_per_page() {
        let (parts, pages) = render("33-page-x-of-y", None);
        assert!(pages > 1);
        check_package(&parts, pages);
        // A4 portrait (595 × 842 points) in EMU.
        assert!(part(&parts, "ppt/presentation.xml").contains("<p:sldSz cx=\"7556500\" cy=\"10693400\"/>"));
        let last = part(&parts, &format!("ppt/slides/slide{pages}.xml"));
        assert!(last.contains(&format!("<a:t>Page {pages} of</a:t>")));
        assert!(
            part(&parts, "ppt/slides/slide1.xml").contains(&format!("<a:t> {pages}</a:t>")),
            "late page totals are in the text"
        );

        let (parts, _) = render("02-invoice", None);
        let slide = part(&parts, "ppt/slides/slide1.xml");
        assert!(slide.contains("<a:t>Ferro Industrial SRL</a:t>"));
        assert!(slide.contains("sz=\"1800\" b=\"1\""), "18pt bold");
        assert!(slide.contains("<a:pPr algn=\"r\">"), "right aligned");
        assert!(slide.contains("<a:latin typeface=\"DejaVu Sans\"/>"));
        // The header's rounded rectangle and its outline.
        assert!(slide.contains("<a:prstGeom prst=\"roundRect\"><a:avLst><a:gd name=\"adj\" fmla=\"val 8571\"/>"));
        assert!(slide.contains("<a:ln w=\"12700\"><a:solidFill><a:srgbClr val=\"C9D3DF\"/>"));

        let (parts, _) = render("08-styles-and-shapes", None);
        let slide = part(&parts, "ppt/slides/slide1.xml");
        assert!(slide.contains("prst=\"ellipse\""));
        assert!(slide.contains("<p:cxnSp>"));
        assert!(slide.contains("flipV=\"1\""), "a line going up");
        assert!(slide.contains("u=\"sng\"") && slide.contains("strike=\"sngStrike\"") && slide.contains("i=\"1\""));

        let (parts, _) = render("23-rotation", None);
        let slide = part(&parts, "ppt/slides/slide1.xml");
        for rot in ["16200000", "5400000", "10800000"] {
            assert!(slide.contains(&format!("<a:xfrm rot=\"{rot}\">")), "{rot}");
        }
    }

    #[test]
    fn a_report_without_pages_is_a_presentation_without_slides() {
        let (parts, pages) = render("02-invoice", Some(serde_json::json!([])));
        assert_eq!(pages, 0);
        check_package(&parts, 0);
        assert!(!part(&parts, "ppt/presentation.xml").contains("sldIdLst"));
    }

    /// Images, charts and barcodes are pictures, each image stored once.
    #[test]
    fn images_charts_and_barcodes_are_pictures() {
        let (parts, pages) = render("15-images", None);
        check_package(&parts, pages);
        let slide = part(&parts, "ppt/slides/slide1.xml");
        let rels = part(&parts, "ppt/slides/_rels/slide1.xml.rels");
        let media: Vec<&str> = parts.iter().map(|(n, _)| n.as_str()).filter(|n| n.starts_with("ppt/media/")).collect();
        // logo.png, photo.jpg, gray.png and the logo in the data (written once though used often).
        assert_eq!(media.len(), 4, "{media:?}");
        assert!(media.iter().any(|n| n.ends_with(".jpg")));
        for m in &media {
            let name = m.trim_start_matches("ppt/media/");
            assert!(rels.contains(&format!("Target=\"../media/{name}\"")), "{name}");
            assert!(slide.contains(&format!("r:embed=\"{}\"", super::picture_id(name))), "{name}");
        }
        assert!(slide.matches("<p:pic>").count() >= 9);
        // A clipped image shows the part of the picture that fits.
        assert!(slide.contains("<a:srcRect "));

        let (parts, _) = render("24-charts", None);
        assert!(part(&parts, "ppt/slides/slide2.xml").matches("<p:pic>").count() >= 6);
        let (parts, _) = render("18-barcodes", None);
        assert!(part(&parts, "ppt/slides/slide1.xml").matches("<p:pic>").count() >= 9);
    }

    /// Links to web pages and other files, and to the slides of pages and anchors of this
    /// document (an anchor may be on a later page).
    #[test]
    fn links_lead_to_pages_files_and_slides() {
        let (parts, pages) = render("46-links", None);
        check_package(&parts, pages);
        let slide = part(&parts, "ppt/slides/slide1.xml");
        let rels = part(&parts, "ppt/slides/_rels/slide1.xml.rels");
        assert!(rels.contains("Target=\"https://github.com/cristianEncarnacion/ferroreport\" TargetMode=\"External\""));
        assert!(rels.contains("Target=\"totals.pdf#Totals\" TargetMode=\"External\""));
        assert!(slide.contains("action=\"ppaction://hlinksldjump\""));
        // "Go to the Este region" and "Go to the Norte region" lead to the slides they start on.
        assert!(rels.contains("relationships/slide\" Target=\"slide2.xml\""));
        assert!(rels.contains("relationships/slide\" Target=\"slide3.xml\""));
    }
}
