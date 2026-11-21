//! Damaged and hostile input must end in an error or a document, never a panic or a hang.
//!
//! Every fixture template is mutated thousands of ways (bytes flipped, pieces cut, repeated
//! or swapped, numbers blown up) with a fixed seed, then parsed and rendered under a page
//! and time limit. The same is done to the data. A coverage-guided fuzzer goes deeper (see
//! `fuzz/`); this runs on every push, on every platform.

use ferroreport::{Format, JsonRows, Limits, Renderer, RowIter};
use std::path::PathBuf;
use std::time::Duration;

/// xorshift64*: small, fast and the same everywhere.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

const NASTY: &[&str] = &[
    "99999999999",
    "-1",
    "0",
    "1e308",
    "NaN",
    "<![CDATA[",
    "]]>",
    "<!--",
    "&#x0;",
    "&amp;",
    "\u{0}",
    "$F{",
    "$V{PAGE_NUMBER}",
    "new java.math.BigDecimal(\"1e999999\")",
    "((((((((((",
    "\"",
    "<band height=\"100000\">",
    "evaluationTime=\"Report\"",
    "isStretchWithOverflow=\"true\"",
    "\u{202e}",
    "💥",
];

fn mutate(src: &[u8], rng: &mut Rng) -> Vec<u8> {
    let mut out = src.to_vec();
    for _ in 0..1 + rng.below(3) {
        if out.is_empty() {
            break;
        }
        let at = rng.below(out.len());
        match rng.below(8) {
            0 => out[at] ^= 1 << rng.below(8),
            1 => {
                let end = (at + 1 + rng.below(40)).min(out.len());
                out.drain(at..end);
            }
            2 => out.truncate(at),
            3 => {
                let end = (at + 1 + rng.below(200)).min(out.len());
                let piece = out[at..end].to_vec();
                for _ in 0..1 + rng.below(4) {
                    out.splice(at..at, piece.iter().copied());
                }
            }
            4 | 5 => {
                let s = NASTY[rng.below(NASTY.len())].as_bytes();
                out.splice(at..at, s.iter().copied());
            }
            6 => {
                // Make a number much larger or negative.
                if let Some(d) = out[at..].iter().position(|b| b.is_ascii_digit()) {
                    let s = NASTY[rng.below(5)].as_bytes();
                    out.splice(at + d..at + d + 1, s.iter().copied());
                }
            }
            _ => {
                let other = rng.below(out.len());
                out.swap(at, other);
            }
        }
    }
    out
}

fn fixtures() -> Vec<(PathBuf, String, String, serde_json::Value)> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
    let mut out = Vec::new();
    for entry in std::fs::read_dir(root).unwrap() {
        let dir = entry.unwrap().path();
        // The 20,000-row fixture adds time, not coverage.
        if !dir.join("report.jrxml").exists() || dir.ends_with("09-large") {
            continue;
        }
        let template = std::fs::read_to_string(dir.join("report.jrxml")).unwrap();
        let data = std::fs::read_to_string(dir.join("data.json")).unwrap_or_else(|_| "[]".into());
        let params =
            std::fs::read_to_string(dir.join("params.json")).map(|p| serde_json::from_str(&p).unwrap()).unwrap_or_default();
        out.push((dir, template, data, params));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    assert!(out.len() >= 30);
    out
}

fn render(dir: &PathBuf, template: &str, data: &[u8], params: &serde_json::Value, format: Format) {
    // If this render panics, leave the input where it can be picked up.
    struct Dump<'a>(&'a str, &'a [u8]);
    impl Drop for Dump<'_> {
        fn drop(&mut self) {
            if std::thread::panicking() {
                let out = std::env::temp_dir();
                let _ = std::fs::write(out.join("ferroreport-crash.jrxml"), self.0);
                let _ = std::fs::write(out.join("ferroreport-crash.json"), self.1);
                eprintln!("the input that panicked is in {}/ferroreport-crash.*", out.display());
            }
        }
    }
    let _dump = Dump(template, data);
    let Ok(report) = ferroreport::parse(template) else { return };
    let Ok(params) = ferroreport::params_from_json(&report, params) else { return };
    let rows: RowIter = Box::new(JsonRows::new(&report, data));
    let limits = Limits { max_pages: Some(60), timeout: Some(Duration::from_secs(5)) };
    if let Some(path) = std::env::var_os("HOSTILE_TRACE") {
        let _ = std::fs::write(path, template);
    }
    let started = std::time::Instant::now();
    let _ = Renderer::new().resource_dir(dir).limits(limits).render_to(&report, rows, params, format, std::io::sink());
    if std::env::var_os("HOSTILE_SLOW").is_some() && started.elapsed() > Duration::from_millis(1500) {
        let n = SLOW.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let out = std::env::temp_dir().join(format!("ferroreport-slow-{n}.jrxml"));
        let _ = std::fs::write(&out, template);
        eprintln!("slow: {:.1}s {format:?} {} -> {}", started.elapsed().as_secs_f32(), dir.display(), out.display());
    }
}

static SLOW: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// `HOSTILE_SEED=<n>` tries other mutations than the ones every push runs.
fn seed() -> u64 {
    std::env::var("HOSTILE_SEED").ok().and_then(|s| s.parse::<u64>().ok()).map_or(0, |n| n.wrapping_mul(0x9E37_79B9_7F4A_7C15))
}

/// Runs on a stack as small as the smallest the engine is used with (1 MB: Windows, wasm).
fn on_small_stack(f: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new().stack_size(1024 * 1024).spawn(f).unwrap().join().expect("a mutated input made the engine panic");
}

#[test]
fn mutated_templates_never_panic() {
    on_small_stack(|| {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ seed());
        let formats = [Format::Pdf, Format::Xlsx, Format::Csv, Format::Html];
        for (dir, template, data, params) in fixtures() {
            for round in 0..120 {
                let bytes = mutate(template.as_bytes(), &mut rng);
                let text = String::from_utf8_lossy(&bytes);
                render(&dir, &text, data.as_bytes(), &params, formats[round % 4]);
            }
        }
    });
}

#[test]
fn mutated_data_never_panics() {
    on_small_stack(|| {
        let mut rng = Rng(0xD1B5_4A32_D192_ED03 ^ seed());
        for (dir, template, data, params) in fixtures() {
            for round in 0..40 {
                let bytes = mutate(data.as_bytes(), &mut rng);
                render(&dir, &template, &bytes, &params, if round % 2 == 0 { Format::Pdf } else { Format::Csv });
                // Parameters get the same treatment through their JSON text.
                let text = mutate(params.to_string().as_bytes(), &mut rng);
                if let Ok(p) = serde_json::from_slice(&text) {
                    render(&dir, &template, data.as_bytes(), &p, Format::Pdf);
                }
            }
        }
    });
}

#[test]
fn sizes_that_are_not_numbers_of_points_are_refused() {
    let page = |attrs: &str, body: &str| {
        format!(
            r#"<jasperReport name="t" pageWidth="300" pageHeight="300" columnWidth="300" leftMargin="0" rightMargin="0" topMargin="0" bottomMargin="0" {attrs}>{body}</jasperReport>"#
        )
    };
    for bad in ["1e3088", "inf", "NaN", "-inf", "99999999999"] {
        let band = format!(r#"<title><band height="{bad}"/></title>"#);
        assert!(ferroreport::parse(&page("", &band)).is_err(), "band height {bad}");
        let el = format!(
            r#"<title><band height="50"><staticText><reportElement x="0" y="0" width="{bad}" height="20"/><text>x</text></staticText></band></title>"#
        );
        assert!(ferroreport::parse(&page("", &el)).is_err(), "element width {bad}");
    }
    // In styled text a size like that is ignored, and the text still renders.
    let styled = page(
        "",
        r#"<title><band height="50"><staticText><reportElement x="0" y="0" width="200" height="20"/>
        <textElement markup="styled"/><text><![CDATA[<style size="1e3088">big</style> <style size="NaN">odd</style>]]></text></staticText></band></title>"#,
    );
    let report = ferroreport::parse(&styled).unwrap();
    let limits = Limits { max_pages: Some(5), timeout: Some(Duration::from_secs(5)) };
    let stats = Renderer::new().limits(limits).render_to(
        &report,
        Box::new(std::iter::once(Ok(Default::default()))),
        Default::default(),
        Format::Pdf,
        std::io::sink(),
    );
    assert_eq!(stats.unwrap().pages, 1);
}

#[test]
fn the_time_limit_reaches_tables_inside_the_report() {
    // A table whose rows are each far taller than a page: slow to lay out, and all of the
    // work happens inside the table. The deadline must still stop it.
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/27-table");
    let template = std::fs::read_to_string(dir.join("report.jrxml")).unwrap();
    let report = ferroreport::parse(&template).unwrap();
    let data = std::fs::read_to_string(dir.join("data.json")).unwrap();
    let rows: RowIter = Box::new(JsonRows::new(&report, data.as_bytes()));
    let limits = Limits { max_pages: None, timeout: Some(Duration::ZERO) };
    let err = Renderer::new().resource_dir(&dir).limits(limits).render_to(
        &report,
        rows,
        Default::default(),
        Format::Pdf,
        std::io::sink(),
    );
    assert!(matches!(err, Err(ferroreport::Error::Limit(_))));
}
