//! Postal barcodes drawn like barcode4j: POSTNET, Royal Mail 4-state (RM4SCC) and the USPS
//! Intelligent Mail barcode. Bars are laid out in millimetres.

use super::{pt2mm, r4, Symbol};

/// A bar of a four-state code.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Bar {
    Track,
    Ascender,
    Descender,
    Full,
}

/// Four-state geometry (`ascenderHeight`, `trackHeight`, bar width and gap, quiet zones).
pub struct FourState {
    pub bar: f64,
    pub gap: f64,
    pub track: f64,
    pub ascender: f64,
    pub quiet: f64,
    pub vquiet: f64,
    /// The bars are centered in the element's height. JasperReports sets the height before
    /// the ascender and track heights, so when those are given the drawing is only as tall as
    /// the bars.
    pub fit_height: bool,
}

fn four_state(bars: &[Bar], g: &FourState, height_pt: f64) -> Symbol {
    let full = g.track + 2.0 * g.ascender;
    let h = if g.fit_height { pt2mm(height_pt) } else { full };
    let top = g.vquiet + (h - full) / 2.0;
    let track_top = top + g.ascender;
    let mut rects = Vec::new();
    for (i, b) in bars.iter().enumerate() {
        let x = g.quiet + i as f64 * (g.bar + g.gap);
        let (y, bh) = match b {
            Bar::Track => (track_top, g.track),
            Bar::Ascender => (top, g.ascender + g.track),
            Bar::Descender => (track_top, g.track + g.ascender),
            Bar::Full => (top, full),
        };
        rects.push([r4(x), r4(y), r4(g.bar), r4(bh)]);
    }
    let width = 2.0 * g.quiet + bars.len() as f64 * (g.bar + g.gap) - g.gap;
    Symbol { width: r4(width), height: r4(h + 2.0 * g.vquiet), rects, texts: Vec::new() }
}

// ---------------------------------------------------------------- Royal Mail (RM4SCC)

const RM_CHARS: &str = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";

/// Which of a character's four bars rise (for its row) or fall (for its column), 1-based.
fn rm_pattern(v: usize) -> [bool; 4] {
    match v {
        1 => [false, false, true, true],
        2 => [false, true, false, true],
        3 => [false, true, true, false],
        4 => [true, false, false, true],
        5 => [true, false, true, false],
        _ => [true, true, false, false],
    }
}

/// barcode4j's `RoyalMailCBCBean`: start bar, the characters, a check character, stop bar.
pub fn royal_mail(msg: &str, add_checksum: bool, g: &FourState, height_pt: f64) -> Result<Symbol, String> {
    let mut chars: Vec<usize> = Vec::new();
    for c in msg.to_ascii_uppercase().chars() {
        match RM_CHARS.find(c) {
            Some(i) => chars.push(i),
            None => return Err(format!("Royal Mail 4-state cannot encode {c:?}")),
        }
    }
    if add_checksum {
        let (rows, cols): (usize, usize) = chars.iter().fold((0, 0), |(r, c), &i| (r + i / 6 + 1, c + i % 6 + 1));
        let (r, c) = ((rows - 1) % 6, (cols - 1) % 6);
        chars.push(r * 6 + c);
    }
    let mut bars = vec![Bar::Ascender];
    for i in chars {
        let (up, down) = (rm_pattern(i / 6 + 1), rm_pattern(i % 6 + 1));
        for k in 0..4 {
            bars.push(match (up[k], down[k]) {
                (true, true) => Bar::Full,
                (true, false) => Bar::Ascender,
                (false, true) => Bar::Descender,
                _ => Bar::Track,
            });
        }
    }
    bars.push(Bar::Full);
    Ok(four_state(&bars, g, height_pt))
}

// ---------------------------------------------------------------- USPS Intelligent Mail

/// The bars of the code (USPS-B-3200): descender character and bit, ascender character and bit.
const BAR_TABLE: [(u8, u8, u8, u8); 65] = [
    (7, 2, 4, 3),
    (1, 10, 0, 0),
    (9, 12, 2, 8),
    (5, 5, 6, 11),
    (8, 9, 3, 1),
    (0, 1, 5, 12),
    (2, 5, 1, 8),
    (4, 4, 9, 11),
    (6, 3, 8, 10),
    (3, 9, 7, 6),
    (5, 11, 1, 4),
    (8, 5, 2, 12),
    (9, 10, 0, 2),
    (7, 1, 6, 7),
    (3, 6, 4, 9),
    (0, 3, 8, 6),
    (6, 4, 2, 7),
    (1, 1, 9, 9),
    (7, 10, 5, 2),
    (4, 0, 3, 8),
    (6, 2, 0, 4),
    (8, 11, 1, 0),
    (9, 8, 3, 12),
    (2, 6, 7, 7),
    (5, 1, 4, 10),
    (1, 12, 6, 9),
    (7, 3, 8, 0),
    (5, 8, 9, 7),
    (4, 6, 2, 10),
    (3, 4, 0, 5),
    (8, 4, 5, 7),
    (7, 11, 1, 9),
    (6, 0, 9, 6),
    (0, 6, 4, 8),
    (2, 1, 3, 2),
    (5, 9, 8, 12),
    (4, 11, 6, 1),
    (9, 5, 7, 4),
    (3, 3, 1, 2),
    (0, 7, 2, 0),
    (1, 3, 4, 1),
    (6, 10, 3, 5),
    (8, 7, 9, 4),
    (2, 11, 5, 6),
    (0, 8, 7, 12),
    (4, 2, 8, 1),
    (5, 10, 3, 0),
    (9, 3, 0, 9),
    (6, 5, 2, 4),
    (7, 8, 1, 7),
    (5, 0, 4, 5),
    (2, 3, 0, 10),
    (6, 12, 9, 2),
    (3, 11, 1, 6),
    (8, 8, 7, 9),
    (5, 4, 0, 11),
    (1, 5, 2, 2),
    (9, 1, 4, 12),
    (8, 3, 6, 6),
    (7, 0, 3, 7),
    (4, 7, 7, 5),
    (0, 12, 1, 11),
    (2, 9, 9, 0),
    (6, 8, 5, 3),
    (3, 10, 8, 2),
];

fn nof13(n: u32, len: usize) -> Vec<u16> {
    let mut table = vec![0u16; len];
    let (mut lower, mut upper) = (0usize, len - 1);
    for count in 0u16..8192 {
        if count.count_ones() != n {
            continue;
        }
        let reverse = count.reverse_bits() >> 3;
        if reverse < count {
            continue;
        }
        if count == reverse {
            table[upper] = count;
            upper = upper.saturating_sub(1);
        } else {
            table[lower] = count;
            table[lower + 1] = reverse;
            lower += 2;
        }
    }
    table
}

/// barcode4j's `USPSIntelligentMailBean`: a 20-digit tracking code and a 0, 5, 9 or 11-digit
/// routing code (spaces and dashes are ignored).
pub fn usps(msg: &str, g: &FourState, height_pt: f64) -> Result<Symbol, String> {
    let digits: Vec<u8> = msg.chars().filter(|c| c.is_ascii_digit()).map(|c| c as u8 - b'0').collect();
    if msg.chars().any(|c| !c.is_ascii_digit() && c != ' ' && c != '-') {
        return Err("the Intelligent Mail barcode takes digits only".into());
    }
    if !matches!(digits.len(), 20 | 25 | 29 | 31) || digits[1] > 4 {
        return Err("the Intelligent Mail barcode needs a 20-digit tracking code and a 0, 5, 9 or 11-digit routing code".into());
    }
    let (tracking, routing) = digits.split_at(20);
    // The value, as a 104-bit number.
    let mut v: u128 = routing.iter().fold(0u128, |a, &d| a * 10 + d as u128);
    v += match routing.len() {
        5 => 1,
        9 => 100_001,
        11 => 1_000_100_001,
        _ => 0,
    };
    v = v * 10 + tracking[0] as u128;
    v = v * 5 + tracking[1] as u128;
    for &d in &tracking[2..] {
        v = v * 10 + d as u128;
    }
    // The frame check sequence over the 13 bytes of the value.
    let bytes: Vec<u8> = (0..13).rev().map(|i| (v >> (8 * i)) as u8).collect();
    let mut fcs: u32 = 0x7FF;
    let mut data = (bytes[0] as u32) << 5;
    for _ in 2..8 {
        fcs = if (fcs ^ data) & 0x400 != 0 { (fcs << 1) ^ 0xF35 } else { fcs << 1 };
        fcs &= 0x7FF;
        data <<= 1;
    }
    for &b in &bytes[1..] {
        let mut data = (b as u32) << 3;
        for _ in 0..8 {
            fcs = if (fcs ^ data) & 0x400 != 0 { (fcs << 1) ^ 0xF35 } else { fcs << 1 };
            fcs &= 0x7FF;
            data <<= 1;
        }
    }
    // Ten codewords.
    let mut cw = [0u32; 10];
    cw[9] = (v % 636) as u32;
    v /= 636;
    for i in (1..9).rev() {
        cw[i] = (v % 1365) as u32;
        v /= 1365;
    }
    cw[0] = v as u32;
    cw[9] *= 2;
    if fcs & 0x400 != 0 {
        cw[0] += 659;
    }
    let (t5, t2) = (nof13(5, 1287), nof13(2, 78));
    let mut chars = [0u16; 10];
    for i in 0..10 {
        let c = cw[i] as usize;
        chars[i] = if c < 1287 { t5[c] } else { *t2.get(c - 1287).ok_or("codeword out of range")? };
        if fcs & (1 << i) != 0 {
            chars[i] = !chars[i] & 0x1FFF;
        }
    }
    let bars: Vec<Bar> = BAR_TABLE
        .iter()
        .map(|&(dc, db, ac, ab)| {
            let down = chars[dc as usize] & (1 << db) != 0;
            let up = chars[ac as usize] & (1 << ab) != 0;
            match (up, down) {
                (true, true) => Bar::Full,
                (true, false) => Bar::Ascender,
                (false, true) => Bar::Descender,
                _ => Bar::Track,
            }
        })
        .collect();
    Ok(four_state(&bars, g, height_pt))
}

// ---------------------------------------------------------------- POSTNET

const POSTNET: [&str; 10] = ["11000", "00011", "00101", "00110", "01001", "01010", "01100", "10001", "10010", "10100"];

/// barcode4j's `POSTNETBean`: frame bars around the digits (and a check digit when added).
pub fn postnet(msg: &str, add_checksum: bool, bar: f64, gap: f64, short: f64, height_pt: f64) -> Result<Symbol, String> {
    let mut d: Vec<u8> = Vec::new();
    for c in msg.chars() {
        match c {
            '0'..='9' => d.push(c as u8 - b'0'),
            '-' | ' ' => {}
            _ => return Err(format!("POSTNET takes digits only, got {c:?}")),
        }
    }
    if add_checksum {
        let sum: u32 = d.iter().map(|&x| x as u32).sum();
        d.push(((10 - sum % 10) % 10) as u8);
    }
    let (quiet, vquiet) = (3.175, 1.016);
    let tall = pt2mm(height_pt);
    let mut pattern = String::from("1");
    for x in &d {
        pattern.push_str(POSTNET[*x as usize]);
    }
    pattern.push('1');
    let mut rects = Vec::new();
    for (i, b) in pattern.bytes().enumerate() {
        let x = quiet + i as f64 * (bar + gap);
        let h = if b == b'1' { tall } else { short };
        rects.push([r4(x), r4(vquiet + tall - h), r4(bar), r4(h)]);
    }
    let width = 2.0 * quiet + pattern.len() as f64 * (bar + gap) - gap;
    Ok(Symbol { width: r4(width), height: r4(tall + 2.0 * vquiet), rects, texts: Vec::new() })
}
