//! Barcodes drawn like the JasperReports barcode components (barcode4j and ZXing).
//!
//! Each symbol is laid out as rectangles in the units JasperReports uses for its SVG
//! (pixels for QR codes, millimetres for barcode4j), so they can be scaled into the element
//! exactly like Jasper scales its SVG image.

pub mod code128;
pub mod datamatrix;
pub mod linear;
pub mod postal;
pub mod qr;
mod qr_tables;

/// A symbol ready to scale into its element: filled rectangles `[x, y, w, h]` and optional
/// human-readable text, in a `width`×`height` drawing.
#[derive(Debug, Clone, Default)]
pub struct Symbol {
    pub width: f64,
    pub height: f64,
    pub rects: Vec<[f64; 4]>,
    pub texts: Vec<SymbolText>,
}

/// Text centered on `x` with its baseline at `y`. With `length`, the text is spread to exactly
/// that width (SVG `textLength`, used for the digit groups of UPC and EAN codes).
#[derive(Debug, Clone, PartialEq)]
pub struct SymbolText {
    pub text: String,
    pub x: f64,
    pub y: f64,
    pub size: f64,
    pub length: Option<f64>,
}

/// Rounds like barcode4j's SVG output (`DecimalFormat("0.####")`).
pub(crate) fn r4(v: f64) -> f64 {
    format!("{v:.4}").parse().unwrap_or(v)
}

/// The QR code of `content` laid out like JasperReports' `QRCodeSVGImageProducer` for a box
/// of `avail_w`×`avail_h` points.
pub fn qr_symbol(
    content: &str,
    ecl: qr::Ecl,
    margin: Option<u32>,
    version: Option<u8>,
    avail_w: i32,
    avail_h: i32,
) -> Result<Symbol, String> {
    let code = qr::encode(content, ecl, version)?;
    let margin = margin.unwrap_or(4) as i32;
    let n = code.size as i32;
    let (qw, qh) = (n + 2 * margin, n + 2 * margin);
    // Jasper sizes the drawing for a 300 dpi rendering of the box.
    let resolution = 300.0;
    let pw = (avail_w as f64 * (resolution / 72.0)).ceil() as i32;
    let ph = (avail_h as f64 * (resolution / 72.0)).ceil() as i32;
    let sx = pw as f64 / qw as f64;
    let sy = ph as f64 / qh as f64;
    let mult = 1.max(sx.ceil().min(sy.ceil()) as i32);
    let (out_w, out_h) = (mult * qw, mult * qh);
    let scale = 1f64.max((out_w as f64 / pw as f64).max(out_h as f64 / ph as f64));
    let (svg_w, svg_h) = ((scale * pw as f64).ceil() as i32, (scale * ph as f64).ceil() as i32);
    let off_x = 0.max((svg_w - out_w) / 2);
    let off_y = 0.max((svg_h - out_h) / 2);
    let mut rects = Vec::new();
    for x in 0..code.size {
        for y in 0..code.size {
            if code.get(x, y) {
                let rx = off_x + mult * (x as i32 + margin);
                let ry = off_y + mult * (y as i32 + margin);
                rects.push([rx as f64, ry as f64, mult as f64, mult as f64]);
            }
        }
    }
    Ok(Symbol { width: svg_w as f64, height: svg_h as f64, rects, texts: Vec::new() })
}

/// barcode4j's points-to-millimetres conversion.
pub fn pt2mm(pt: f64) -> f64 {
    pt / 2.835
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextPosition {
    None,
    Bottom,
    Top,
}

/// The data and human-readable text of an EAN-128 (GS1-128) message: application
/// identifiers and their data, either bracketed (`(01)0950...(10)AB`) or run together, where
/// the known identifiers tell how long each field is. A check digit is added to the fields
/// that have one (SSCC, GTIN, location numbers) when it is missing, and verified with `check`.
pub fn ean128_message(msg: &str, checksum: linear::Checksum) -> Result<(String, String), String> {
    const FNC1: char = '\u{F1}';
    // (identifier, data length when fixed, whether its last digit is a check digit)
    fn ai_info(ai: &str) -> (Option<usize>, bool) {
        match ai {
            "00" => (Some(18), true),
            "01" | "02" | "03" => (Some(14), true),
            "04" => (Some(16), false),
            "11" | "12" | "13" | "14" | "15" | "16" | "17" | "18" | "19" => (Some(6), false),
            "20" => (Some(2), false),
            "402" => (Some(17), true),
            "410" | "411" | "412" | "413" | "414" | "415" | "416" | "417" => (Some(13), true),
            "7001" => (Some(13), false),
            "8005" => (Some(6), false),
            "8006" | "8017" | "8018" => (Some(18), false),
            ai if ai.len() == 4 && ("31".."37").contains(&&ai[..2]) => (Some(6), false),
            _ => (None, false),
        }
    }
    fn ai_len(s: &str) -> usize {
        match s.get(..2).unwrap_or("") {
            "23" | "24" | "25" | "40" | "41" | "42" => 3,
            "31" | "32" | "33" | "34" | "35" | "36" | "39" | "70" | "71" | "72" | "80" | "81" | "82" => 4,
            _ => 2,
        }
    }
    // Split into (identifier, data).
    let mut fields: Vec<(String, String)> = Vec::new();
    if msg.starts_with('(') {
        let mut rest = msg;
        while let Some(r) = rest.strip_prefix('(') {
            let close = r.find(')').ok_or("EAN-128: an application identifier is not closed")?;
            let ai = r[..close].to_string();
            let after = &r[close + 1..];
            let end = after.find('(').unwrap_or(after.len());
            fields.push((ai, after[..end].replace(['\u{1d}', FNC1], "")));
            rest = &after[end..];
        }
    } else {
        let mut rest: &str = msg;
        while !rest.is_empty() {
            rest = rest.trim_start_matches(['\u{1d}', FNC1]);
            if rest.is_empty() {
                break;
            }
            let n = ai_len(rest).min(rest.len());
            let ai = &rest[..n];
            let body = &rest[n..];
            let len = match ai_info(ai).0 {
                Some(l) => l.min(body.len()),
                None => body.find(['\u{1d}', FNC1]).unwrap_or(body.len()),
            };
            // A fixed field one digit short gets its check digit added below.
            fields.push((ai.to_string(), body[..len].to_string()));
            rest = &body[len..];
        }
    }
    if fields.is_empty() {
        return Err("the EAN-128 message is empty".into());
    }
    let (mut data, mut text) = (String::from(FNC1), String::new());
    let count = fields.len();
    for (i, (ai, mut value)) in fields.into_iter().enumerate() {
        if !ai.bytes().all(|b| b.is_ascii_digit()) {
            return Err(format!("EAN-128: `{ai}` is not an application identifier"));
        }
        let (fixed, has_check) = ai_info(&ai);
        if let (Some(len), true) = (fixed, has_check) {
            let digits: Option<Vec<u8>> = value.bytes().map(|b| b.is_ascii_digit().then(|| b - b'0')).collect();
            if let Some(d) = digits {
                let sum: u32 = d.iter().rev().enumerate().map(|(k, &x)| x as u32 * if k % 2 == 0 { 3 } else { 1 }).sum();
                if d.len() == len - 1 && checksum != linear::Checksum::Ignore {
                    value.push((b'0' + ((10 - sum % 10) % 10) as u8) as char);
                } else if d.len() == len && checksum == linear::Checksum::Check {
                    let body = &d[..len - 1];
                    let s: u32 = body.iter().rev().enumerate().map(|(k, &x)| x as u32 * if k % 2 == 0 { 3 } else { 1 }).sum();
                    let expected = ((10 - s % 10) % 10) as u8;
                    if d[len - 1] != expected {
                        return Err(format!("Message '{msg}' has a bad checksum. Expected: {expected}"));
                    }
                }
            }
        }
        data.push_str(&ai);
        data.push_str(&value);
        if fixed.is_none() && i + 1 < count {
            data.push(FNC1);
        }
        text.push_str(&format!("({ai}){value}"));
    }
    Ok((data, text))
}

/// Code 128 laid out like barcode4j's `Code128Bean` in millimetres.
pub fn code128_symbol(
    msg: &str,
    module_width_pt: Option<f64>,
    height_pt: f64,
    position: TextPosition,
    font_size_pt: Option<f64>,
    quiet_zone_pt: Option<f64>,
    vertical_quiet_zone_pt: Option<f64>,
) -> Result<Symbol, String> {
    code128_layout(msg, None, module_width_pt, height_pt, position, font_size_pt, quiet_zone_pt, vertical_quiet_zone_pt)
}

/// Code 128 with the human-readable text given (EAN-128 shows its identifiers in brackets).
#[allow(clippy::too_many_arguments)]
pub fn code128_layout(
    msg: &str,
    human: Option<&str>,
    module_width_pt: Option<f64>,
    height_pt: f64,
    position: TextPosition,
    font_size_pt: Option<f64>,
    quiet_zone_pt: Option<f64>,
    vertical_quiet_zone_pt: Option<f64>,
) -> Result<Symbol, String> {
    let widths = code128::modules(msg)?;
    let module = module_width_pt.map(pt2mm).unwrap_or(0.21);
    // The default quiet zone is fixed when the bean is created (10 × 0.21 mm).
    let quiet = quiet_zone_pt.map(pt2mm).unwrap_or(2.1);
    let vquiet = vertical_quiet_zone_pt.map(pt2mm).unwrap_or(0.0);
    let font = font_size_pt.map(pt2mm).unwrap_or(2.8222);
    let text_height = if position == TextPosition::None { 0.0 } else { 1.3 * font };
    let bar_height = pt2mm(height_pt) - text_height;
    let bars_y = vquiet + if position == TextPosition::Top { text_height } else { 0.0 };
    let mut x = quiet;
    let mut rects = Vec::new();
    for (i, &w) in widths.iter().enumerate() {
        let bw = module * w as f64;
        if i % 2 == 0 {
            rects.push([r4(x), r4(bars_y), r4(bw), r4(bar_height)]);
        }
        x += bw;
    }
    let width = x + quiet;
    let height = bar_height + text_height + 2.0 * vquiet;
    let texts = match position {
        TextPosition::None => Vec::new(),
        _ => {
            let base = if position == TextPosition::Top { text_height } else { bar_height + text_height };
            // barcode4j shifts the baseline by 0.2 × pt2mm(size in mm).
            let y = base - text_height / 13.0 * 3.0 - pt2mm(font) * 0.2;
            vec![SymbolText {
                text: human.map(str::to_string).unwrap_or_else(|| code128::human_readable(msg)),
                x: r4(quiet + (x - quiet) / 2.0),
                y: r4(y),
                size: r4(font),
                length: None,
            }]
        }
    };
    Ok(Symbol { width: r4(width), height: r4(height), rects, texts })
}

/// A Data Matrix symbol like barcode4j's `DataMatrixBean`: one square per dark module, with a
/// quiet zone of one point (the bean's default module width, kept when the width changes).
pub fn datamatrix_symbol(msg: &str, module_width_pt: Option<f64>, shape: datamatrix::Shape) -> Result<Symbol, String> {
    let (m, rows, cols) = datamatrix::matrix(msg, shape)?;
    // barcode4j's default module: one point, as inches (UnitConv.in2mm(1 / 72)).
    let quiet = 25.4 / 72.0;
    let mw = module_width_pt.map(pt2mm).unwrap_or(quiet);
    let mut rects = Vec::new();
    for r in 0..rows {
        for c in 0..cols {
            if m[r * cols + c] {
                rects.push([r4(quiet + c as f64 * mw), r4(quiet + r as f64 * mw), r4(mw), r4(mw)]);
            }
        }
    }
    Ok(Symbol { width: r4(2.0 * quiet + cols as f64 * mw), height: r4(2.0 * quiet + rows as f64 * mw), rects, texts: Vec::new() })
}
