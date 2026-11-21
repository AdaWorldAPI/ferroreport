//! Renders every fixture and checks the page count recorded from JasperReports.
//! Pixel-level comparison against Jasper lives in `tools/compare.py`.

use std::path::PathBuf;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

#[test]
fn fixtures_render_with_expected_page_counts() {
    let mut checked = 0;
    for entry in std::fs::read_dir(fixtures_dir()).unwrap() {
        let dir = entry.unwrap().path();
        let expected = dir.join("expected.json");
        if !expected.exists() {
            continue;
        }
        let template = std::fs::read_to_string(dir.join("report.jrxml")).unwrap();
        let read = |name: &str, default: &str| -> serde_json::Value {
            let p = dir.join(name);
            serde_json::from_str(&if p.exists() { std::fs::read_to_string(p).unwrap() } else { default.to_string() }).unwrap()
        };
        let report = ferroreport::parse(&template).unwrap();
        let records = ferroreport::data::records_from_json(&report, &read("data.json", "[]")).unwrap();
        let params = ferroreport::params_from_json(&report, &read("params.json", "{}")).unwrap();
        let mut renderer = ferroreport::Renderer::new().resource_dir(&dir);
        let doc = renderer.fill(&report, &records, params).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
        let want = read("expected.json", "{}")["pages"].as_u64().unwrap() as usize;
        assert_eq!(doc.pages.len(), want, "{}", dir.display());
        let pdf = renderer.to_pdf(&doc).unwrap();
        assert!(pdf.starts_with(b"%PDF-"));
        checked += 1;
    }
    assert!(checked >= 8, "only {checked} fixtures checked");
}
