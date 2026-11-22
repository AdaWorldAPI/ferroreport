//! Code 128, encoded like barcode4j's `DefaultCode128Encoder` (all code sets enabled).

const FNC1: char = '\u{F1}';
const FNC2: char = '\u{F2}';
const FNC3: char = '\u{F3}';
const FNC4: char = '\u{F4}';

/// Bar/space widths of each symbol value (6 elements), from barcode4j.
pub(crate) const PATTERNS: [&str; 106] = [
    "212222", "222122", "222221", "121223", "121322", "131222", "122213", "122312", "132212", "221213", "221312", "231212",
    "112232", "122132", "122231", "113222", "123122", "123221", "223211", "221132", "221231", "213212", "223112", "312131",
    "311222", "321122", "321221", "312212", "322112", "322211", "212123", "212321", "232121", "111323", "131123", "131321",
    "112313", "132113", "132311", "211313", "231113", "231311", "112133", "112331", "132131", "113123", "113321", "133121",
    "313121", "211331", "231131", "213113", "213311", "213131", "311123", "311321", "331121", "312113", "312311", "332111",
    "314111", "221411", "431111", "111224", "111422", "121124", "121421", "141122", "141221", "112214", "112412", "122114",
    "122411", "142112", "142211", "241211", "221114", "413111", "241112", "134111", "111242", "121142", "121241", "114212",
    "124112", "124211", "411212", "421112", "421211", "212141", "214121", "412121", "111143", "111341", "131141", "114113",
    "114311", "411113", "411311", "113141", "114131", "311141", "411131", "211412", "211214", "211232",
];
pub(crate) const STOP: &str = "2331112";

fn need_a(c: char) -> bool {
    (c as u32) < 32
}

fn need_b(c: char) -> bool {
    (96..128).contains(&(c as u32))
}

fn encode_a_or_b(c: char, set_a: bool) -> Result<u32, String> {
    let v = c as u32;
    Ok(match c {
        FNC1 => 102,
        FNC2 => 97,
        FNC3 => 96,
        FNC4 => {
            if set_a {
                101
            } else {
                100
            }
        }
        _ if set_a && v < 32 => v + 64,
        _ if set_a && (32..=95).contains(&v) => v - 32,
        _ if !set_a && (32..128).contains(&v) => v - 32,
        _ => return Err(format!("Code 128 cannot encode the character {c:?}")),
    })
}

fn encode_c(msg: &[char], start: usize, end: usize, out: &mut Vec<u32>) {
    if start == end {
        return;
    }
    out.push(if start == 0 { 105 } else { 99 });
    let mut i = start;
    while i < end {
        if msg[i] == FNC1 {
            out.push(102);
            i += 1;
        } else {
            let d = |c: char| c.to_digit(10).unwrap_or(0);
            out.push(d(msg[i]) * 10 + d(msg[i + 1]));
            i += 2;
        }
    }
}

fn encode_a_or_b_run(msg: &[char], start: usize, end: usize, out: &mut Vec<u32>) -> Result<(), String> {
    if start == end {
        return Ok(());
    }
    let mut current_b = true;
    for &c in &msg[start..end] {
        if need_a(c) {
            current_b = false;
            break;
        }
        if need_b(c) {
            break;
        }
    }
    out.push(match (current_b, start == 0) {
        (true, true) => 104,
        (true, false) => 100,
        (false, true) => 103,
        (false, false) => 101,
    });
    for i in start..end {
        let c = msg[i];
        if current_b {
            if need_a(c) {
                if i + 1 < end && need_a(msg[i + 1]) {
                    out.push(101);
                    current_b = false;
                } else {
                    out.push(98);
                }
                out.push(encode_a_or_b(c, true)?);
            } else {
                out.push(encode_a_or_b(c, false)?);
            }
        } else if need_b(c) {
            if i + 1 < end && need_b(msg[i + 1]) {
                out.push(100);
                current_b = true;
            } else {
                out.push(98);
            }
            out.push(encode_a_or_b(c, false)?);
        } else {
            out.push(encode_a_or_b(c, true)?);
        }
    }
    Ok(())
}

/// Symbol values for `msg`, start code included (no check digit, no stop).
pub fn encode(msg: &str) -> Result<Vec<u32>, String> {
    let chars: Vec<char> = msg.chars().collect();
    let n = chars.len();
    let mut out = Vec::with_capacity(n * 2);
    let mut ab_start = 0;
    let mut i = 0;
    while i < n {
        let mut digits = 0;
        let mut pairs = 0;
        let mut odd = false;
        while i + digits < n {
            let c = chars[i + digits];
            if c.is_ascii_digit() {
                if i + digits + 1 == n {
                    odd = true;
                    break;
                }
                if !chars[i + digits + 1].is_ascii_digit() {
                    break;
                }
                pairs += 1;
                digits += 2;
            } else if c == FNC1 && (i == 0 || digits > 0) {
                digits += 1;
            } else {
                break;
            }
        }
        if pairs >= 2 || digits == n {
            if odd {
                let fnc = chars[i..].iter().position(|&c| c == FNC1).map(|p| p + i);
                if fnc.map_or(true, |f| f > i + digits) {
                    i += 1;
                }
            }
            encode_a_or_b_run(&chars, ab_start, i, &mut out)?;
            ab_start = i + digits;
            encode_c(&chars, i, ab_start, &mut out);
        }
        i += digits + 1;
    }
    encode_a_or_b_run(&chars, ab_start, n, &mut out)?;
    Ok(out)
}

/// Module widths of the whole symbol (bar, space, bar, ...): data, check digit and stop.
pub fn modules(msg: &str) -> Result<Vec<u8>, String> {
    let codes = encode(msg)?;
    let check = codes.iter().enumerate().map(|(i, &c)| if i == 0 { c } else { c * i as u32 }).sum::<u32>() % 103;
    let mut widths = Vec::with_capacity((codes.len() + 2) * 6 + 1);
    for &c in codes.iter().chain(std::iter::once(&check)) {
        widths.extend(PATTERNS[c as usize].bytes().map(|b| b - b'0'));
    }
    widths.extend(STOP.bytes().map(|b| b - b'0'));
    Ok(widths)
}

/// The text printed under the bars (function characters removed).
pub fn human_readable(msg: &str) -> String {
    msg.chars().filter(|c| !matches!(*c, FNC1 | FNC2 | FNC3 | FNC4)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switches_code_sets_like_barcode4j() {
        // "ABC-1" in B, then "2345" in C.
        assert_eq!(encode("ABC-12345").unwrap(), vec![104, 33, 34, 35, 13, 17, 99, 23, 45]);
        // All digits, even: start C.
        assert_eq!(encode("123456").unwrap(), vec![105, 12, 34, 56]);
        // Odd digits: the first one goes to B.
        assert_eq!(encode("12345").unwrap(), vec![104, 17, 99, 23, 45]);
        // barcode4j reads U+00F1..U+00F4 as the function characters FNC1..FNC4.
        assert_eq!(encode("a\u{F1}b").unwrap(), vec![104, 65, 102, 66]);
        assert!(encode("€").is_err());
    }

    #[test]
    fn symbol_width() {
        // 9 codes + check (11 modules each) + stop (13).
        assert_eq!(modules("ABC-12345").unwrap().iter().map(|&w| w as u32).sum::<u32>(), 10 * 11 + 13);
    }
}
