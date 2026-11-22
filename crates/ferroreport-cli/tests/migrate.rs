//! `ferroreport migrate` over the fixtures: every template loads, and a report is written.

use std::process::Command;

#[test]
fn migrate_checks_and_converts_a_folder_of_templates() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = std::env::temp_dir().join(format!("ferroreport-migrate-{}", std::process::id()));
    let report = out.join("report.json");
    std::fs::create_dir_all(&out).unwrap();
    let status = Command::new(env!("CARGO_BIN_EXE_ferroreport"))
        .current_dir(&root)
        .args(["migrate", "fixtures", "--report"])
        .arg(&report)
        .args(["--to", "ferro", "--out"])
        .arg(out.join("converted"))
        .output()
        .unwrap();
    assert!(status.status.success(), "{}", String::from_utf8_lossy(&status.stderr));
    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&report).unwrap()).unwrap();
    assert_eq!(json["failed"], 0);
    let results = json["results"].as_array().unwrap();
    assert!(results.len() >= 40);
    for r in results {
        let converted = r["converted"].as_str().expect("every template is converted");
        assert!(std::fs::read_to_string(converted).unwrap().starts_with("report "), "{converted}");
    }
    std::fs::remove_dir_all(&out).ok();
}
