//! Barcodes drawn like the JasperReports barcode components (barcode4j and ZXing).
//!
//! Each symbol is laid out as rectangles in the units JasperReports uses for its SVG
//! (pixels for QR codes, millimetres for barcode4j), so they can be scaled into the element
//! exactly like Jasper scales its SVG image.

pub mod code128;
pub mod linear;
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
                text: code128::human_readable(msg),
                x: r4(quiet + (x - quiet) / 2.0),
                y: r4(y),
                size: r4(font),
                length: None,
            }]
        }
    };
    Ok(Symbol { width: r4(width), height: r4(height), rects, texts })
}
