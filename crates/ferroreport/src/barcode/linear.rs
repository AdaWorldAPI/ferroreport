//! One-dimensional barcodes drawn like barcode4j 2.1 beans configured by JasperReports:
//! Code 39, Interleaved 2 of 5, Codabar, EAN-13, EAN-8 and UPC-A. Code 128 lives in
//! [`super::code128`]. Geometry is in millimetres, as in barcode4j's SVG output, and is checked
//! against that output by `tests/barcode4j.rs`.

use super::{pt2mm, r4, Symbol, SymbolText, TextPosition};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Code39,
    Interleaved2Of5,
    Codabar,
    Ean13,
    Ean8,
    UpcA,
}

impl Kind {
    /// The component name in JRXML (`jr:Code39`, `jr:EAN13`, ...).
    pub fn from_component(name: &str) -> Option<Kind> {
        Some(match name {
            "Code39" => Kind::Code39,
            "Interleaved2Of5" => Kind::Interleaved2Of5,
            "Codabar" => Kind::Codabar,
            "EAN13" => Kind::Ean13,
            "EAN8" => Kind::Ean8,
            "UPCA" => Kind::UpcA,
            _ => return None,
        })
    }

    pub fn component(self) -> &'static str {
        match self {
            Kind::Code39 => "Code39",
            Kind::Interleaved2Of5 => "Interleaved2Of5",
            Kind::Codabar => "Codabar",
            Kind::Ean13 => "EAN13",
            Kind::Ean8 => "EAN8",
            Kind::UpcA => "UPCA",
        }
    }

    /// barcode4j's default module width (a `float` constant widened to `double`).
    fn default_module(self) -> f64 {
        match self {
            Kind::Code39 => 0.19f32 as f64,
            Kind::Interleaved2Of5 | Kind::Codabar => 0.21f32 as f64,
            Kind::Ean13 | Kind::Ean8 | Kind::UpcA => 0.33f32 as f64,
        }
    }

    fn default_wide_factor(self) -> f64 {
        match self {
            Kind::Code39 => 2.5,
            _ => 3.0,
        }
    }
}

/// barcode4j's `ChecksumMode`. `Auto` adds the check digit to UPC/EAN codes that lack it and
/// verifies it otherwise; for Code 39 and Interleaved 2 of 5 it means no check character.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Checksum {
    #[default]
    Auto,
    Add,
    Check,
    Ignore,
}

impl Checksum {
    pub fn from_name(name: &str) -> Option<Checksum> {
        Some(match name.trim() {
            "auto" => Checksum::Auto,
            "add" => Checksum::Add,
            "check" => Checksum::Check,
            "ignore" => Checksum::Ignore,
            _ => return None,
        })
    }
}

/// The attributes of a barcode4j component, as JasperReports passes them to the bean.
#[derive(Debug, Clone, PartialEq)]
pub struct Spec {
    pub kind: Kind,
    /// Points; barcode4j's default when absent.
    pub module_width: Option<f64>,
    pub text_position: TextPosition,
    /// Points.
    pub quiet_zone: Option<f64>,
    /// Points.
    pub vertical_quiet_zone: Option<f64>,
    pub checksum: Checksum,
    pub wide_factor: Option<f64>,
    pub display_checksum: bool,
    pub display_start_stop: bool,
    pub extended: bool,
    /// Millimetres: JasperReports hands the attribute to barcode4j without converting it.
    pub intercharacter_gap: Option<f64>,
}

impl Spec {
    pub fn new(kind: Kind) -> Spec {
        Spec {
            kind,
            module_width: None,
            text_position: TextPosition::Bottom,
            quiet_zone: None,
            vertical_quiet_zone: None,
            checksum: Checksum::Auto,
            wide_factor: None,
            display_checksum: false,
            display_start_stop: false,
            extended: false,
            intercharacter_gap: None,
        }
    }
}

/// Lays out `msg` for an element `height_pt` points tall with a human-readable font of
/// `font_size_pt` points (barcode4j's 8 pt when absent).
pub fn symbol(spec: &Spec, msg: &str, height_pt: f64, font_size_pt: Option<f64>) -> Result<Symbol, String> {
    if msg.is_empty() {
        return Err("the barcode message is empty".into());
    }
    let mw = spec.module_width.map(pt2mm).unwrap_or_else(|| spec.kind.default_module());
    let quiet = spec.quiet_zone.map(pt2mm).unwrap_or(10.0 * spec.kind.default_module());
    let font = pt2mm(font_size_pt.unwrap_or(8.0));
    let hrh = if spec.text_position == TextPosition::None { 0.0 } else { font };
    let total = pt2mm(height_pt);
    let vquiet = spec.vertical_quiet_zone.map(pt2mm).unwrap_or(0.0);
    let geo = Geometry { mw, quiet, vquiet, font, hrh, bar_height: total - hrh, position: spec.text_position };
    match spec.kind {
        Kind::Code39 => code39(spec, msg, &geo),
        Kind::Interleaved2Of5 => interleaved(spec, msg, &geo),
        Kind::Codabar => codabar(spec, msg, &geo),
        Kind::Ean13 | Kind::Ean8 | Kind::UpcA => upc_ean(spec, msg, &geo),
    }
}

struct Geometry {
    mw: f64,
    quiet: f64,
    /// Moves the bars of the classic symbologies down without growing the drawing (barcode4j).
    vquiet: f64,
    font: f64,
    /// Height of the human-readable text area (no descender for these symbologies).
    hrh: f64,
    bar_height: f64,
    position: TextPosition,
}

impl Geometry {
    /// Baseline of text whose box ends at `bottom` (barcode4j's `DrawingUtil`).
    fn baseline(&self, bottom: f64) -> f64 {
        bottom - pt2mm(self.font) * 0.2
    }

    fn text(&self, text: String, x: f64, length: Option<f64>) -> SymbolText {
        let bottom = if self.position == TextPosition::Top { self.hrh } else { self.bar_height + self.hrh };
        SymbolText { text, x: r4(x), y: r4(self.baseline(bottom)), size: r4(self.font), length: length.map(r4) }
    }
}

/// Bars and spaces of the classic symbologies (`DefaultCanvasLogicHandler`): one rect per bar,
/// full bar height, text centered under (or over) the bars.
fn classic(elements: &[(bool, f64)], geo: &Geometry, width: f64, text: String) -> Symbol {
    let y = geo.vquiet + if geo.position == TextPosition::Top { geo.hrh } else { 0.0 };
    let mut x = geo.quiet;
    let mut rects = Vec::new();
    for &(bar, w) in elements {
        if bar {
            rects.push([r4(x), r4(y), r4(w), r4(geo.bar_height)]);
        }
        x += w;
    }
    let texts = if geo.position == TextPosition::None {
        Vec::new()
    } else {
        vec![geo.text(text, geo.quiet + (x - geo.quiet) / 2.0, None)]
    };
    Symbol { width: r4(width + 2.0 * geo.quiet), height: r4(geo.bar_height + geo.hrh), rects, texts }
}

/// Narrow/wide element widths from a pattern string ('1' wide, '0' narrow), alternating bar and
/// space starting with a bar.
fn push_pattern(out: &mut Vec<(bool, f64)>, pattern: &str, narrow: f64, wide: f64) {
    for (i, c) in pattern.bytes().enumerate() {
        out.push((i % 2 == 0, if c == b'1' { wide } else { narrow }));
    }
}

// ---------------------------------------------------------------- Code 39

const CODE39_CHARS: &str = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ-. $/+%";
const CODE39: [&str; 43] = [
    "000110100",
    "100100001",
    "001100001",
    "101100000",
    "000110001",
    "100110000",
    "001110000",
    "000100101",
    "100100100",
    "001100100",
    "100001001",
    "001001001",
    "101001000",
    "000011001",
    "100011000",
    "001011000",
    "000001101",
    "100001100",
    "001001100",
    "000011100",
    "100000011",
    "001000011",
    "101000010",
    "000010011",
    "100010010",
    "001010010",
    "000000111",
    "100000110",
    "001000110",
    "000010110",
    "110000001",
    "011000001",
    "111000000",
    "010010001",
    "110010000",
    "011010000",
    "010000101",
    "110000100",
    "011000100",
    "010101000",
    "010100010",
    "010001010",
    "000101010",
];
const CODE39_STAR: &str = "010010100";

/// Full-ASCII Code 39: each character as one or two characters of the basic set.
fn code39_escape(c: char) -> Result<String, String> {
    let v = c as u32;
    let pair = |p: char, base: u32, off: u32| format!("{p}{}", char::from_u32(base + v - off).unwrap_or('?'));
    Ok(match c {
        '0'..='9' | 'A'..='Z' | ' ' | '-' | '.' => c.to_string(),
        '\0' => "%U".into(),
        '\u{1}'..='\u{1a}' => pair('$', 'A' as u32, 1),
        '\u{1b}'..='\u{1f}' => pair('%', 'A' as u32, 27),
        '!'..=',' => pair('/', 'A' as u32, '!' as u32),
        '/' => "/O".into(),
        ':' => "/Z".into(),
        ';'..='?' => pair('%', 'F' as u32, ';' as u32),
        '@' => "%V".into(),
        '['..='_' => pair('%', 'K' as u32, '[' as u32),
        '`' => "%W".into(),
        'a'..='z' => pair('+', 'A' as u32, 'a' as u32),
        '{'..='\u{7f}' => pair('%', 'P' as u32, '{' as u32),
        _ => return Err(format!("Code 39 cannot encode the character {c:?}")),
    })
}

fn code39_value(c: char) -> Result<usize, String> {
    CODE39_CHARS.find(c).ok_or_else(|| format!("Code 39 cannot encode the character {c:?} (enable extendedCharSetEnabled)"))
}

fn code39(spec: &Spec, msg: &str, geo: &Geometry) -> Result<Symbol, String> {
    let escape_all = |s: &str| -> Result<String, String> {
        if spec.extended {
            s.chars().map(code39_escape).collect()
        } else {
            Ok(s.to_ascii_uppercase())
        }
    };
    let mut data = escape_all(msg)?;
    let mut shown = msg.to_string();
    let check = |s: &str| -> Result<char, String> {
        let sum = s.chars().map(code39_value).sum::<Result<usize, _>>()?;
        Ok(CODE39_CHARS.as_bytes()[sum % 43] as char)
    };
    match spec.checksum {
        Checksum::Add => {
            let c = check(&data)?;
            data.push(c);
            // In extended mode the check character belongs to the escaped message, so barcode4j
            // does not show it.
            if spec.display_checksum && !spec.extended {
                shown.push(c);
            }
        }
        Checksum::Check => {
            let (body, last) = data.split_at(data.len() - 1);
            let expected = check(body)?;
            if !last.starts_with(expected) {
                return Err(format!("Message '{msg}' has a bad checksum. Expected: {expected}"));
            }
        }
        Checksum::Auto | Checksum::Ignore => {}
    }
    if spec.display_start_stop {
        shown = format!("*{shown}*");
    }
    let mw = geo.mw;
    let wide = mw * spec.wide_factor.unwrap_or_else(|| spec.kind.default_wide_factor());
    // The gap defaults to the default module width, even when the module width is changed.
    let gap = spec.intercharacter_gap.unwrap_or_else(|| spec.kind.default_module());
    let mut el = Vec::new();
    let chars: Vec<&str> = std::iter::once(Ok(CODE39_STAR))
        .chain(data.chars().map(|c| code39_value(c).map(|v| CODE39[v])))
        .chain(std::iter::once(Ok(CODE39_STAR)))
        .collect::<Result<_, _>>()?;
    for (i, p) in chars.iter().enumerate() {
        if i > 0 {
            el.push((false, gap));
        }
        push_pattern(&mut el, p, mw, wide);
    }
    // barcode4j sizes the drawing from the message escaped a second time (a quirk of its
    // extended mode), so extended symbols get a wider drawing than their bars.
    let sized = if spec.extended { escape_all(&data)?.chars().count() } else { data.chars().count() } + 2;
    let char_width = (3.0 * (wide / mw) + 6.0) * mw;
    let width = sized as f64 * char_width + (sized - 1) as f64 * gap;
    Ok(classic(&el, geo, width, shown))
}

// ---------------------------------------------------------------- Interleaved 2 of 5

const I25: [&str; 10] = ["00110", "10001", "01001", "11000", "00101", "10100", "01100", "00011", "10010", "01010"];

/// UPC/EAN check digit: weights 3, 1, 3, ... from the rightmost digit.
fn mod10(digits: &[u8]) -> u8 {
    let sum: u32 = digits.iter().rev().enumerate().map(|(i, &d)| d as u32 * if i % 2 == 0 { 3 } else { 1 }).sum();
    ((10 - sum % 10) % 10) as u8
}

/// Interleaved 2 of 5 check digit as barcode4j computes it: weights 3, 1, 3, ... from the left.
fn mod10_left(digits: &[u8]) -> u8 {
    let sum: u32 = digits.iter().enumerate().map(|(i, &d)| d as u32 * if i % 2 == 0 { 3 } else { 1 }).sum();
    ((10 - sum % 10) % 10) as u8
}

fn digits(msg: &str, what: &str) -> Result<Vec<u8>, String> {
    msg.bytes()
        .map(|b| if b.is_ascii_digit() { Ok(b - b'0') } else { Err(format!("{what} accepts only digits, got {:?}", b as char)) })
        .collect()
}

fn interleaved(spec: &Spec, msg: &str, geo: &Geometry) -> Result<Symbol, String> {
    let mut d = digits(msg, "Interleaved 2 of 5")?;
    let mut shown = msg.to_string();
    match spec.checksum {
        Checksum::Add => {
            let c = mod10_left(&d);
            d.push(c);
            if spec.display_checksum {
                shown.push((b'0' + c) as char);
            }
        }
        Checksum::Check => {
            let (body, last) = d.split_at(d.len() - 1);
            if last[0] != mod10_left(body) {
                return Err(format!("Message '{msg}' has a bad checksum. Expected: {}", mod10_left(body)));
            }
        }
        Checksum::Auto | Checksum::Ignore => {}
    }
    if d.len() % 2 == 1 {
        d.insert(0, 0);
    }
    let mw = geo.mw;
    let wf = spec.wide_factor.unwrap_or_else(|| spec.kind.default_wide_factor());
    let wide = mw * wf;
    let mut el = Vec::new();
    push_pattern(&mut el, "0000", mw, wide);
    for pair in d.chunks(2) {
        let (a, b) = (I25[pair[0] as usize].as_bytes(), I25[pair[1] as usize].as_bytes());
        for k in 0..5 {
            el.push((true, if a[k] == b'1' { wide } else { mw }));
            el.push((false, if b[k] == b'1' { wide } else { mw }));
        }
    }
    push_pattern(&mut el, "100", mw, wide);
    let width = (d.len() as f64 * (2.0 * wf + 3.0) + 6.0 + wf) * mw;
    Ok(classic(&el, geo, width, shown))
}

// ---------------------------------------------------------------- Codabar

const CODABAR_CHARS: &str = "0123456789-$:/.+abcd";
const CODABAR: [&str; 20] = [
    "0000011", "0000110", "0001001", "1100000", "0010010", "1000010", "0100001", "0100100", "0110000", "1001000", "0001100",
    "0011000", "1000101", "1010001", "1010100", "0010101", "0011010", "0101001", "0001011", "0001110",
];

fn codabar_index(c: char) -> Result<usize, String> {
    let c = match c {
        't' => 'a',
        'n' => 'b',
        '*' => 'c',
        'e' => 'd',
        c => c,
    };
    CODABAR_CHARS.find(c).ok_or_else(|| format!("Codabar cannot encode the character {c:?} (start/stop characters are a-d)"))
}

fn codabar(spec: &Spec, msg: &str, geo: &Geometry) -> Result<Symbol, String> {
    let mw = geo.mw;
    let wide = mw * spec.wide_factor.unwrap_or_else(|| spec.kind.default_wide_factor());
    let mut el = Vec::new();
    for (i, c) in msg.chars().enumerate() {
        if i > 0 {
            el.push((false, mw));
        }
        push_pattern(&mut el, CODABAR[codabar_index(c)?], mw, wide);
    }
    let is_start_stop = |c: char| "abcdtn*e".contains(c);
    let mut shown: &str = msg;
    if !spec.display_start_stop && msg.chars().count() >= 2 {
        let (first, last) = (msg.chars().next().unwrap_or(' '), msg.chars().last().unwrap_or(' '));
        if is_start_stop(first) && is_start_stop(last) {
            shown = &msg[first.len_utf8()..msg.len() - last.len_utf8()];
        }
    }
    let width: f64 = el.iter().map(|e| e.1).sum();
    Ok(classic(&el, geo, width, shown.to_string()))
}

// ---------------------------------------------------------------- UPC / EAN

const UPC_L: [&str; 10] =
    ["0001101", "0011001", "0010011", "0111101", "0100011", "0110001", "0101111", "0111011", "0110111", "0001011"];
const EAN13_PARITY: [&str; 10] =
    ["LLLLLL", "LLGLGG", "LLGGLG", "LLGGGL", "LGLLGG", "LGGLLG", "LGGGLL", "LGLGLG", "LGLGGL", "LGGLGL"];

fn upc_code(digit: u8, set: char) -> String {
    let l = UPC_L[digit as usize];
    match set {
        'L' => l.to_string(),
        // R is L inverted; G is R reversed.
        'R' => l.chars().map(|c| if c == '1' { '0' } else { '1' }).collect(),
        _ => l.chars().rev().map(|c| if c == '1' { '0' } else { '1' }).collect(),
    }
}

/// A human-readable digit group: first module, module count, digits.
type Group = (usize, usize, String);
/// Runs, groups, and the digits printed outside the bars (left, right).
type UpcLayout = (Vec<Run>, Vec<Group>, Option<u8>, Option<u8>);

/// Modules of one group, with whether its bars are drawn long (guards, and the first and last
/// digits of UPC-A) or short (digits under the human-readable text).
struct Run {
    modules: String,
    long: bool,
}

fn upc_ean(spec: &Spec, msg: &str, geo: &Geometry) -> Result<Symbol, String> {
    let (name, body_len) = match spec.kind {
        Kind::Ean13 => ("EAN-13", 12),
        Kind::Ean8 => ("EAN-8", 7),
        _ => ("UPC-A", 11),
    };
    if msg.contains('+') {
        return Err(format!("{name} supplementals (+2 and +5 add-ons) are not supported yet"));
    }
    let mut d = digits(msg, name)?;
    match (spec.checksum, d.len()) {
        (Checksum::Auto | Checksum::Add, n) if n == body_len => d.push(mod10(&d)),
        (Checksum::Auto | Checksum::Check, n) if n == body_len + 1 => {
            let expected = mod10(&d[..body_len]);
            if d[body_len] != expected {
                return Err(format!("Message '{msg}' has a bad checksum. Expected: {expected}"));
            }
        }
        (Checksum::Ignore, n) if n == body_len + 1 => {}
        _ => return Err(format!("{name} needs {body_len} digits, or {} with the check digit", body_len + 1)),
    }
    let enc =
        |ds: &[u8], set: &dyn Fn(usize) -> char| -> String { ds.iter().enumerate().map(|(i, &x)| upc_code(x, set(i))).collect() };
    let guard = |s: &str| Run { modules: s.into(), long: true };
    let group = |s: String| Run { modules: s, long: false };
    let mw = geo.mw;
    // Runs, and the human-readable groups as (first module, module count, text).
    let (runs, groups, lead, trail): UpcLayout = match spec.kind {
        Kind::Ean13 => {
            let parity = EAN13_PARITY[d[0] as usize].as_bytes();
            let left = enc(&d[1..7], &|i| parity[i] as char);
            let right = enc(&d[7..13], &|_| 'R');
            let text = |r: std::ops::Range<usize>| d[r].iter().map(|x| (b'0' + x) as char).collect::<String>();
            (
                vec![guard("101"), group(left), guard("01010"), group(right), guard("101")],
                vec![(3, 42, text(1..7)), (50, 42, text(7..13))],
                Some(d[0]),
                None,
            )
        }
        Kind::Ean8 => {
            let text = |r: std::ops::Range<usize>| d[r].iter().map(|x| (b'0' + x) as char).collect::<String>();
            (
                vec![guard("101"), group(enc(&d[0..4], &|_| 'L')), guard("01010"), group(enc(&d[4..8], &|_| 'R')), guard("101")],
                vec![(3, 28, text(0..4)), (36, 28, text(4..8))],
                None,
                None,
            )
        }
        _ => {
            let text = |r: std::ops::Range<usize>| d[r].iter().map(|x| (b'0' + x) as char).collect::<String>();
            (
                vec![
                    guard("101"),
                    Run { modules: upc_code(d[0], 'L'), long: true },
                    group(enc(&d[1..6], &|_| 'L')),
                    guard("01010"),
                    group(enc(&d[6..11], &|_| 'R')),
                    Run { modules: upc_code(d[11], 'R'), long: true },
                    guard("101"),
                ],
                vec![(10, 35, text(1..6)), (50, 35, text(6..11))],
                Some(d[0]),
                Some(d[11]),
            )
        }
    };

    let half = geo.hrh / 2.0;
    let (long_y, long_h, short_y) = match geo.position {
        TextPosition::None => (0.0, geo.bar_height, 0.0),
        TextPosition::Top => (half, geo.bar_height + half, geo.hrh),
        TextPosition::Bottom => (0.0, geo.bar_height + half, 0.0),
    };
    let mut rects = Vec::new();
    let mut x = geo.quiet;
    for run in &runs {
        let m = run.modules.as_bytes();
        let mut i = 0;
        while i < m.len() {
            let mut j = i;
            while j < m.len() && m[j] == m[i] {
                j += 1;
            }
            let w = mw * (j - i) as f64;
            if m[i] == b'1' {
                let (y, h) = if run.long { (long_y, long_h) } else { (short_y, geo.bar_height) };
                rects.push([r4(x), r4(y), r4(w), r4(h)]);
            }
            x += w;
            i = j;
        }
    }
    let modules: usize = runs.iter().map(|r| r.modules.len()).sum();
    let mut texts = Vec::new();
    if geo.position != TextPosition::None {
        let start = geo.quiet;
        let digit = |v: u8| ((b'0' + v) as char).to_string();
        if let Some(v) = lead {
            texts.push(geo.text(digit(v), start - 6.5 * mw, None));
        }
        for (first, count, text) in groups {
            let gx = start + first as f64 * mw;
            let gw = count as f64 * mw;
            texts.push(geo.text(text, gx + gw / 2.0, Some(gw - 2.0 * mw)));
        }
        if let Some(v) = trail {
            texts.push(geo.text(digit(v), start + modules as f64 * mw + 6.5 * mw, None));
        }
    }
    Ok(Symbol { width: r4(modules as f64 * mw + 2.0 * geo.quiet), height: r4(geo.bar_height + geo.hrh), rects, texts })
}
