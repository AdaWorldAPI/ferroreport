//! QR code encoder, following ZXing's `Encoder` step by step (mode choice, version choice,
//! mask selection) so the symbols are module-for-module identical to the ones
//! JasperReports draws.

use super::qr_tables::{ALIGNMENT, EC_BLOCKS};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ecl {
    L,
    M,
    Q,
    H,
}

impl Ecl {
    pub fn from_name(name: &str) -> Option<Ecl> {
        Some(match name {
            "L" => Ecl::L,
            "M" => Ecl::M,
            "Q" => Ecl::Q,
            "H" => Ecl::H,
            _ => return None,
        })
    }

    fn index(self) -> usize {
        self as usize
    }

    /// The two format bits of the level (not in alphabetical order).
    fn format_bits(self) -> u32 {
        match self {
            Ecl::L => 1,
            Ecl::M => 0,
            Ecl::Q => 3,
            Ecl::H => 2,
        }
    }
}

/// An encoded symbol: `size`×`size` modules, row-major, `true` = dark.
#[derive(Debug, Clone)]
pub struct QrCode {
    pub version: u8,
    pub mask: u8,
    pub size: usize,
    pub modules: Vec<bool>,
}

impl QrCode {
    pub fn get(&self, x: usize, y: usize) -> bool {
        self.modules[y * self.size + x]
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Mode {
    Numeric,
    Alphanumeric,
    Byte,
}

impl Mode {
    fn bits(self) -> u32 {
        match self {
            Mode::Numeric => 0b0001,
            Mode::Alphanumeric => 0b0010,
            Mode::Byte => 0b0100,
        }
    }

    fn count_bits(self, version: u8) -> usize {
        let i = match version {
            1..=9 => 0,
            10..=26 => 1,
            _ => 2,
        };
        match self {
            Mode::Numeric => [10, 12, 14][i],
            Mode::Alphanumeric => [9, 11, 13][i],
            Mode::Byte => [8, 16, 16][i],
        }
    }
}

const ALNUM: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ $%*+-./:";

fn alnum_code(c: char) -> Option<u32> {
    if c.is_ascii() {
        ALNUM.iter().position(|&a| a == c as u8).map(|p| p as u32)
    } else {
        None
    }
}

#[derive(Default, Clone)]
struct Bits(Vec<bool>);

impl Bits {
    fn push(&mut self, value: u32, n: usize) {
        for i in (0..n).rev() {
            self.0.push((value >> i) & 1 == 1);
        }
    }
    fn len(&self) -> usize {
        self.0.len()
    }
    fn bytes(&self) -> Vec<u8> {
        self.0.chunks(8).map(|c| c.iter().enumerate().fold(0u8, |acc, (i, &b)| acc | ((b as u8) << (7 - i)))).collect()
    }
}

struct VersionInfo {
    total: usize,
    data: usize,
    ec_per_block: usize,
    blocks: Vec<usize>,
}

fn version_info(version: u8, ecl: Ecl) -> VersionInfo {
    let (ec, groups) = EC_BLOCKS[version as usize - 1][ecl.index()];
    let mut blocks = Vec::new();
    for (count, data) in groups {
        for _ in 0..count {
            blocks.push(data as usize);
        }
    }
    let data: usize = blocks.iter().sum();
    VersionInfo { total: data + blocks.len() * ec as usize, data, ec_per_block: ec as usize, blocks }
}

fn fits(bits: usize, version: u8, ecl: Ecl) -> bool {
    version_info(version, ecl).data >= bits.div_ceil(8)
}

fn choose_version(bits: usize, ecl: Ecl) -> Result<u8, String> {
    (1..=40).find(|&v| fits(bits, v, ecl)).ok_or_else(|| "data too big for a QR code".to_string())
}

/// Encodes `content` (as UTF-8, with an ECI marker in byte mode, like JasperReports).
pub fn encode(content: &str, ecl: Ecl, version: Option<u8>) -> Result<QrCode, String> {
    // Mode: like ZXing's chooseMode for a non-Shift_JIS charset.
    let mut has_numeric = false;
    let mut has_alnum = false;
    let mut mode = None;
    for c in content.chars() {
        if c.is_ascii_digit() {
            has_numeric = true;
        } else if alnum_code(c).is_some() {
            has_alnum = true;
        } else {
            mode = Some(Mode::Byte);
            break;
        }
    }
    let mode = mode.unwrap_or(if has_alnum {
        Mode::Alphanumeric
    } else if has_numeric {
        Mode::Numeric
    } else {
        Mode::Byte
    });

    let mut header = Bits::default();
    if mode == Mode::Byte {
        header.push(0b0111, 4); // ECI
        header.push(26, 8); // UTF-8
    }
    header.push(mode.bits(), 4);

    let mut data = Bits::default();
    let letters;
    match mode {
        Mode::Numeric => {
            let digits: Vec<u32> = content.bytes().map(|b| (b - b'0') as u32).collect();
            letters = digits.len();
            for chunk in digits.chunks(3) {
                match chunk {
                    [a, b, c] => data.push(a * 100 + b * 10 + c, 10),
                    [a, b] => data.push(a * 10 + b, 7),
                    [a] => data.push(*a, 4),
                    _ => {}
                }
            }
        }
        Mode::Alphanumeric => {
            let codes: Vec<u32> = content.chars().filter_map(alnum_code).collect();
            letters = codes.len();
            for chunk in codes.chunks(2) {
                match chunk {
                    [a, b] => data.push(a * 45 + b, 11),
                    [a] => data.push(*a, 6),
                    _ => {}
                }
            }
        }
        Mode::Byte => {
            let bytes = content.as_bytes();
            letters = bytes.len();
            for &b in bytes {
                data.push(b as u32, 8);
            }
        }
    }

    let needed = |v: u8| header.len() + mode.count_bits(v) + data.len();
    let version = match version {
        Some(v) if (1..=40).contains(&v) => {
            if !fits(needed(v), v, ecl) {
                return Err(format!("data too big for QR version {v}"));
            }
            v
        }
        Some(v) => return Err(format!("invalid QR version {v}")),
        None => {
            let provisional = choose_version(needed(1), ecl)?;
            choose_version(needed(provisional), ecl)?
        }
    };
    let info = version_info(version, ecl);

    let mut bits = header;
    bits.push(letters as u32, mode.count_bits(version));
    bits.0.extend_from_slice(&data.0);
    let capacity = info.data * 8;
    if bits.len() > capacity {
        return Err("data too big for a QR code".into());
    }
    for _ in 0..4.min(capacity - bits.len()) {
        bits.0.push(false);
    }
    while bits.len() % 8 != 0 {
        bits.0.push(false);
    }
    let mut codewords = bits.bytes();
    let mut pad = [0xEC, 0x11].into_iter().cycle();
    while codewords.len() < info.data {
        codewords.push(pad.next().unwrap_or(0xEC));
    }

    // Reed-Solomon blocks, interleaved.
    let mut data_blocks = Vec::new();
    let mut ec_blocks = Vec::new();
    let mut offset = 0;
    for &n in &info.blocks {
        let block = &codewords[offset..offset + n];
        offset += n;
        ec_blocks.push(reed_solomon(block, info.ec_per_block));
        data_blocks.push(block.to_vec());
    }
    let mut final_bytes = Vec::with_capacity(info.total);
    let max_data = info.blocks.iter().copied().max().unwrap_or(0);
    for i in 0..max_data {
        for b in &data_blocks {
            if let Some(&v) = b.get(i) {
                final_bytes.push(v);
            }
        }
    }
    for i in 0..info.ec_per_block {
        for b in &ec_blocks {
            final_bytes.push(b[i]);
        }
    }
    let mut final_bits = Bits::default();
    for b in final_bytes {
        final_bits.push(b as u32, 8);
    }

    // Mask with the lowest penalty (the first one on ties).
    let size = 17 + 4 * version as usize;
    let mut best: Option<(u32, u8, Matrix)> = None;
    for mask in 0..8u8 {
        let m = build_matrix(&final_bits.0, ecl, version, mask, size);
        let penalty = m.penalty();
        if best.as_ref().is_none_or(|(p, _, _)| penalty < *p) {
            best = Some((penalty, mask, m));
        }
    }
    let (_, mask, m) = best.ok_or("no mask")?;
    Ok(QrCode { version, mask, size, modules: m.cells.iter().map(|&c| c == 1).collect() })
}

fn reed_solomon(data: &[u8], ec: usize) -> Vec<u8> {
    // GF(256) with the QR polynomial 0x11D.
    let mut exp = [0u8; 512];
    let mut log = [0u8; 256];
    let mut x: u16 = 1;
    for (i, e) in exp.iter_mut().enumerate().take(255) {
        *e = x as u8;
        log[x as usize] = i as u8;
        x <<= 1;
        if x & 0x100 != 0 {
            x ^= 0x11D;
        }
    }
    for i in 255..512 {
        exp[i] = exp[i - 255];
    }
    let mul = |a: u8, b: u8| if a == 0 || b == 0 { 0 } else { exp[log[a as usize] as usize + log[b as usize] as usize] };
    // Generator: product of (x - a^i) for i in 0..ec, highest degree first.
    let mut gen = vec![1u8];
    for &root in exp.iter().take(ec) {
        let mut next = vec![0u8; gen.len() + 1];
        for (j, &g) in gen.iter().enumerate() {
            next[j] ^= g;
            next[j + 1] ^= mul(g, root);
        }
        gen = next;
    }
    let mut rem = vec![0u8; ec];
    for &d in data {
        let factor = d ^ rem[0];
        rem.remove(0);
        rem.push(0);
        for (r, &g) in rem.iter_mut().zip(&gen[1..]) {
            *r ^= mul(g, factor);
        }
    }
    rem
}

const EMPTY: u8 = 2;

struct Matrix {
    size: usize,
    cells: Vec<u8>,
}

impl Matrix {
    fn get(&self, x: usize, y: usize) -> u8 {
        self.cells[y * self.size + x]
    }
    fn set(&mut self, x: usize, y: usize, v: bool) {
        self.cells[y * self.size + x] = v as u8;
    }
    fn is_empty(&self, x: usize, y: usize) -> bool {
        self.get(x, y) == EMPTY
    }

    fn penalty(&self) -> u32 {
        self.rule1(true) + self.rule1(false) + self.rule2() + self.rule3() + self.rule4()
    }

    fn rule1(&self, horizontal: bool) -> u32 {
        let n = self.size;
        let mut penalty = 0;
        for i in 0..n {
            let mut same = 0;
            let mut prev = EMPTY;
            for j in 0..n {
                let bit = if horizontal { self.get(j, i) } else { self.get(i, j) };
                if bit == prev {
                    same += 1;
                } else {
                    if same >= 5 {
                        penalty += 3 + (same - 5);
                    }
                    same = 1;
                    prev = bit;
                }
            }
            if same >= 5 {
                penalty += 3 + (same - 5);
            }
        }
        penalty
    }

    fn rule2(&self) -> u32 {
        let n = self.size;
        let mut count = 0;
        for y in 0..n - 1 {
            for x in 0..n - 1 {
                let v = self.get(x, y);
                if v == self.get(x + 1, y) && v == self.get(x, y + 1) && v == self.get(x + 1, y + 1) {
                    count += 1;
                }
            }
        }
        3 * count
    }

    fn rule3(&self) -> u32 {
        let n = self.size as isize;
        let at = |x: isize, y: isize| self.get(x as usize, y as usize);
        let white_h = |y: isize, from: isize, to: isize| (from.max(0)..to.min(n)).all(|x| at(x, y) != 1);
        let white_v = |x: isize, from: isize, to: isize| (from.max(0)..to.min(n)).all(|y| at(x, y) != 1);
        const PATTERN: [u8; 7] = [1, 0, 1, 1, 1, 0, 1];
        let mut count = 0;
        for y in 0..n {
            for x in 0..n {
                if x + 6 < n
                    && (0..7).all(|k| at(x + k, y) == PATTERN[k as usize])
                    && (white_h(y, x - 4, x) || white_h(y, x + 7, x + 11))
                {
                    count += 1;
                }
                if y + 6 < n
                    && (0..7).all(|k| at(x, y + k) == PATTERN[k as usize])
                    && (white_v(x, y - 4, y) || white_v(x, y + 7, y + 11))
                {
                    count += 1;
                }
            }
        }
        40 * count
    }

    fn rule4(&self) -> u32 {
        let dark = self.cells.iter().filter(|&&c| c == 1).count() as i64;
        let total = self.cells.len() as i64;
        ((dark * 2 - total).abs() * 10 / total) as u32 * 10
    }
}

fn bch(value: u32, poly: u32) -> u32 {
    let msb = |v: u32| 32 - v.leading_zeros();
    let poly_msb = msb(poly);
    let mut value = value << (poly_msb - 1);
    while msb(value) >= poly_msb {
        value ^= poly << (msb(value) - poly_msb);
    }
    value
}

fn mask_bit(mask: u8, x: usize, y: usize) -> bool {
    let t = x * y;
    match mask {
        0 => (y + x) & 1 == 0,
        1 => y & 1 == 0,
        2 => x % 3 == 0,
        3 => (y + x) % 3 == 0,
        4 => ((y / 2) + (x / 3)) & 1 == 0,
        5 => (t & 1) + (t % 3) == 0,
        6 => ((t & 1) + (t % 3)) & 1 == 0,
        _ => ((t % 3) + ((y + x) & 1)) & 1 == 0,
    }
}

fn build_matrix(bits: &[bool], ecl: Ecl, version: u8, mask: u8, size: usize) -> Matrix {
    let mut m = Matrix { size, cells: vec![EMPTY; size * size] };

    // Finder patterns with their separators.
    for (fx, fy) in [(0, 0), (size - 7, 0), (0, size - 7)] {
        for dy in 0..7 {
            for dx in 0..7 {
                let d = (dx as isize - 3).abs().max((dy as isize - 3).abs());
                m.set(fx + dx, fy + dy, d != 2);
            }
        }
    }
    for i in 0..8 {
        for (x, y) in [(i, 7), (size - 8 + i, 7), (i, size - 8), (7, i), (size - 8, i), (7, size - 8 + i)] {
            m.set(x, y, false);
        }
    }
    // Dark module.
    m.set(8, size - 8, true);
    // Alignment patterns.
    let centers = ALIGNMENT[version as usize - 1];
    for &cy in centers {
        for &cx in centers {
            let (cx, cy) = (cx as usize, cy as usize);
            if m.is_empty(cx, cy) {
                for dy in 0..5 {
                    for dx in 0..5 {
                        let d = (dx as isize - 2).abs().max((dy as isize - 2).abs());
                        m.set(cx - 2 + dx, cy - 2 + dy, d != 1);
                    }
                }
            }
        }
    }
    // Timing patterns.
    for i in 8..size - 8 {
        let bit = (i + 1) % 2 == 1;
        if m.is_empty(i, 6) {
            m.set(i, 6, bit);
        }
        if m.is_empty(6, i) {
            m.set(6, i, bit);
        }
    }

    // Format information.
    let info = (ecl.format_bits() << 3) | mask as u32;
    let format = ((info << 10) | bch(info, 0x537)) ^ 0x5412;
    const COORDS: [(usize, usize); 15] =
        [(8, 0), (8, 1), (8, 2), (8, 3), (8, 4), (8, 5), (8, 7), (8, 8), (7, 8), (5, 8), (4, 8), (3, 8), (2, 8), (1, 8), (0, 8)];
    for (i, &(x, y)) in COORDS.iter().enumerate() {
        let bit = (format >> i) & 1 == 1;
        m.set(x, y, bit);
        if i < 8 {
            m.set(size - i - 1, 8, bit);
        } else {
            m.set(8, size - 7 + (i - 8), bit);
        }
    }
    // Version information.
    if version >= 7 {
        let v = ((version as u32) << 12) | bch(version as u32, 0x1F25);
        let mut index: i32 = 17;
        for i in 0..6 {
            for j in 0..3 {
                let bit = (v >> (17 - index)) & 1 == 1;
                index -= 1;
                m.set(i, size - 11 + j, bit);
                m.set(size - 11 + j, i, bit);
            }
        }
    }

    // Data, in the zig-zag order.
    let mut index = 0;
    let mut up = true;
    let mut x = size as isize - 1;
    let mut y = size as isize - 1;
    while x > 0 {
        if x == 6 {
            x -= 1;
        }
        while y >= 0 && y < size as isize {
            for i in 0..2 {
                let xx = (x - i) as usize;
                let yy = y as usize;
                if !m.is_empty(xx, yy) {
                    continue;
                }
                let mut bit = bits.get(index).copied().unwrap_or(false);
                index += 1;
                if mask_bit(mask, xx, yy) {
                    bit = !bit;
                }
                m.set(xx, yy, bit);
            }
            y += if up { -1 } else { 1 };
        }
        up = !up;
        y += if up { -1 } else { 1 };
        x -= 2;
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(q: &QrCode) -> String {
        q.modules.iter().map(|&b| if b { '1' } else { '0' }).collect()
    }

    #[test]
    fn matches_zxing_reference_symbols() {
        // Version, mask and first modules of symbols produced by ZXing 3.4 (UTF-8 hint).
        let q = encode("E310000000001", Ecl::H, None).unwrap();
        assert_eq!((q.version, q.mask), (2, 4));
        assert!(render(&q).starts_with("1111111001101110"));
        let q = encode("0123456789", Ecl::L, None).unwrap();
        assert_eq!((q.version, q.mask), (1, 7));
        assert!(render(&q).starts_with("1111111001011011"));
        let q = encode(
            "https://ecf.dgii.gov.do/ecf/ConsultaTimbre?RncEmisor=131880681&ENCF=E310000000001&MontoTotal=1180.00&CodigoSeguridad=Ab12Cd",
            Ecl::M,
            None,
        )
        .unwrap();
        assert_eq!((q.version, q.mask), (8, 2));
        assert!(render(&q).starts_with("1111111001100001"));
    }

    #[test]
    fn reference_vectors() {
        // Full comparison against ZXing output when the vectors are available locally
        // (QR_VECTORS=dir with cases.tsv and expected.txt).
        let Ok(dir) = std::env::var("QR_VECTORS") else { return };
        let cases = std::fs::read_to_string(format!("{dir}/cases.tsv")).unwrap();
        let expected = std::fs::read_to_string(format!("{dir}/expected.txt")).unwrap();
        for (case, want) in cases.lines().zip(expected.lines()) {
            let (level, text) = case.split_once('\t').unwrap();
            let q = encode(text, Ecl::from_name(level).unwrap(), None).unwrap();
            let got = format!("{} {} {}", q.version, q.mask, render(&q));
            assert_eq!(got, want, "{level} {text:?}");
        }
    }
}
