//! Barcode geometry against barcode4j 2.1 itself.
//!
//! `tests/data/barcode4j.txt` holds the SVG that barcode4j generates for each case of
//! `tools/oracle/barcode4j-cases.txt` (regenerate it with `tools/oracle/Barcode4jCorpus.java`).
//! Every bar, the drawing size and every piece of human-readable text must match exactly.

use ferroreport::barcode::linear::{self, Checksum, Kind, Spec};
use ferroreport::barcode::postal::{self, FourState};
use ferroreport::barcode::{
    code128_layout, code128_symbol, datamatrix, datamatrix_symbol, ean128_message, Symbol, SymbolText, TextPosition,
};

struct Case {
    args: String,
    width: f64,
    height: f64,
    rects: Vec<[f64; 4]>,
    texts: Vec<SymbolText>,
}

fn attr(line: &str, name: &str) -> Option<f64> {
    let key = format!(" {name}=\"");
    let start = line.find(&key)? + key.len();
    let end = start + line[start..].find('"')?;
    line[start..end].trim_end_matches("mm").parse().ok()
}

fn corpus() -> Vec<Case> {
    let text = include_str!("data/barcode4j.txt");
    let mut cases: Vec<Case> = Vec::new();
    for line in text.lines() {
        if let Some(args) = line.strip_prefix("@ ") {
            cases.push(Case { args: args.to_string(), width: 0.0, height: 0.0, rects: Vec::new(), texts: Vec::new() });
            continue;
        }
        let case = cases.last_mut().expect("case header first");
        if line.starts_with("<svg") {
            let vb = line.split("viewBox=\"").nth(1).unwrap().split('"').next().unwrap();
            let v: Vec<f64> = vb.split(' ').map(|x| x.parse().unwrap()).collect();
            case.width = v[2];
            case.height = v[3];
        } else if line.starts_with("<rect") {
            let g = |n| attr(line, n).unwrap();
            case.rects.push([g("x"), g("y"), g("width"), g("height")]);
        } else if line.starts_with("<text") {
            let body = line.split('>').nth(1).unwrap().split('<').next().unwrap();
            case.texts.push(SymbolText {
                text: body.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">"),
                x: attr(line, "x").unwrap(),
                y: attr(line, "y").unwrap(),
                size: attr(line, "font-size").unwrap(),
                length: attr(line, "textLength"),
            });
        }
    }
    cases
}

/// Runs one case line: `type message heightPt [moduleWidthPt|-] [textPos|-] [fontSizePt|-] [key=value...]`.
fn generate(args: &str) -> Result<Symbol, String> {
    let a: Vec<&str> = args.split(' ').collect();
    let opt = |i: usize| a.get(i).copied().filter(|v| *v != "-");
    let num = |i: usize| opt(i).map(|v| v.parse::<f64>().unwrap());
    let height: f64 = a[2].parse().unwrap();
    let position = match opt(4) {
        Some("top") => TextPosition::Top,
        Some("none") => TextPosition::None,
        _ => TextPosition::Bottom,
    };
    let font = Some(num(5).unwrap_or(10.0));
    let extras: Vec<(&str, &str)> = a.iter().skip(6).map(|kv| kv.split_once('=').unwrap()).collect();
    let extra = |k: &str| extras.iter().find(|(key, _)| *key == k).map(|(_, v)| *v);
    let extra_num = |k: &str| extra(k).map(|v| v.parse::<f64>().unwrap());
    let mm = |pt: f64| pt / 2.835;
    match a[0] {
        "code128" => return code128_symbol(a[1], num(3), height, position, font, extra_num("quiet"), extra_num("vquiet")),
        "ean128" => {
            let checksum = extra("checksum").and_then(Checksum::from_name).unwrap_or_default();
            let (data, human) = ean128_message(a[1], checksum)?;
            return code128_layout(&data, Some(&human), num(3), height, position, font, extra_num("quiet"), extra_num("vquiet"));
        }
        "pdf417" => {
            let int = |k: &str| extra(k).map(|v| v.parse::<usize>().unwrap());
            let spec = ferroreport::barcode::Pdf417Spec {
                module_width: num(3),
                quiet_zone: extra_num("quiet"),
                vertical_quiet_zone: extra_num("vquiet"),
                min_columns: int("mincols"),
                max_columns: int("maxcols"),
                min_rows: int("minrows"),
                max_rows: int("maxrows"),
                ratio: extra_num("ratio"),
                level: extra("ecl").map(|v| v.parse().unwrap()),
            };
            return ferroreport::barcode::pdf417_symbol(a[1], &spec, height, position, font);
        }
        "datamatrix" => {
            let shape = extra("shape").map(datamatrix::Shape::from_name).unwrap_or_default();
            return datamatrix_symbol(a[1], num(3), shape);
        }
        "postnet" => {
            let short = extra_num("short").map(mm).unwrap_or(1.27);
            return postal::postnet(a[1], extra("pchecksum") == Some("add"), 0.508, 0.508, short, height);
        }
        "royalmail" => {
            let g = FourState {
                bar: num(3).map(mm).unwrap_or(0.53),
                gap: extra_num("fgap").map(mm).unwrap_or(0.53),
                track: extra_num("track").map(mm).unwrap_or(1.25),
                ascender: extra_num("ascender").map(mm).unwrap_or(1.8),
                quiet: 2.0,
                vquiet: 2.0,
                fit_height: true,
            };
            return postal::royal_mail(a[1], extra("checksum") != Some("ignore"), &g, height);
        }
        "usps" => {
            let g = FourState {
                bar: num(3).map(mm).unwrap_or(0.508),
                gap: extra_num("fgap").map(mm).unwrap_or(0.635),
                track: extra_num("track").map(mm).unwrap_or(1.27),
                ascender: extra_num("ascender").map(mm).unwrap_or(1.27),
                quiet: 3.175,
                vquiet: 0.7112,
                fit_height: true,
            };
            return postal::usps(a[1], &g, height);
        }
        _ => {}
    }
    let kind = match a[0] {
        "code39" => Kind::Code39,
        "i25" => Kind::Interleaved2Of5,
        "codabar" => Kind::Codabar,
        "ean13" => Kind::Ean13,
        "ean8" => Kind::Ean8,
        "upca" => Kind::UpcA,
        "upce" => Kind::UpcE,
        other => panic!("unknown type {other}"),
    };
    let start_stop = extra("startstop") == Some("true");
    let spec = Spec {
        module_width: num(3),
        text_position: position,
        quiet_zone: extra_num("quiet"),
        vertical_quiet_zone: extra_num("vquiet"),
        checksum: extra("checksum").and_then(Checksum::from_name).unwrap_or_default(),
        wide_factor: extra_num("wide"),
        display_checksum: extra("dispcheck") == Some("true") || start_stop,
        display_start_stop: start_stop,
        extended: extra("ext") == Some("true"),
        intercharacter_gap: extra_num("gap"),
        ..Spec::new(kind)
    };
    linear::symbol(&spec, a[1], height, font)
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

#[test]
fn every_barcode_matches_barcode4j() {
    let cases = corpus();
    assert!(cases.len() >= 30, "corpus not found");
    let mut failures = Vec::new();
    for case in &cases {
        let sym = match generate(&case.args) {
            Ok(s) => s,
            Err(e) => {
                failures.push(format!("{}: error {e}", case.args));
                continue;
            }
        };
        let mut problems = Vec::new();
        if !close(sym.width, case.width) || !close(sym.height, case.height) {
            problems.push(format!("size {}x{} != {}x{}", sym.width, sym.height, case.width, case.height));
        }
        if sym.rects.len() != case.rects.len() {
            problems.push(format!("{} bars != {}", sym.rects.len(), case.rects.len()));
        }
        if let Some((i, (got, want))) =
            sym.rects.iter().zip(&case.rects).enumerate().find(|(_, (g, w))| (0..4).any(|k| !close(g[k], w[k])))
        {
            problems.push(format!("bar {i}: {got:?} != {want:?}"));
        }
        if sym.texts != case.texts {
            problems.push(format!("text {:?} != {:?}", sym.texts, case.texts));
        }
        if !problems.is_empty() {
            failures.push(format!("{}: {}", case.args, problems.join("; ")));
        }
    }
    assert!(failures.is_empty(), "{} of {} cases differ:\n{}", failures.len(), cases.len(), failures.join("\n"));
}

#[test]
fn bad_input_is_reported() {
    let spec = Spec::new(Kind::Ean13);
    assert!(linear::symbol(&spec, "12345", 50.0, None).unwrap_err().contains("12 digits"));
    assert!(linear::symbol(&spec, "5901234123458", 50.0, None).unwrap_err().contains("bad checksum"));
    assert!(linear::symbol(&Spec::new(Kind::Interleaved2Of5), "12a4", 40.0, None).is_err());
    assert!(linear::symbol(&Spec::new(Kind::Code39), "ñ", 40.0, None).is_err());
}
