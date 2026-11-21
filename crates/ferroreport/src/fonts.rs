//! Font registry and metrics.
//!
//! The DejaVu families that ship with JasperReports (`jasperreports-fonts`) are embedded, so
//! templates designed against them lay out identically. Fonts are parsed once per process
//! and shared between renders. Additional TrueType fonts can be registered per render.

use crate::error::{Error, Result};
use crate::model::FontSpec;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

/// Metrics and glyph tables of one font face.
#[derive(Debug)]
pub struct FaceData {
    pub family: String,
    pub postscript_name: String,
    pub data: Arc<[u8]>,
    pub units_per_em: f32,
    pub ascender: f32,
    pub descender: f32,
    pub line_gap: f32,
    /// OS/2 typographic ascender and descender (both positive), used by iText for backgrounds.
    pub typo_ascender: f32,
    pub typo_descender: f32,
    pub cap_height: f32,
    pub italic_angle: f32,
    pub bbox: [f32; 4],
    pub underline_position: f32,
    pub underline_thickness: f32,
    pub strikeout_position: f32,
    pub strikeout_thickness: f32,
    pub bold: bool,
    pub italic: bool,
    /// Glyph ids of the Basic Multilingual Plane (0 = none), the hot path of text layout.
    bmp: Vec<u16>,
    /// Glyph ids beyond the BMP.
    cmap: HashMap<char, u16>,
    advances: Vec<u16>,
}

impl FaceData {
    pub fn parse(family: &str, data: Arc<[u8]>, bold: bool, italic: bool) -> Result<FaceData> {
        let face = ttf_parser::Face::parse(&data, 0).map_err(|e| Error::Font(format!("{family}: {e}")))?;
        let mut cmap = HashMap::new();
        let mut bmp = vec![0u16; 0x10000];
        if let Some(table) = face.tables().cmap {
            for sub in table.subtables {
                if !sub.is_unicode() {
                    continue;
                }
                sub.codepoints(|cp| {
                    if let (Some(c), Some(g)) = (char::from_u32(cp), sub.glyph_index(cp)) {
                        if cp < 0x10000 {
                            if bmp[cp as usize] == 0 {
                                bmp[cp as usize] = g.0;
                            }
                        } else {
                            cmap.entry(c).or_insert(g.0);
                        }
                    }
                });
            }
        }
        let advances =
            (0..face.number_of_glyphs()).map(|g| face.glyph_hor_advance(ttf_parser::GlyphId(g)).unwrap_or(0)).collect();
        let upem = face.units_per_em() as f32;
        let bb = face.global_bounding_box();
        let ps_name = face
            .names()
            .into_iter()
            .find(|n| n.name_id == ttf_parser::name_id::POST_SCRIPT_NAME && n.is_unicode())
            .and_then(|n| n.to_string())
            .unwrap_or_else(|| family.replace(' ', ""));
        let (ul_pos, ul_th) =
            face.underline_metrics().map(|m| (m.position as f32, m.thickness as f32)).unwrap_or((-0.1 * upem, 0.05 * upem));
        let (st_pos, st_th) =
            face.strikeout_metrics().map(|m| (m.position as f32, m.thickness as f32)).unwrap_or((0.3 * upem, 0.05 * upem));
        Ok(FaceData {
            family: family.to_string(),
            postscript_name: ps_name,
            units_per_em: upem,
            ascender: face.ascender() as f32,
            descender: -(face.descender() as f32),
            line_gap: face.line_gap() as f32,
            typo_ascender: face.typographic_ascender().unwrap_or(face.ascender()) as f32,
            typo_descender: -(face.typographic_descender().unwrap_or(face.descender()) as f32),
            cap_height: face.capital_height().map(|c| c as f32).unwrap_or(0.7 * upem),
            italic_angle: face.italic_angle(),
            bbox: [bb.x_min as f32, bb.y_min as f32, bb.x_max as f32, bb.y_max as f32],
            underline_position: ul_pos,
            underline_thickness: ul_th,
            strikeout_position: st_pos,
            strikeout_thickness: st_th,
            bold,
            italic,
            bmp,
            cmap,
            advances,
            data,
        })
    }

    /// Appends the outline of `c` at `size` units per em, with its origin at `(x, y)` and y
    /// growing downwards.
    pub fn glyph_outline(&self, c: char, size: f64, x: f64, y: f64, out: &mut Vec<crate::print::PathOp>) {
        use crate::print::PathOp;
        struct Builder<'a> {
            out: &'a mut Vec<PathOp>,
            k: f64,
            x: f64,
            y: f64,
        }
        impl Builder<'_> {
            fn p(&self, px: f32, py: f32) -> (f64, f64) {
                (self.x + px as f64 * self.k, self.y - py as f64 * self.k)
            }
        }
        impl ttf_parser::OutlineBuilder for Builder<'_> {
            fn move_to(&mut self, x: f32, y: f32) {
                let (a, b) = self.p(x, y);
                self.out.push(PathOp::Move(a, b));
            }
            fn line_to(&mut self, x: f32, y: f32) {
                let (a, b) = self.p(x, y);
                self.out.push(PathOp::Line(a, b));
            }
            fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
                let (a, b) = self.p(x1, y1);
                let (c, d) = self.p(x, y);
                self.out.push(PathOp::Quad(a, b, c, d));
            }
            fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
                let (a, b) = self.p(x1, y1);
                let (c, d) = self.p(x2, y2);
                let (e, f) = self.p(x, y);
                self.out.push(PathOp::Cubic(a, b, c, d, e, f));
            }
            fn close(&mut self) {
                self.out.push(PathOp::Close);
            }
        }
        let Ok(face) = ttf_parser::Face::parse(&self.data, 0) else { return };
        let mut b = Builder { out, k: size / self.units_per_em as f64, x, y };
        face.outline_glyph(ttf_parser::GlyphId(self.glyph(c)), &mut b);
    }

    /// Right edge of the outline of `c`'s glyph, in font units (0 for empty glyphs).
    pub fn glyph_x_max(&self, c: char) -> f64 {
        let Ok(face) = ttf_parser::Face::parse(&self.data, 0) else { return 0.0 };
        face.glyph_bounding_box(ttf_parser::GlyphId(self.glyph(c))).map(|b| b.x_max as f64).unwrap_or(0.0)
    }

    /// Glyph for `c`, or the `.notdef` glyph.
    pub fn glyph(&self, c: char) -> u16 {
        match self.bmp.get(c as usize) {
            Some(&g) => g,
            None => self.cmap.get(&c).copied().unwrap_or(0),
        }
    }

    pub fn has_glyph(&self, c: char) -> bool {
        self.glyph(c) != 0
    }

    /// Advance of glyph `g` in font units.
    pub fn advance(&self, g: u16) -> f32 {
        self.advances.get(g as usize).copied().unwrap_or(0) as f32
    }

    /// Width of `text` as JasperReports measures it while filling (AWT, fractional metrics,
    /// no kerning). Decides how tall a stretching text gets and where it is cut.
    pub fn fill_width(&self, text: &str, size: f32) -> f32 {
        text.chars().map(|c| self.fill_char_width(c, size)).sum()
    }

    pub fn fill_char_width(&self, c: char, size: f32) -> f32 {
        (self.advance(self.glyph(c)) as f64 * size as f64 / self.units_per_em as f64) as f32
    }

    /// Width of `text` as JasperReports' PDF exporter measures it when drawing: each glyph
    /// advance truncated to whole thousandths of an em (iText font metrics). Decides the
    /// drawn line breaks and the alignment offsets.
    pub fn text_width(&self, text: &str, size: f32) -> f32 {
        let units: i64 = text.chars().map(|c| self.advance_millis(c)).sum();
        (units as f64 * size as f64 / 1000.0) as f32
    }

    pub fn char_width(&self, c: char, size: f32) -> f32 {
        (self.advance_millis(c) as f64 * size as f64 / 1000.0) as f32
    }

    /// Glyph advance in thousandths of an em, truncated.
    fn advance_millis(&self, c: char) -> i64 {
        (self.advance(self.glyph(c)) as f64 * 1000.0 / self.units_per_em as f64) as i64
    }

    /// Distance from the top of a line to its baseline.
    pub fn ascent(&self, size: f32) -> f32 {
        self.ascender * size / self.units_per_em
    }

    pub fn descent(&self, size: f32) -> f32 {
        self.descender * size / self.units_per_em
    }

    /// Typographic ascent and descent at `size` (iText's ASCENT and DESCENT).
    pub fn typo_metrics(&self, size: f32) -> (f32, f32) {
        (self.typo_ascender * size / self.units_per_em, self.typo_descender * size / self.units_per_em)
    }

    /// Single-spaced line height (ascent + descent + leading).
    pub fn line_height(&self, size: f32) -> f32 {
        (self.ascender + self.descender + self.line_gap) * size / self.units_per_em
    }
}

/// Identifies a face inside a [`FontSet`].
pub type FaceId = usize;

macro_rules! dejavu {
    ($file:literal) => {
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/fonts/", $file))
    };
}

/// (family, bold, italic, bytes)
const BUILTIN: &[(&str, bool, bool, &[u8])] = &[
    ("DejaVu Sans", false, false, dejavu!("DejaVuSans.ttf")),
    ("DejaVu Sans", true, false, dejavu!("DejaVuSans-Bold.ttf")),
    ("DejaVu Sans", false, true, dejavu!("DejaVuSans-Oblique.ttf")),
    ("DejaVu Sans", true, true, dejavu!("DejaVuSans-BoldOblique.ttf")),
    ("DejaVu Serif", false, false, dejavu!("DejaVuSerif.ttf")),
    ("DejaVu Serif", true, false, dejavu!("DejaVuSerif-Bold.ttf")),
    ("DejaVu Serif", false, true, dejavu!("DejaVuSerif-Italic.ttf")),
    ("DejaVu Serif", true, true, dejavu!("DejaVuSerif-BoldItalic.ttf")),
    ("DejaVu Sans Mono", false, false, dejavu!("DejaVuSansMono.ttf")),
    ("DejaVu Sans Mono", true, false, dejavu!("DejaVuSansMono-Bold.ttf")),
    ("DejaVu Sans Mono", false, true, dejavu!("DejaVuSansMono-Oblique.ttf")),
    ("DejaVu Sans Mono", true, true, dejavu!("DejaVuSansMono-BoldOblique.ttf")),
];

fn builtin_face(i: usize) -> &'static Arc<FaceData> {
    static CACHE: OnceLock<Vec<OnceLock<Arc<FaceData>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| (0..BUILTIN.len()).map(|_| OnceLock::new()).collect());
    cache[i].get_or_init(|| {
        let (family, bold, italic, bytes) = BUILTIN[i];
        Arc::new(FaceData::parse(family, Arc::from(bytes), bold, italic).expect("bundled font is valid"))
    })
}

/// Maps common family names onto the bundled DejaVu families.
fn builtin_family(name: &str) -> Option<&'static str> {
    let n = name.trim().to_ascii_lowercase();
    Some(match n.as_str() {
        "dejavu sans" | "sansserif" | "sans-serif" | "sans" | "dialog" | "arial" | "helvetica" | "verdana" | "tahoma"
        | "calibri" | "segoe ui" | "liberation sans" | "open sans" | "roboto" => "DejaVu Sans",
        "dejavu serif" | "serif" | "times new roman" | "times" | "times-roman" | "georgia" | "liberation serif" | "cambria" => {
            "DejaVu Serif"
        }
        "dejavu sans mono" | "monospaced" | "monospace" | "courier" | "courier new" | "consolas" | "liberation mono"
        | "dialoginput" => "DejaVu Sans Mono",
        _ => return None,
    })
}

/// Fonts available to one render: the bundled families plus any user-supplied faces.
#[derive(Default)]
pub struct FontSet {
    faces: Vec<Arc<FaceData>>,
    by_key: HashMap<(String, bool, bool), FaceId>,
    /// Families registered by the user (lowercase).
    user_families: std::collections::HashSet<String>,
    resolved: HashMap<(String, bool, bool), FaceId>,
    /// Families that had to be substituted, for diagnostics.
    pub substitutions: Vec<(String, String)>,
}

impl FontSet {
    pub fn new() -> FontSet {
        FontSet::default()
    }

    /// Registers a TrueType/OpenType face under `family`.
    pub fn register(&mut self, family: &str, bold: bool, italic: bool, data: Vec<u8>) -> Result<()> {
        let face = FaceData::parse(family, Arc::from(data), bold, italic)?;
        let id = self.faces.len();
        self.faces.push(Arc::new(face));
        self.by_key.insert((family.to_ascii_lowercase(), bold, italic), id);
        self.user_families.insert(family.to_ascii_lowercase());
        self.resolved.clear();
        Ok(())
    }

    pub fn face(&self, id: FaceId) -> &FaceData {
        &self.faces[id]
    }

    pub fn faces(&self) -> &[Arc<FaceData>] {
        &self.faces
    }

    fn builtin(&mut self, family: &str, bold: bool, italic: bool) -> FaceId {
        let key = (family.to_ascii_lowercase(), bold, italic);
        if let Some(&id) = self.by_key.get(&key) {
            return id;
        }
        let idx = BUILTIN.iter().position(|(f, b, i, _)| *f == family && *b == bold && *i == italic).unwrap_or(0);
        let id = self.faces.len();
        self.faces.push(builtin_face(idx).clone());
        self.by_key.insert(key, id);
        id
    }

    /// Picks the face used for `spec`, substituting a bundled family when needed.
    pub fn resolve(&mut self, spec: &FontSpec) -> FaceId {
        let key = (spec.name.to_ascii_lowercase(), spec.bold, spec.italic);
        if let Some(&id) = self.resolved.get(&key) {
            return id;
        }
        let user_regular =
            if self.user_families.contains(&key.0) { self.by_key.get(&(key.0.clone(), false, false)).copied() } else { None };
        let id = if let Some(&id) = self.by_key.get(&key) {
            id
        } else if let Some(id) = user_regular {
            // User family without the requested style: use its regular face.
            id
        } else {
            let family = match builtin_family(&spec.name) {
                Some(f) => f,
                None => {
                    if !self.substitutions.iter().any(|(from, _)| from == &spec.name) {
                        self.substitutions.push((spec.name.clone(), "DejaVu Sans".into()));
                    }
                    "DejaVu Sans"
                }
            };
            self.builtin(family, spec.bold, spec.italic)
        };
        self.resolved.insert(key, id);
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dejavu_metrics_match_awt() {
        let mut fonts = FontSet::new();
        let spec =
            FontSpec { name: "DejaVu Sans".into(), size: 10.0, bold: false, italic: false, underline: false, strike: false };
        let id = fonts.resolve(&spec);
        let f = fonts.face(id);
        // Values observed in PDFs produced by JasperReports 6.20.
        assert!((f.line_height(10.0) - 11.640625).abs() < 1e-4);
        assert!((f.ascent(10.0) - 9.282_227).abs() < 1e-4);
    }

    #[test]
    fn styles_resolve_after_regular_is_loaded() {
        let mut fonts = FontSet::new();
        let mut spec =
            FontSpec { name: "DejaVu Sans".into(), size: 10.0, bold: false, italic: false, underline: false, strike: false };
        fonts.resolve(&spec);
        spec.bold = true;
        spec.italic = true;
        let id = fonts.resolve(&spec);
        assert!(fonts.face(id).bold && fonts.face(id).italic);
    }

    #[test]
    fn substitutes_unknown_families() {
        let mut fonts = FontSet::new();
        let spec = FontSpec { name: "Comic Sans".into(), size: 10.0, bold: true, italic: false, underline: false, strike: false };
        let id = fonts.resolve(&spec);
        assert_eq!(fonts.face(id).family, "DejaVu Sans");
        assert!(fonts.face(id).bold);
        assert_eq!(fonts.substitutions.len(), 1);
    }
}
