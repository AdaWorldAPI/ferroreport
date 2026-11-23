//! PDF417, encoded like barcode4j's `PDF417Bean` (the one JasperReports uses): the same choice
//! of text, byte and numeric compaction, the same Reed-Solomon error correction, and the same
//! number of rows and columns for a message.

#[path = "pdf417_tables.rs"]
mod tables;

const TEXT: u8 = 0;
const BYTE: u8 = 1;
const NUMERIC: u8 = 2;

const ALPHA: u8 = 0;
const LOWER: u8 = 1;
const MIXED: u8 = 2;
const PUNCTUATION: u8 = 3;

const LATCH_TO_TEXT: u16 = 900;
const LATCH_TO_BYTE_PADDED: u16 = 901;
const LATCH_TO_NUMERIC: u16 = 902;
const SHIFT_TO_BYTE: u16 = 913;
const LATCH_TO_BYTE: u16 = 924;

const MIXED_RAW: [u8; 30] =
    [48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 38, 13, 9, 44, 58, 35, 45, 46, 36, 47, 43, 37, 42, 61, 94, 0, 32, 0, 0, 0];
const PUNCTUATION_RAW: [u8; 30] =
    [59, 60, 62, 64, 91, 92, 93, 95, 96, 126, 33, 13, 9, 44, 58, 10, 45, 46, 36, 47, 34, 124, 42, 40, 41, 63, 123, 125, 39, 0];

/// Where a character is in a text submode's table.
fn index(raw: &[u8; 30], ch: char) -> Option<u16> {
    let c = u32::from(ch);
    (c > 0 && c < 128).then(|| raw.iter().position(|&r| u32::from(r) == c).map(|i| i as u16)).flatten()
}

fn is_digit(c: char) -> bool {
    c.is_ascii_digit()
}
fn is_upper(c: char) -> bool {
    c == ' ' || c.is_ascii_uppercase()
}
fn is_lower(c: char) -> bool {
    c == ' ' || c.is_ascii_lowercase()
}
fn is_mixed(c: char) -> bool {
    index(&MIXED_RAW, c).is_some()
}
fn is_punctuation(c: char) -> bool {
    index(&PUNCTUATION_RAW, c).is_some()
}
fn is_text(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r') || (' '..='~').contains(&c)
}

/// The message's characters as code page 437 bytes, as barcode4j reads them.
fn cp437(chars: &[char]) -> Result<Vec<u8>, String> {
    chars
        .iter()
        .map(|&c| {
            if (c as u32) < 128 {
                return Ok(c as u8);
            }
            CP437_HIGH
                .iter()
                .position(|&h| h == c)
                .map(|i| 128 + i as u8)
                .ok_or_else(|| format!("PDF417: character `{c}` (Unicode: {}) cannot be encoded", c as u32))
        })
        .collect()
}

const CP437_HIGH: [char; 128] = [
    'Ç', 'ü', 'é', 'â', 'ä', 'à', 'å', 'ç', 'ê', 'ë', 'è', 'ï', 'î', 'ì', 'Ä', 'Å', 'É', 'æ', 'Æ', 'ô', 'ö', 'ò', 'û', 'ù', 'ÿ',
    'Ö', 'Ü', '¢', '£', '¥', '₧', 'ƒ', 'á', 'í', 'ó', 'ú', 'ñ', 'Ñ', 'ª', 'º', '¿', '⌐', '¬', '½', '¼', '¡', '«', '»', '░', '▒',
    '▓', '│', '┤', '╡', '╢', '╖', '╕', '╣', '║', '╗', '╝', '╜', '╛', '┐', '└', '┴', '┬', '├', '─', '┼', '╞', '╟', '╚', '╔', '╩',
    '╦', '╠', '═', '╬', '╧', '╨', '╤', '╥', '╙', '╘', '╒', '╓', '╫', '╪', '┘', '┌', '█', '▄', '▌', '▐', '▀', 'α', 'ß', 'Γ', 'π',
    'Σ', 'σ', 'µ', 'τ', 'Φ', 'Θ', 'Ω', 'δ', '∞', 'φ', 'ε', '∩', '≡', '±', '≥', '≤', '⌠', '⌡', '÷', '≈', '°', '∙', '·', '√', 'ⁿ',
    '²', '■', '\u{a0}',
];

/// The data codewords of a message (without the length codeword in front).
fn encode_high_level(msg: &str) -> Result<Vec<u16>, String> {
    let chars: Vec<char> = msg.chars().collect();
    let len = chars.len();
    let mut bytes: Option<Vec<u8>> = None;
    let mut out = Vec::with_capacity(len);
    let (mut p, mut mode, mut submode) = (0, TEXT, ALPHA);
    while p < len {
        let n = consecutive_digits(&chars, p);
        if n >= 13 {
            out.push(LATCH_TO_NUMERIC);
            mode = NUMERIC;
            submode = ALPHA;
            encode_numeric(&chars, p, n, &mut out);
            p += n;
            continue;
        }
        let t = consecutive_text(&chars, p);
        if t >= 5 || n == len {
            if mode != TEXT {
                out.push(LATCH_TO_TEXT);
                mode = TEXT;
                submode = ALPHA;
            }
            submode = encode_text(&chars, p, t, &mut out, submode);
            p += t;
            continue;
        }
        if bytes.is_none() {
            bytes = Some(cp437(&chars)?);
        }
        let bytes = bytes.as_deref().unwrap_or_default();
        let b = consecutive_binary(&chars, bytes, p)?.max(1);
        if b == 1 && mode == TEXT {
            encode_binary(bytes, p, 1, TEXT, &mut out);
        } else {
            encode_binary(bytes, p, b, mode, &mut out);
            mode = BYTE;
            submode = ALPHA;
        }
        p += b;
    }
    Ok(out)
}

fn encode_text(chars: &[char], start: usize, count: usize, out: &mut Vec<u16>, initial: u8) -> u8 {
    let mut tmp: Vec<u16> = Vec::with_capacity(count);
    let mut submode = initial;
    let mut idx = 0;
    loop {
        let ch = chars[start + idx];
        match submode {
            ALPHA => {
                if is_upper(ch) {
                    tmp.push(if ch == ' ' { 26 } else { ch as u16 - 65 });
                } else if is_lower(ch) {
                    submode = LOWER;
                    tmp.push(27);
                    continue;
                } else if is_mixed(ch) {
                    submode = MIXED;
                    tmp.push(28);
                    continue;
                } else {
                    tmp.push(29);
                    tmp.push(index(&PUNCTUATION_RAW, ch).unwrap_or(0));
                }
            }
            LOWER => {
                if is_lower(ch) {
                    tmp.push(if ch == ' ' { 26 } else { ch as u16 - 97 });
                } else if is_upper(ch) {
                    tmp.push(27);
                    tmp.push(ch as u16 - 65);
                } else if is_mixed(ch) {
                    submode = MIXED;
                    tmp.push(28);
                    continue;
                } else {
                    tmp.push(29);
                    tmp.push(index(&PUNCTUATION_RAW, ch).unwrap_or(0));
                }
            }
            MIXED => {
                if is_mixed(ch) {
                    tmp.push(index(&MIXED_RAW, ch).unwrap_or(0));
                } else if is_upper(ch) {
                    submode = ALPHA;
                    tmp.push(28);
                    continue;
                } else if is_lower(ch) {
                    submode = LOWER;
                    tmp.push(27);
                    continue;
                } else {
                    // barcode4j compares the next position with the count, not with the end.
                    if start + idx + 1 < count && is_punctuation(chars[start + idx + 1]) {
                        submode = PUNCTUATION;
                        tmp.push(25);
                        continue;
                    }
                    tmp.push(29);
                    tmp.push(index(&PUNCTUATION_RAW, ch).unwrap_or(0));
                }
            }
            _ => {
                if is_punctuation(ch) {
                    tmp.push(index(&PUNCTUATION_RAW, ch).unwrap_or(0));
                } else {
                    submode = ALPHA;
                    tmp.push(29);
                    continue;
                }
            }
        }
        idx += 1;
        if idx >= count {
            break;
        }
    }
    let mut h = 0u16;
    for (i, &v) in tmp.iter().enumerate() {
        if i % 2 == 1 {
            h = h * 30 + v;
            out.push(h);
        } else {
            h = v;
        }
    }
    if tmp.len() % 2 == 1 {
        out.push(h * 30 + 29);
    }
    submode
}

fn encode_binary(bytes: &[u8], start: usize, count: usize, mode: u8, out: &mut Vec<u16>) {
    if count == 1 && mode == TEXT {
        out.push(SHIFT_TO_BYTE);
    } else if count % 6 == 0 {
        out.push(LATCH_TO_BYTE);
    } else {
        out.push(LATCH_TO_BYTE_PADDED);
    }
    let mut idx = start;
    while start + count - idx >= 6 {
        let mut t: u64 = 0;
        for i in 0..6 {
            t = (t << 8) + u64::from(bytes[idx + i]);
        }
        let mut five = [0u16; 5];
        for c in five.iter_mut() {
            *c = (t % 900) as u16;
            t /= 900;
        }
        out.extend(five.iter().rev());
        idx += 6;
    }
    out.extend(bytes[idx..start + count].iter().map(|&b| u16::from(b)));
}

fn encode_numeric(chars: &[char], start: usize, count: usize, out: &mut Vec<u16>) {
    let mut idx = 0;
    // barcode4j stops before a last single digit.
    while idx + 1 < count {
        let len = (count - idx).min(44);
        // "1" followed by up to 44 digits, in base 900.
        let mut digits: Vec<u32> =
            std::iter::once(1).chain(chars[start + idx..start + idx + len].iter().map(|c| *c as u32 - 48)).collect();
        let mut tmp = Vec::new();
        loop {
            // Divide the decimal number by 900.
            let mut rem = 0u32;
            let mut quotient = Vec::with_capacity(digits.len());
            for &d in &digits {
                let cur = rem * 10 + d;
                quotient.push(cur / 900);
                rem = cur % 900;
            }
            tmp.push(rem as u16);
            let first = quotient.iter().position(|&q| q != 0);
            match first {
                Some(f) => digits = quotient[f..].to_vec(),
                None => break,
            }
        }
        out.extend(tmp.iter().rev());
        idx += len;
    }
}

fn consecutive_digits(chars: &[char], start: usize) -> usize {
    chars[start..].iter().take_while(|c| is_digit(**c)).count()
}

fn consecutive_text(chars: &[char], start: usize) -> usize {
    let len = chars.len();
    let mut idx = start;
    while idx < len {
        let mut ch = chars[idx];
        let mut numeric = 0;
        while numeric < 13 && is_digit(ch) && idx < len {
            numeric += 1;
            idx += 1;
            if idx < len {
                ch = chars[idx];
            }
        }
        if numeric >= 13 {
            return idx - start - numeric;
        }
        if numeric > 0 {
            continue;
        }
        if !is_text(chars[idx]) {
            break;
        }
        idx += 1;
    }
    idx - start
}

fn consecutive_binary(chars: &[char], bytes: &[u8], start: usize) -> Result<usize, String> {
    let len = chars.len();
    let mut idx = start;
    while idx < len {
        let mut ch = chars[idx];
        let mut numeric = 0;
        while numeric < 13 && is_digit(ch) {
            numeric += 1;
            let i = idx + numeric;
            if i >= len {
                break;
            }
            ch = chars[i];
        }
        if numeric >= 13 {
            return Ok(idx - start);
        }
        let mut text = 0;
        while text < 5 && is_text(ch) {
            text += 1;
            let i = idx + text;
            if i >= len {
                break;
            }
            ch = chars[i];
        }
        if text >= 5 {
            return Ok(idx - start);
        }
        if bytes[idx] == b'?' && chars[idx] != '?' {
            return Err(format!("PDF417: character `{}` cannot be encoded", chars[idx]));
        }
        idx += 1;
    }
    Ok(idx - start)
}

fn ec_count(level: u8) -> usize {
    1 << (level + 1)
}

fn error_correction(data: &[u16], level: u8) -> Vec<u16> {
    let k = ec_count(level);
    let coefficients = tables::EC_COEFFICIENTS[level as usize];
    let mut e = vec![0u32; k];
    for &d in data {
        let t1 = (u32::from(d) + e[k - 1]) % 929;
        for j in (1..k).rev() {
            let t2 = (t1 * u32::from(coefficients[j])) % 929;
            e[j] = (e[j - 1] + 929 - t2) % 929;
        }
        let t2 = (t1 * u32::from(coefficients[0])) % 929;
        e[0] = (929 - t2) % 929;
    }
    e.iter().rev().map(|&v| if v != 0 { (929 - v) as u16 } else { 0 }).collect()
}

fn rows_for(m: usize, k: usize, c: usize) -> usize {
    let mut r = (m + 1 + k) / c + 1;
    if c * r >= m + 1 + k + c {
        r -= 1;
    }
    r
}

/// The settings barcode4j's `PDF417Bean` takes, sizes in millimeters.
#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub min_cols: usize,
    pub max_cols: usize,
    pub min_rows: usize,
    pub max_rows: usize,
    pub ratio: f64,
    pub level: u8,
    pub module_width: f64,
    pub row_height: f64,
}

/// The modules of the symbol: one row of 17 × columns + 69 modules per row, dark as `true`.
pub fn matrix(msg: &str, o: &Options) -> Result<(Vec<Vec<bool>>, usize), String> {
    if msg.is_empty() {
        return Err("PDF417: the message is empty".into());
    }
    if o.level > 8 {
        return Err("PDF417: error correction level must be between 0 and 8".into());
    }
    let k = ec_count(o.level);
    let data = encode_high_level(msg)?;
    let m = data.len();
    // The columns and rows whose proportions come closest to the wanted ratio.
    let mut best: Option<(usize, usize)> = None;
    let mut ratio = 0.0;
    for cols in o.min_cols..=o.max_cols {
        let rows = rows_for(m, k, cols);
        if rows < o.min_rows {
            break;
        }
        if rows > o.max_rows {
            continue;
        }
        let r = ((17 * cols + 69) as f64 * o.module_width) / (rows as f64 * o.row_height);
        if best.is_some() && (r - o.ratio).abs() > (ratio - o.ratio).abs() {
            continue;
        }
        ratio = r;
        best = Some((cols, rows));
    }
    let (cols, rows) = best.ok_or("PDF417: unable to fit the message in the columns")?;
    if rows > 90 {
        return Err("PDF417: the message needs more than 90 rows".into());
    }
    let pad = {
        let free = cols * rows - k;
        if free > m + 1 {
            free - m - 1
        } else {
            0
        }
    };
    if cols * rows - k > 929 {
        return Err(format!("PDF417: the message is too long ({m} codewords)"));
    }
    let mut codewords = Vec::with_capacity(cols * rows);
    codewords.push((m + pad + 1) as u16);
    codewords.extend(&data);
    codewords.extend(std::iter::repeat_n(900u16, pad));
    let ec = error_correction(&codewords, o.level);
    codewords.extend(ec);

    let level = o.level as usize;
    let mut out = Vec::with_capacity(rows);
    let mut idx = 0;
    for y in 0..rows {
        let cluster = y % 3;
        let base = 30 * (y / 3);
        let (left, right) = match cluster {
            0 => (base + (rows - 1) / 3, base + cols - 1),
            1 => (base + level * 3 + (rows - 1) % 3, base + (rows - 1) / 3),
            _ => (base + cols - 1, base + level * 3 + (rows - 1) % 3),
        };
        let mut row = Vec::with_capacity(17 * cols + 69);
        let mut put = |pattern: u32, len: u32| {
            for bit in (0..len).rev() {
                row.push(pattern & (1 << bit) != 0);
            }
        };
        put(130728, 17);
        put(tables::CODEWORDS[cluster][left], 17);
        for _ in 0..cols {
            put(tables::CODEWORDS[cluster][codewords[idx] as usize], 17);
            idx += 1;
        }
        put(tables::CODEWORDS[cluster][right], 17);
        put(260649, 18);
        out.push(row);
    }
    Ok((out, cols))
}

/// The modules of a PDF417 symbol as the Barbecue library draws it: every message in byte
/// compaction, 12 data columns, at least 3 rows, error correction level 0. Barbecue writes the
/// length descriptor without the padding, counts the rows from 1 in the row indicators and
/// starts every row with `11111110110101000`; all of it is followed here, since that is what
/// JasperReports prints.
pub fn barbecue_matrix(msg: &str) -> Result<Vec<Vec<bool>>, String> {
    const COLS: usize = 12;
    if let Some(c) = msg.chars().find(|c| !c.is_ascii()) {
        return Err(format!("Barbecue's PDF417 cannot encode {c:?}"));
    }
    let mut data = Vec::new();
    encode_binary(msg.as_bytes(), 0, msg.len(), BYTE, &mut data);
    let k = ec_count(0);
    let rows = (data.len() + 1 + k).div_ceil(COLS).max(3);
    if rows * COLS > 928 {
        return Err(format!("PDF417: the message is too long ({} codewords)", data.len()));
    }
    let mut codewords = Vec::with_capacity(rows * COLS);
    codewords.push((data.len() + 1) as u16);
    codewords.extend(&data);
    codewords.resize(rows * COLS - k, 900);
    let ec = error_correction(&codewords, 0);
    codewords.extend(ec);
    let mut out = Vec::with_capacity(rows);
    for (y, row_codewords) in codewords.chunks(COLS).enumerate() {
        let cluster = y % 3;
        let base = 30 * (y / 3);
        let (left, right) = match cluster {
            0 => (base + rows / 3, base + COLS - 1),
            1 => (base + rows % 3, base + rows / 3),
            _ => (base + COLS - 1, base + rows % 3),
        };
        let mut row = Vec::with_capacity(17 * COLS + 69);
        let mut put = |pattern: u32, len: u32| {
            for bit in (0..len).rev() {
                row.push(pattern & (1 << bit) != 0);
            }
        };
        put(0b11111110110101000, 17);
        put(tables::CODEWORDS[cluster][left], 17);
        for &c in row_codewords {
            put(tables::CODEWORDS[cluster][c as usize], 17);
        }
        put(tables::CODEWORDS[cluster][right], 17);
        put(260649, 18);
        out.push(row);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_and_numbers_compact_as_barcode4j() {
        // Fewer than five text characters go as bytes.
        assert_eq!(encode_high_level("ABC").unwrap(), vec![LATCH_TO_BYTE_PADDED, 65, 66, 67]);
        // Five or more as text, two letters a codeword.
        assert_eq!(encode_high_level("ABCDE").unwrap(), vec![1, 30 * 2 + 3, 4 * 30 + 29]);
        // Thirteen digits or more go to numeric compaction.
        let n = encode_high_level("1234567890123").unwrap();
        assert_eq!(n[0], LATCH_TO_NUMERIC);
        // A single byte inside text is shifted.
        let b = encode_high_level("ABCDEé").unwrap();
        assert!(b.contains(&SHIFT_TO_BYTE));
    }
}
