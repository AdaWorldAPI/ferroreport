//! Text shaping and bidirectional text, for scripts where one character is not one glyph
//! drawn after the other: Arabic and Hebrew (right to left, Arabic letters joined),
//! Devanagari, Thai and the other Indic and South-East Asian scripts (clusters, reordered
//! vowel signs, stacked marks).
//!
//! Text in simple scripts (Latin, Greek, Cyrillic...) never comes here: it keeps the
//! character-by-character layout that reproduces JasperReports' widths exactly. Text with a
//! right-to-left character or a script that needs shaping is shaped with harfrust (the
//! HarfBuzz project's Rust port of HarfBuzz) run by run, reordered with the Unicode
//! Bidirectional Algorithm, and broken into lines at the Unicode line-break opportunities
//! (UAX #14), measured with the shaped advances.
//!
//! Thai, Lao, Khmer and Myanmar write words without spaces, and finding the words needs a
//! dictionary. Without one, lines break before a Thai or Lao leading vowel (เ แ โ ใ ไ),
//! after a syllable-final sign (ะ ำ ๆ ฯ) and at spaces, and a run still too long for the line
//! is cut between grapheme clusters, never inside one.

use crate::fonts::FaceData;
use unicode_bidi::{BidiClass, Level, ParagraphBidiInfo};
use unicode_script::{Script, UnicodeScript};

/// One glyph of a shaped line, in visual order (left to right on the page).
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct Glyph {
    pub id: u16,
    /// Byte offset, in the line's text, of the first character of the glyph's cluster.
    pub cluster: u32,
    /// Advance in thousandths of an em, truncated to a whole number as iText (JasperReports'
    /// PDF library) measures glyphs: what the pen moves by when the line is drawn.
    pub advance: f32,
    /// The advance in thousandths of an em, not truncated (how AWT measures when filling).
    pub exact: f32,
    /// Offset of the glyph from its pen position, in thousandths of an em (y up).
    pub dx: f32,
    pub dy: f32,
    /// The glyph belongs to a right-to-left run.
    pub rtl: bool,
}

/// A line of text as shaped glyphs, in visual order.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ShapedLine {
    pub glyphs: Vec<Glyph>,
    /// The line's base direction is right to left.
    pub rtl: bool,
    /// Sum of the advances, in thousandths of an em.
    pub advance: f32,
}

impl ShapedLine {
    /// The glyphs of each cluster, with the text of the cluster, in visual order. The text
    /// is what copying the glyphs gives back.
    pub fn clusters<'a>(&'a self, text: &'a str) -> impl Iterator<Item = (&'a str, &'a [Glyph])> + 'a {
        let mut starts: Vec<usize> = self.glyphs.iter().map(|g| g.cluster as usize).collect();
        starts.sort_unstable();
        starts.dedup();
        let end_of = move |c: usize| match starts.binary_search(&c) {
            Ok(i) => starts.get(i + 1).copied().unwrap_or(text.len()),
            Err(_) => text.len(),
        };
        self.glyphs.chunk_by(|a, b| a.cluster == b.cluster).map(move |gs| {
            let start = gs[0].cluster as usize;
            (text.get(start..end_of(start)).unwrap_or(""), gs)
        })
    }
}

/// Whether `text` needs shaping: it has a right-to-left character, a directional control,
/// or a character of a script whose glyphs are not one per character.
#[inline]
pub fn needs_shaping(text: &str) -> bool {
    // ASCII, the common case, is decided a word at a time.
    !text.is_ascii() && has_complex(text)
}

fn has_complex(text: &str) -> bool {
    // Every character below U+0590 (UTF-8 lead byte below 0xD6) is in a simple script.
    !text.bytes().all(|b| b < 0xD6) && text.chars().any(complex)
}

fn complex(c: char) -> bool {
    matches!(c as u32,
        0x0590..=0x08FF // Hebrew, Arabic, Syriac, Thaana, NKo, Samaritan, Mandaic, Arabic Extended
        | 0x0900..=0x0DFF // Devanagari, Bengali, Gurmukhi, Gujarati, Oriya, Tamil, Telugu, Kannada, Malayalam, Sinhala
        | 0x0E00..=0x0FFF // Thai, Lao, Tibetan
        | 0x1000..=0x109F // Myanmar
        | 0x1780..=0x18AF // Khmer, Mongolian
        | 0x1900..=0x1BFF // Limbu, Tai Le, New Tai Lue, Buginese, Tai Tham, Balinese, Sundanese, Batak
        | 0x200E | 0x200F | 0x202A..=0x202E | 0x2066..=0x2069 // directional marks and controls
        | 0xA800..=0xABFF // Syloti Nagri to Meetei Mayek
        | 0xFB1D..=0xFDFF // Hebrew and Arabic presentation forms
        | 0xFE70..=0xFEFC // Arabic presentation forms B
        | 0x10800..=0x10FFF // historic right-to-left scripts, Hanifi Rohingya
        | 0x11000..=0x11FFF // Brahmi and the other historic Indic scripts
        | 0x1E800..=0x1EFFF // Mende Kikakui, Adlam, Arabic mathematical letters
    )
}

/// Whether JasperReports runs `text` right to left: it does when a paragraph of it starts,
/// at its first strongly directional character, right to left (AWT's `TextLayout`
/// measured it so); the whole element then runs right to left.
pub fn is_rtl(text: &str) -> bool {
    if !needs_shaping(text) {
        return false;
    }
    text.split('\n').any(|p| {
        p.chars()
            .find_map(|c| match unicode_bidi::bidi_class(c) {
                BidiClass::L => Some(false),
                BidiClass::R | BidiClass::AL => Some(true),
                _ => None,
            })
            .unwrap_or(false)
    })
}

/// What shaping keeps of a face between texts, made the first time the face shapes: the face
/// as harfrust reads it (which keeps its layout tables, parsed once, inside it), and the shape
/// plans made for it.
#[derive(Default)]
pub(crate) struct ShapingCache {
    font: std::sync::OnceLock<Option<harfrust::Font>>,
    plans: std::sync::Mutex<Vec<(PlanKey, std::sync::Arc<harfrust::ShapePlan>)>>,
}

impl std::fmt::Debug for ShapingCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ShapingCache")
    }
}

/// A shape plan is made for a direction, a script and the features asked.
#[derive(Clone, Copy, PartialEq)]
struct PlanKey {
    rtl: bool,
    script: harfrust::Script,
    simple: bool,
}

/// The features shaping turns off. Like JasperReports, no kerning; and Latin and the other
/// simple scripts inside a shaped line keep one glyph per character (no fi ligature), as
/// everywhere else: a plan for simple scripts takes all three, any other the first.
fn features_off() -> [harfrust::Feature; 3] {
    let off = |t: &[u8; 4]| harfrust::Feature::new(harfrust::Tag::new(t), 0, ..);
    [off(b"kern"), off(b"liga"), off(b"clig")]
}

/// Shapes text in one face.
pub(crate) struct Shaper<'a> {
    font: harfrust::ShaperFont<'a, 'a>,
    cache: &'a ShapingCache,
}

impl<'a> Shaper<'a> {
    pub fn new(face: &'a FaceData) -> Option<Shaper<'a>> {
        let cache = face.shaping_cache();
        let font = cache
            .font
            .get_or_init(|| {
                // The font's bytes are shared with the face, not copied.
                let bytes: std::sync::Arc<dyn AsRef<[u8]> + Send + Sync> = std::sync::Arc::new(face.data.clone());
                harfrust::Font::new(harfrust::font::Blob::Shared(bytes), 0)
            })
            .as_ref()?;
        Some(Shaper { font: harfrust::ShaperFont::new(font), cache })
    }

    /// The plan for shaping in `key`'s direction and script, with `features` (see
    /// [`features_off`]).
    fn plan(&self, key: PlanKey, features: &[harfrust::Feature]) -> std::sync::Arc<harfrust::ShapePlan> {
        let mut plans = self.cache.plans.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((_, plan)) = plans.iter().find(|(k, _)| *k == key) {
            return plan.clone();
        }
        let plan =
            std::sync::Arc::new(harfrust::ShapePlan::new(&self.font, direction(key.rtl), Some(key.script), None, features));
        plans.push((key, plan.clone()));
        plan
    }

    /// Shapes one line (no line breaks in it) with base direction `rtl`: glyphs in visual
    /// order, each with the byte offset of its cluster in `text`.
    pub fn shape_line(&self, text: &str, rtl: bool) -> ShapedLine {
        let mut glyphs = Vec::with_capacity(text.len());
        let level = if rtl { Level::rtl() } else { Level::ltr() };
        if !text.is_empty() {
            let bidi = ParagraphBidiInfo::new(text, Some(level));
            let (levels, runs) = bidi.visual_runs(0..text.len());
            let mut buf = harfrust::Buffer::new();
            for run in runs {
                let run_rtl = levels[run.start].is_rtl();
                let mut items = script_items(text, run.clone());
                if run_rtl {
                    items.reverse();
                }
                for (range, script) in items {
                    self.shape_item(text, range, run_rtl, script, &mut buf, &mut glyphs);
                }
            }
        }
        let advance = glyphs.iter().map(|g| g.advance).sum();
        ShapedLine { glyphs, rtl, advance }
    }

    /// Shapes `range` of `text`, one script in one direction, into `out`, in `buf` (reused
    /// from item to item).
    fn shape_item(
        &self,
        text: &str,
        range: std::ops::Range<usize>,
        rtl: bool,
        script: Script,
        buf: &mut harfrust::Buffer,
        out: &mut Vec<Glyph>,
    ) {
        buf.clear();
        for (i, c) in text[range.clone()].char_indices() {
            buf.push(c as u32, (range.start + i) as u32);
        }
        buf.set_pre_context(&text[..range.start]);
        buf.set_post_context(&text[range.end..]);
        buf.set_direction(direction(rtl));
        let script = script.short_name().parse().unwrap_or(harfrust::Script::COMMON);
        buf.set_script(Some(script));
        let simple = !text[range].chars().any(complex);
        let features = features_off();
        let features = if simple { &features[..] } else { &features[..1] };
        let plan = self.plan(PlanKey { rtl, script, simple }, features);
        // Shaping only fails when the buffer's direction or script is not the plan's, and both
        // come from the same key.
        if harfrust::shape(&self.font, buf, harfrust::ShapeOptions::new().plan(Some(&plan)).features(features)).is_err() {
            return;
        }
        let millis = 1000.0 / self.font.units_per_em() as f32;
        // Right-to-left glyphs come out in visual order, clusters decreasing (harfrust left
        // them in reading order when the plan had features and they had to be reversed here;
        // harfrust does not).
        for (info, pos) in buf.glyph_infos().iter().zip(buf.glyph_positions()) {
            out.push(Glyph {
                id: info.glyph_id as u16,
                cluster: info.cluster,
                advance: (pos.x_advance as f32 * millis).trunc(),
                exact: pos.x_advance as f32 * millis,
                dx: pos.x_offset as f32 * millis,
                dy: pos.y_offset as f32 * millis,
                rtl,
            });
        }
    }
}

fn direction(rtl: bool) -> harfrust::Direction {
    if rtl {
        harfrust::Direction::RightToLeft
    } else {
        harfrust::Direction::LeftToRight
    }
}

/// `text` (one line) shaped in `face` with base direction `rtl`, and its width at `size`
/// points; `None` when the text is in simple scripts and left to right, drawn one glyph per
/// character.
pub fn line(face: &FaceData, text: &str, rtl: bool, size: f32) -> Option<(std::sync::Arc<ShapedLine>, f32)> {
    if !rtl && !needs_shaping(text) {
        return None;
    }
    let shaped = shaped(face, text, rtl)?;
    let width = (shaped.advance as f64 * size as f64 / 1000.0) as f32;
    Some((shaped, width))
}

/// `text` shaped in `face`, remembered for the next few calls: a text is measured when it is
/// filled, measured again when it is exported, then drawn, and is most often one line.
fn shaped(face: &FaceData, text: &str, rtl: bool) -> Option<std::sync::Arc<ShapedLine>> {
    type Memo = Vec<(std::sync::Arc<[u8]>, bool, String, std::sync::Arc<ShapedLine>)>;
    thread_local!(static RECENT: std::cell::RefCell<Memo> = const { std::cell::RefCell::new(Vec::new()) });
    // The entry holds the font's bytes, so the same address is the same font.
    let hit = RECENT.with(|r| {
        r.borrow()
            .iter()
            .find(|(data, r, t, _)| std::sync::Arc::ptr_eq(data, &face.data) && *r == rtl && t == text)
            .map(|e| e.3.clone())
    });
    if hit.is_some() {
        return hit;
    }
    let line = std::sync::Arc::new(Shaper::new(face)?.shape_line(text, rtl));
    RECENT.with(|r| {
        let mut r = r.borrow_mut();
        if r.len() >= 8 {
            r.remove(0);
        }
        r.push((face.data.clone(), rtl, text.to_string(), line.clone()));
    });
    Some(line)
}

/// Splits `range` of `text` into runs of one script each. Characters common to all scripts
/// (spaces, digits, punctuation) and combining marks join the run they sit in.
fn script_items(text: &str, range: std::ops::Range<usize>) -> Vec<(std::ops::Range<usize>, Script)> {
    let mut out: Vec<(std::ops::Range<usize>, Script)> = Vec::new();
    let mut current = Script::Common;
    let mut start = range.start;
    for (i, c) in text[range.clone()].char_indices() {
        let s = c.script();
        if matches!(s, Script::Common | Script::Inherited | Script::Unknown) || s == current {
            continue;
        }
        if current == Script::Common {
            // Leading common characters take the first real script.
            current = s;
            continue;
        }
        out.push((start..range.start + i, current));
        start = range.start + i;
        current = s;
    }
    if start < range.end || out.is_empty() {
        out.push((start..range.end, current));
    }
    out
}

/// Where a line may break in `paragraph` (no newlines): byte offsets, each after a break
/// opportunity, with whether the break is mandatory. The end of the paragraph is not listed.
fn break_opportunities(paragraph: &str) -> Vec<(usize, bool)> {
    let mut out: Vec<(usize, bool)> = unicode_linebreak::linebreaks(paragraph)
        .filter(|&(i, _)| i < paragraph.len())
        .map(|(i, o)| (i, o == unicode_linebreak::BreakOpportunity::Mandatory))
        .collect();
    // Thai and Lao: no dictionary, so break at what surely starts or ends a syllable.
    let chars: Vec<(usize, char)> = paragraph.char_indices().collect();
    let tl = |c: char| matches!(c as u32, 0x0E01..=0x0E5B | 0x0E81..=0x0EDF);
    let mark =
        |c: char| matches!(c as u32, 0x0E31 | 0x0E34..=0x0E3A | 0x0E47..=0x0E4E | 0x0EB1 | 0x0EB4..=0x0EBC | 0x0EC8..=0x0ECE);
    let mut extra = Vec::new();
    for k in 1..chars.len() {
        let (i, c) = chars[k];
        let prev = chars[k - 1].1;
        let leading = matches!(c as u32, 0x0E40..=0x0E44 | 0x0EC0..=0x0EC4);
        let final_sign = matches!(prev as u32, 0x0E30 | 0x0E33 | 0x0E46 | 0x0E2F | 0x0EB0 | 0x0EB3 | 0x0EC6 | 0x0EAF);
        if tl(prev) && tl(c) && !mark(c) && (leading || final_sign) {
            extra.push((i, false));
        }
    }
    if !extra.is_empty() {
        out.extend(extra);
        out.sort_unstable();
        out.dedup_by(|b, a| {
            // Keep one entry per offset, mandatory if either is.
            if a.0 == b.0 {
                a.1 |= b.1;
                true
            } else {
                false
            }
        });
    }
    out
}

/// Advance of every cluster of a shaped paragraph, by byte offset: `prefix[i]` is the
/// advance of the clusters that start before byte `i`, in thousandths of an em. `boundary[i]`
/// says whether a cluster starts at `i` (lines can only break there).
struct Advances {
    prefix: Vec<f64>,
    boundary: Vec<bool>,
}

impl Advances {
    fn of(shaped: &ShapedLine, len: usize, exact: bool) -> Advances {
        let mut at = vec![0f64; len + 1];
        let mut boundary = vec![false; len + 1];
        for g in &shaped.glyphs {
            let c = (g.cluster as usize).min(len);
            at[c] += if exact { g.exact } else { g.advance } as f64;
            boundary[c] = true;
        }
        boundary[len] = true;
        boundary[0] = true;
        let mut prefix = Vec::with_capacity(len + 1);
        let mut sum = 0.0;
        for a in at {
            prefix.push(sum);
            sum += a;
        }
        Advances { prefix, boundary }
    }

    fn width(&self, from: usize, to: usize) -> f64 {
        self.prefix[to] - self.prefix[from]
    }
}

/// Breaks one paragraph (no newlines) into lines no wider than `max_width` points, measured
/// with shaped advances (`exact`, as when filling, or truncated to thousandths of an em, as
/// when exporting). Returns `(start, end)` byte ranges, the end before trailing spaces
/// excluded, and each line's width in points.
pub(crate) fn break_paragraph(
    face: &FaceData,
    paragraph: &str,
    size: f32,
    max_width: f32,
    exact: bool,
) -> Vec<(usize, usize, f32)> {
    let rtl = is_rtl(paragraph);
    let Some(shaped) = shaped(face, paragraph, rtl) else {
        return vec![(0, paragraph.len(), 0.0)];
    };
    let adv = Advances::of(&shaped, paragraph.len(), exact);
    let pts = |millis: f64| (millis * size as f64 / 1000.0) as f32;
    let trim = |from: usize, to: usize| from + paragraph[from..to].trim_end_matches(' ').len();
    let fits = |from: usize, to: usize| pts(adv.width(from, trim(from, to))) <= max_width + 1e-3;
    let mut lines = Vec::new();
    let push = |lines: &mut Vec<(usize, usize, f32)>, from: usize, to: usize| {
        let end = trim(from, to);
        lines.push((from, to, pts(adv.width(from, end))));
    };
    let mut start = 0;
    let mut last_fit: Option<usize> = None;
    let mut opportunities = break_opportunities(paragraph).into_iter().filter(|&(i, _)| adv.boundary[i]).peekable();
    loop {
        let (pos, mandatory) = match opportunities.peek() {
            Some(&o) => o,
            None if start >= paragraph.len() => break,
            None => (paragraph.len(), true),
        };
        if pos <= start {
            opportunities.next();
            continue;
        }
        if fits(start, pos) {
            if mandatory {
                push(&mut lines, start, pos);
                start = pos;
                last_fit = None;
                if pos >= paragraph.len() {
                    break;
                }
            } else {
                last_fit = Some(pos);
            }
            opportunities.next();
            continue;
        }
        if let Some(fit) = last_fit.take() {
            // Break at the last opportunity that fit and measure this one again.
            push(&mut lines, start, fit);
            start = fit;
            continue;
        }
        // Even the first piece is too wide: cut it between clusters, at least one each line.
        let mut cut = start;
        for (i, _) in paragraph[start..pos].char_indices().skip(1) {
            let i = start + i;
            if !adv.boundary[i] {
                continue;
            }
            if !fits(start, i) {
                break;
            }
            cut = i;
        }
        if cut == start {
            cut = (start + 1..=pos).find(|&i| adv.boundary[i] && paragraph.is_char_boundary(i)).unwrap_or(pos);
        }
        push(&mut lines, start, cut);
        start = cut;
    }
    if lines.is_empty() {
        lines.push((0, 0, 0.0));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fonts::FontSet;
    use crate::model::FontSpec;

    fn dejavu() -> (FontSet, usize) {
        let mut fonts = FontSet::new();
        let id = fonts.resolve(&FontSpec {
            name: "DejaVu Sans".into(),
            size: 10.0,
            bold: false,
            italic: false,
            underline: false,
            strike: false,
            pdf_font: None,
        });
        (fonts, id)
    }

    #[test]
    fn simple_scripts_are_not_shaped() {
        assert!(!needs_shaping("Hello, wörld — ¿qué tal? Ελληνικά Кириллица"));
        assert!(needs_shaping("abc שלום"));
        assert!(needs_shaping("مرحبا"));
        assert!(needs_shaping("नमस्ते"));
        assert!(needs_shaping("สวัสดี"));
        assert!(!is_rtl("abc שלום"));
        assert!(is_rtl("שלום abc"));
        assert!(is_rtl("abc\n123 مرحبا"));
    }

    #[test]
    fn arabic_letters_join() {
        let (fonts, id) = dejavu();
        let face = fonts.face(id);
        let shaper = Shaper::new(face).unwrap();
        // Beh alone and beh between two others take different glyphs.
        let alone = shaper.shape_line("ب", true);
        let joined = shaper.shape_line("ببب", true);
        assert_eq!(joined.glyphs.len(), 3);
        assert_ne!(alone.glyphs[0].id, joined.glyphs[1].id);
        assert!(joined.glyphs.iter().all(|g| g.id != face.glyph('ب')));
        // Lam followed by alef is one ligature.
        let lam_alef = shaper.shape_line("لا", true);
        assert_eq!(lam_alef.glyphs.len(), 1);
    }

    #[test]
    fn right_to_left_glyphs_come_in_visual_order() {
        let (fonts, id) = dejavu();
        let shaper = Shaper::new(fonts.face(id)).unwrap();
        // Arabic (kerning off) and Hebrew: left to right on the page is the text backwards.
        for text in ["مرحبا بالعالم", "שלום עולם"] {
            let line = shaper.shape_line(text, true);
            assert!(line.glyphs.windows(2).all(|w| w[0].cluster >= w[1].cluster), "{text}: {:?}", line.glyphs);
            assert_eq!(line.glyphs.first().map(|g| g.cluster as usize), text.char_indices().last().map(|(i, _)| i));
        }
    }

    #[test]
    fn right_to_left_with_numbers_and_latin() {
        let (fonts, id) = dejavu();
        let face = fonts.face(id);
        let shaper = Shaper::new(face).unwrap();
        let text = "שלום 123 abc";
        let line = shaper.shape_line(text, true);
        // Visual order, left to right: "abc", " ", "123", " ", then the Hebrew word reversed.
        let order: Vec<&str> = line.clusters(text).map(|(s, _)| s).collect();
        assert_eq!(order.concat(), "abc 123 םולש");
        // In a left-to-right line the Hebrew word is still reversed, the rest stays.
        let text = "ab שלום cd";
        let line = shaper.shape_line(text, false);
        let order: Vec<&str> = line.clusters(text).map(|(s, _)| s).collect();
        assert_eq!(order.concat(), "ab םולש cd");
    }

    #[test]
    fn parentheses_mirror_in_right_to_left_runs() {
        let (fonts, id) = dejavu();
        let face = fonts.face(id);
        let shaper = Shaper::new(face).unwrap();
        let line = shaper.shape_line("(שלום)", true);
        // The first glyph on the left is the closing parenthesis's mirror: "(".
        assert_eq!(line.glyphs.first().map(|g| g.id), Some(face.glyph('(')));
        assert_eq!(line.glyphs.last().map(|g| g.id), Some(face.glyph(')')));
    }

    #[test]
    fn breaks_at_spaces_with_shaped_widths() {
        let (fonts, id) = dejavu();
        let face = fonts.face(id);
        let text = "هذا نص عربي طويل يلتف على عدة أسطر";
        let lines = break_paragraph(face, text, 12.0, 100.0, false);
        assert!(lines.len() > 1);
        for (from, to, w) in &lines {
            assert!(*w <= 100.0 + 1e-3, "{}", &text[*from..*to]);
            assert!(text.is_char_boundary(*from) && text.is_char_boundary(*to));
        }
        assert_eq!(lines.first().map(|l| l.0), Some(0));
        assert_eq!(lines.last().map(|l| l.1), Some(text.len()));
    }

    #[test]
    fn thai_breaks_without_spaces() {
        let opportunities = break_opportunities("สวัสดีครับเราไปโรงเรียน");
        // Before เ (U+0E40) and before โ (U+0E42).
        assert!(opportunities.len() >= 2, "{opportunities:?}");
    }
}
