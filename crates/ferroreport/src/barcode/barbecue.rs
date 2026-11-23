//! Barcodes of the `barbecue` component, drawn as the Barbecue library draws them for
//! JasperReports on a server (no display, so without the human-readable text): bars of a whole
//! number of pixels, the quiet zones Barbecue leaves, its default heights.

use super::linear::{mod10, upc_code, CODABAR, CODABAR_CHARS, CODE39, CODE39_CHARS, CODE39_STAR, EAN13_PARITY, I25};
use super::Symbol;

/// The `type` names Barbecue draws here.
pub const TYPES: &[&str] = &[
    "Code128", "Code128A", "Code128B", "Code128C", "EAN128", "SSCC18", "Code39", "3of9", "USD3", "Int2of5", "Std2of5", "Codabar",
    "2of7", "NW7", "Monarch", "USD4", "EAN13", "UPCA",
];

/// What a `barbecue` element asks for.
#[derive(Debug, Clone, PartialEq)]
pub struct Spec {
    pub kind: String,
    pub draw_text: bool,
    pub checksum: bool,
    /// Width of the narrowest bar, in pixels (Barbecue's default: 2).
    pub bar_width: Option<u32>,
    pub bar_height: Option<u32>,
}

/// One bar or space, in modules; `long` bars go below the others (UPC and EAN guards).
struct Element {
    bar: bool,
    modules: u32,
    long: bool,
}

fn push(out: &mut Vec<Element>, bar: bool, modules: u32) {
    out.push(Element { bar, modules, long: false });
}

/// Bars and spaces from a pattern of digits (widths), starting with a bar.
fn widths(out: &mut Vec<Element>, pattern: &str) {
    for (i, w) in pattern.bytes().enumerate() {
        push(out, i % 2 == 0, u32::from(w - b'0'));
    }
}

/// Bars and spaces from a pattern of wide (`1`) and narrow (`0`) elements, starting with a bar.
fn wide_narrow(out: &mut Vec<Element>, pattern: &str, wide: u32) {
    for (i, w) in pattern.bytes().enumerate() {
        push(out, i % 2 == 0, if w == b'1' { wide } else { 1 });
    }
}

/// Default height of the Code 128 family: it grows with the length of the text.
fn grows(chars: i64) -> u32 {
    (chars * 7).div_euclid(6).saturating_add(7).max(18) as u32
}

fn code128_values(kind: &str, code: &str) -> Result<Vec<u32>, String> {
    let chars: Vec<char> = code.chars().collect();
    let b = |c: char| match c as u32 {
        32..=127 => Ok(c as u32 - 32),
        _ => Err(format!("Code 128 cannot encode {c:?}")),
    };
    let mut v = Vec::new();
    match kind {
        "Code128A" => {
            v.push(103);
            for &c in &chars {
                v.push(match c as u32 {
                    32..=95 => c as u32 - 32,
                    0..=31 => c as u32 + 64,
                    _ => return Err(format!("Code 128 A cannot encode {c:?}")),
                });
            }
        }
        "Code128B" => {
            v.push(104);
            for &c in &chars {
                v.push(b(c)?);
            }
        }
        "Code128C" => {
            if let Some(c) = chars.iter().find(|c| !c.is_ascii_digit()) {
                return Err(format!("Code 128 C encodes digits only, not {c:?}"));
            }
            v.push(105);
            let mut digits: Vec<u32> = chars.iter().map(|c| *c as u32 - '0' as u32).collect();
            if digits.len() % 2 == 1 {
                digits.insert(0, 0);
            }
            v.extend(digits.chunks(2).map(|p| p[0] * 10 + p[1]));
        }
        "EAN128" | "SSCC18" => {
            // Start C, FNC1, the application identifier and the digits that follow it, in pairs.
            v.extend([105, 102]);
            let prefix = if kind == "SSCC18" { "00" } else { "01" };
            let mut digits: Vec<u32> = prefix
                .chars()
                .chain(chars.iter().copied().take_while(char::is_ascii_digit))
                .map(|c| c as u32 - '0' as u32)
                .collect();
            digits.truncate(digits.len() / 2 * 2);
            v.extend(digits.chunks(2).map(|p| p[0] * 10 + p[1]));
        }
        _ => {
            // Character set B, with runs of four digits or more in C.
            v.push(104);
            let mut in_c = false;
            let mut i = 0;
            while i < chars.len() {
                let run = chars[i..].iter().take_while(|c| c.is_ascii_digit()).count();
                if run >= 4 {
                    if !in_c {
                        v.push(99);
                        in_c = true;
                    }
                    for p in chars[i..i + run / 2 * 2].chunks(2) {
                        v.push((p[0] as u32 - '0' as u32) * 10 + (p[1] as u32 - '0' as u32));
                    }
                    i += run / 2 * 2;
                    continue;
                }
                if in_c {
                    v.push(100);
                    in_c = false;
                }
                v.push(b(chars[i])?);
                i += 1;
            }
        }
    }
    let check = v.iter().enumerate().map(|(i, &x)| x * (i.max(1) as u32)).sum::<u32>() % 103;
    v.push(check);
    Ok(v)
}

fn code39(code: &str, checksum: bool) -> Result<Vec<Element>, String> {
    let mut values = Vec::new();
    for c in code.chars() {
        values.push(CODE39_CHARS.find(c).ok_or_else(|| format!("Code 39 cannot encode {c:?}"))?);
    }
    if checksum {
        values.push(values.iter().sum::<usize>() % 43);
    }
    let mut out = Vec::new();
    wide_narrow(&mut out, CODE39_STAR, 2);
    for v in values {
        push(&mut out, false, 1);
        wide_narrow(&mut out, CODE39[v], 2);
    }
    push(&mut out, false, 1);
    wide_narrow(&mut out, CODE39_STAR, 2);
    Ok(out)
}

fn digits(code: &str, what: &str) -> Result<Vec<u8>, String> {
    code.chars()
        .map(|c| c.to_digit(10).map(|d| d as u8).ok_or_else(|| format!("{what} encodes digits only, not {c:?}")))
        .collect()
}

fn interleaved(code: &str, checksum: bool) -> Result<Vec<Element>, String> {
    let mut d = digits(code, "Interleaved 2 of 5")?;
    if checksum {
        // Weights 1, 3, 1, ... from the left.
        let sum: u32 = d.iter().enumerate().map(|(i, &x)| u32::from(x) * if i % 2 == 0 { 1 } else { 3 }).sum();
        d.push(((10 - sum % 10) % 10) as u8);
    }
    if d.len() % 2 == 1 {
        return Err("Interleaved 2 of 5 needs an even number of digits".into());
    }
    let mut out = Vec::new();
    widths(&mut out, "1111");
    for p in d.chunks(2) {
        let (a, b) = (I25[p[0] as usize].as_bytes(), I25[p[1] as usize].as_bytes());
        for i in 0..5 {
            push(&mut out, true, if a[i] == b'1' { 3 } else { 1 });
            push(&mut out, false, if b[i] == b'1' { 3 } else { 1 });
        }
    }
    widths(&mut out, "311");
    Ok(out)
}

fn standard(code: &str, checksum: bool) -> Result<Vec<Element>, String> {
    let mut d = digits(code, "Standard 2 of 5")?;
    if checksum {
        d.push(mod10(&d));
    }
    let mut out = Vec::new();
    widths(&mut out, "313111");
    for digit in d {
        for w in I25[digit as usize].bytes() {
            push(&mut out, true, if w == b'1' { 3 } else { 1 });
            push(&mut out, false, 1);
        }
    }
    widths(&mut out, "31113");
    push(&mut out, false, 1);
    Ok(out)
}

/// Codabar (Barbecue ignores the checksum here). Barbecue's B and C are each other's C and B in
/// the usual table; without start and stop characters it adds A and C.
fn codabar(code: &str) -> Result<Vec<Element>, String> {
    let mut chars: Vec<char> = code
        .chars()
        .map(|c| match c.to_ascii_uppercase() {
            'A' => 'a',
            'B' => 'c',
            'C' => 'b',
            'D' => 'd',
            c => c,
        })
        .collect();
    let is_start_stop = |c: &char| ('a'..='d').contains(c);
    if !chars.first().is_some_and(is_start_stop) {
        chars.insert(0, 'a');
    }
    if chars.len() < 2 || !chars.last().is_some_and(is_start_stop) {
        chars.push('b');
    }
    let mut values = Vec::new();
    for c in &chars {
        values.push(CODABAR_CHARS.find(*c).ok_or_else(|| format!("Codabar cannot encode {c:?}"))?);
    }
    let mut out = Vec::new();
    for (i, v) in values.iter().enumerate() {
        if i > 0 {
            push(&mut out, false, 1);
        }
        wide_narrow(&mut out, CODABAR[*v], 2);
    }
    Ok(out)
}

/// EAN-13 (12 digits) or UPC-A (11 digits), with the check digit added.
fn upc_ean(kind: &str, code: &str) -> Result<Vec<Element>, String> {
    let n = if kind == "UPCA" { 11 } else { 12 };
    let mut d = digits(code, kind)?;
    if d.len() != n {
        return Err(format!("{kind} takes {n} digits (the check digit is added), not {}", d.len()));
    }
    d.push(mod10(&d));
    // UPC-A is EAN-13 with a leading zero; its first and last digits have long bars.
    let upc = kind == "UPCA";
    if upc {
        d.insert(0, 0);
    }
    let mut out = Vec::new();
    let modules = |out: &mut Vec<Element>, bits: &str, long_bars: bool| {
        for b in bits.bytes() {
            out.push(Element { bar: b == b'1', modules: 1, long: long_bars });
        }
    };
    modules(&mut out, "101", true);
    let parity = EAN13_PARITY[d[0] as usize].as_bytes();
    for i in 0..6 {
        let set = if parity[i] == b'G' { 'G' } else { 'L' };
        modules(&mut out, &upc_code(d[1 + i], set), upc && i == 0);
    }
    modules(&mut out, "01010", true);
    for i in 0..6 {
        modules(&mut out, &upc_code(d[7 + i], 'R'), upc && i == 5);
    }
    modules(&mut out, "101", true);
    Ok(out)
}

/// The symbol of `code`, in pixels, before any rotation.
pub fn symbol(spec: &Spec, code: &str) -> Result<Symbol, String> {
    let kind = spec.kind.as_str();
    let bw = spec.bar_width.filter(|w| *w > 0).unwrap_or(2);
    let n = code.chars().count() as i64;
    let (elements, quiet_left, quiet_right, default_height) = match kind {
        "Code128" | "Code128A" | "Code128B" | "Code128C" | "EAN128" | "SSCC18" => {
            let mut out = Vec::new();
            for v in code128_values(kind, code)? {
                widths(&mut out, super::code128::PATTERNS[v as usize]);
            }
            widths(&mut out, super::code128::STOP);
            let grow = if kind.starts_with("Code128") { n - 3 } else { n };
            (out, 10, 10, grows(grow))
        }
        "Code39" | "3of9" | "USD3" => (code39(code, spec.checksum)?, 0, 0, 50),
        "Int2of5" => (interleaved(code, spec.checksum)?, 0, 0, 50),
        "Std2of5" => (standard(code, spec.checksum)?, 0, 0, 50),
        "Codabar" | "2of7" | "NW7" | "Monarch" | "USD4" => (codabar(code)?, 0, 0, 50),
        "EAN13" => (upc_ean(kind, code)?, 11, 7, 50),
        "UPCA" => (upc_ean(kind, code)?, 11, 11, 50),
        other => return Err(format!("barbecue type `{other}` is not supported yet")),
    };
    let height = spec.bar_height.filter(|h| *h > 0).unwrap_or(default_height) as f64;
    // UPC and EAN: Barbecue keeps room under the bars for the text (even when it cannot draw it).
    let (long_h, short_h) = if matches!(kind, "EAN13" | "UPCA") {
        let long = if spec.draw_text { height - 5.0 * bw as f64 } else { height };
        (long, long - 6.0 * bw as f64)
    } else {
        (height, height)
    };
    let mut rects: Vec<[f64; 4]> = Vec::new();
    let mut x = (quiet_left * bw) as f64;
    for e in &elements {
        let w = (e.modules * bw) as f64;
        if e.bar {
            let h = if e.long { long_h } else { short_h };
            // Modules side by side make one bar.
            match rects.last_mut() {
                Some(last) if last[0] + last[2] == x && last[3] == h => last[2] += w,
                _ => rects.push([x, 0.0, w, h]),
            }
        }
        x += w;
    }
    Ok(Symbol { width: x + (quiet_right * bw) as f64, height, rects, texts: Vec::new() })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What Barbecue 1.5 drew through JasperReports for each case (`r` lines are its black
    /// rectangles), against what is drawn here.
    #[test]
    fn draws_like_barbecue() {
        let corpus = include_str!("../../tests/data/barbecue.txt");
        let mut failures = Vec::new();
        let mut checked = 0;
        for case in corpus.split("@ ").skip(1) {
            let mut lines = case.lines();
            let args: Vec<&str> = lines.next().unwrap_or("").split_whitespace().collect();
            let rest: Vec<&str> = lines.collect();
            let (kind, code) = (args[0], args[1].replace('_', " "));
            let spec = Spec {
                kind: kind.into(),
                draw_text: args.get(2).is_none_or(|v| *v == "true"),
                checksum: args.get(3).is_some_and(|v| *v == "true"),
                bar_width: args.get(4).and_then(|v| v.parse().ok()),
                bar_height: args.get(5).and_then(|v| v.parse().ok()),
            };
            let ours = symbol(&spec, &code);
            if rest.first().is_some_and(|l| l.starts_with("error")) {
                if ours.is_ok() && TYPES.contains(&kind) {
                    failures.push(format!("{kind} {code}: Barbecue refuses it, drawn here"));
                }
                continue;
            }
            // Barbecue leaves out what it cannot encode in a way not followed here: letters
            // in Code 128 A and after the digits of an EAN-128 code.
            if !TYPES.contains(&kind) || ["Code128A ABC def", "EAN128 10ABC"].contains(&format!("{kind} {code}").as_str()) {
                continue;
            }
            checked += 1;
            let ours = match ours {
                Ok(s) => s,
                Err(e) => {
                    failures.push(format!("{kind} {code}: {e}"));
                    continue;
                }
            };
            let size: Vec<f64> = rest[0].split_whitespace().skip(1).filter_map(|v| v.parse().ok()).collect();
            let theirs: Vec<[f64; 4]> = rest[1..]
                .iter()
                .filter(|l| l.starts_with("r "))
                .map(|l| {
                    let v: Vec<f64> = l.split_whitespace().skip(1).filter_map(|v| v.parse().ok()).collect();
                    [v[0], v[1], v[2], v[3]]
                })
                .collect();
            if [ours.width, ours.height] != [size[0], size[1]] || ours.rects != theirs {
                failures.push(format!(
                    "{}: size {}x{} (Barbecue {}x{}), {} bars (Barbecue {}), first difference {:?}",
                    args.join(" "),
                    ours.width,
                    ours.height,
                    size[0],
                    size[1],
                    ours.rects.len(),
                    theirs.len(),
                    ours.rects.iter().zip(&theirs).find(|(a, b)| a != b)
                ));
            }
        }
        assert!(failures.is_empty(), "{} of {checked} differ:\n{}", failures.len(), failures.join("\n"));
    }
}
