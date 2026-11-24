//! The `.ferro` format stays compatible within 1.x: a template written for 1.0 compiles to the
//! JRXML that 1.0.2 compiled it to, byte for byte, and so renders the same.
//!
//! `tests/data/ferro-1.0/` holds the templates of the examples, the documentation and the README
//! as 1.0 shipped them, and a few that sit close to the short forms added later (braces in
//! expressions, `text` lines, attribute names). Next to each is the JRXML 1.0.2 made of it.
//! `fixtures.txt` holds, for every fixture, a hash of its `.ferro` (as `convert` writes it) and
//! of the JRXML that `.ferro` compiles to.
//!
//! The expected files were written by 1.0.2 with `FERRO_BLESS=1 cargo test --test ferro_compat`;
//! they are not meant to be blessed again.

use ferroreport::{jrxml, template};
use std::path::{Path, PathBuf};

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/ferro-1.0")
}

fn bless() -> bool {
    std::env::var_os("FERRO_BLESS").is_some()
}

fn compile(source: &str, dir: &Path) -> String {
    let load = |l: &str| std::fs::read(dir.join(l)).ok();
    template::to_jrxml_with(source, Some(&load)).unwrap()
}

/// The report as the renderer reads it (or the error it gives: some edge templates hold
/// expressions ferroreport does not evaluate, and must fail the same way).
fn described(source: &str, dir: &Path) -> String {
    let load = |l: &str| std::fs::read(dir.join(l)).ok();
    match jrxml::parse_report_with(source, &load) {
        Ok(mut report) => {
            report.warnings.clear();
            format!("{report:?}")
        }
        Err(_) => "error".to_string(),
    }
}

/// The JRXML differences 1.1 makes on purpose, as (file, 1.0.2 wrote, 1.1 writes). Each one
/// leaves what ferroreport renders as it was (checked below against the 1.0.2 JRXML).
///
/// - `size=` on a text element or a style: ferroreport has always read it as the font size,
///   and 1.1 writes it as `fontSize=`, the attribute JasperReports reads too.
const INTENDED: &[(&str, &str, &str)] = &[("edge-braces.ferro", r#" size="12">"#, r#" fontSize="12">"#)];

/// FNV-1a, 64 bits: enough to tell two texts apart.
fn hash(s: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

#[test]
fn templates_written_for_1_0_compile_as_they_did() {
    let dir = corpus();
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "ferro"))
        .collect();
    files.sort();
    assert!(files.len() >= 10, "{files:?}");
    for path in files {
        let source = std::fs::read_to_string(&path).unwrap();
        let compiled = compile(&source, &dir);
        let expected_path = path.with_extension("jrxml");
        if bless() {
            std::fs::write(&expected_path, &compiled).unwrap();
            continue;
        }
        let expected = std::fs::read_to_string(&expected_path).unwrap();
        let file = path.file_name().unwrap().to_string_lossy();
        let mut now = expected.clone();
        for (_, old, new) in INTENDED.iter().filter(|(f, _, _)| *f == file) {
            assert!(now.contains(old), "{file}: {old}");
            now = now.replacen(old, new, 1);
        }
        assert_eq!(compiled, now, "{} compiles differently from 1.0.2", path.display());
        // And the renderer reads the template as it read the JRXML.
        if expected.contains("<jasperReport") {
            assert_eq!(described(&source, &dir), described(&expected, &dir), "{}", path.display());
        }
    }
}

#[test]
fn fixtures_in_ferro_compile_as_they_did() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
    let mut dirs: Vec<PathBuf> =
        std::fs::read_dir(&root).unwrap().flatten().map(|e| e.path()).filter(|p| p.join("report.jrxml").exists()).collect();
    dirs.sort();
    let mut lines = String::new();
    for dir in &dirs {
        let original = std::fs::read_to_string(dir.join("report.jrxml")).unwrap();
        let ferro = template::from_jrxml(&original).unwrap();
        let compiled = template::to_jrxml(&ferro).unwrap();
        let name = dir.file_name().unwrap().to_string_lossy();
        lines.push_str(&format!("{name} {} {}\n", hash(&ferro), hash(&compiled)));
    }
    let expected_path = corpus().join("fixtures.txt");
    if bless() {
        std::fs::write(&expected_path, &lines).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&expected_path).unwrap();
    for (got, want) in lines.lines().zip(expected.lines()) {
        assert_eq!(got, want, "fixture written or compiled differently from 1.0.2");
    }
    assert_eq!(lines.lines().count(), expected.lines().count());
}
