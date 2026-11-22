//! Every fixture read three ways gives the same report: as written (JRXML 6), rewritten as
//! JRXML 7, and rewritten in ferroreport's own format.

use ferroreport::{convert, jrxml, template};
use std::path::Path;

fn fixtures() -> Vec<std::path::PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
    let mut out: Vec<_> =
        std::fs::read_dir(&root).unwrap().flatten().map(|e| e.path()).filter(|p| p.join("report.jrxml").exists()).collect();
    out.sort();
    out
}

fn described(source: &str, dir: &Path) -> String {
    let load = |l: &str| std::fs::read(dir.join(l)).ok();
    let mut report = jrxml::parse_report_with(source, &load).unwrap();
    report.warnings.clear();
    format!("{report:?}")
}

/// Fails with the place where two descriptions part ways.
fn same(a: &str, b: &str, what: &str) {
    if a != b {
        let at = a.bytes().zip(b.bytes()).position(|(x, y)| x != y).unwrap_or(a.len().min(b.len()));
        let from = at.saturating_sub(300);
        let cut = |s: &str| s.get(from..(at + 200).min(s.len())).unwrap_or("").to_string();
        panic!("{what}: differs at byte {at}\n  converted: ...{}\n  original:  ...{}", cut(a), cut(b));
    }
}

#[test]
fn fixtures_read_the_same_in_every_format() {
    let mut checked = 0;
    for dir in fixtures() {
        let original = std::fs::read_to_string(dir.join("report.jrxml")).unwrap();
        let expected = described(&original, &dir);
        let v7 = convert::jrxml_to_v7(&original).unwrap();
        assert_eq!(convert::jrxml_version(&v7).unwrap(), convert::JrxmlVersion::V7);
        same(&described(&v7, &dir), &expected, &format!("{}: JRXML 7", dir.display()));
        let ferro = template::from_jrxml(&original).unwrap();
        assert!(template::is_template(&ferro));
        same(&described(&ferro, &dir), &expected, &format!("{}: .ferro", dir.display()));
        // Converting again changes nothing.
        let compiled = template::to_jrxml(&ferro).unwrap();
        assert_eq!(template::from_jrxml(&compiled).unwrap(), ferro, "{}", dir.display());
        assert_eq!(template::from_jrxml(&v7).unwrap(), ferro, "{}", dir.display());
        checked += 1;
    }
    assert!(checked >= 45, "{checked} fixtures");
}
