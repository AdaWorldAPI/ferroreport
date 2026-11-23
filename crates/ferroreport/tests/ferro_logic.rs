//! `.ferro` logic: `import`, `if` and `each` (examples/invoices.ferro, which JasperReports
//! 7.1.1 renders pixel for pixel like ferroreport once converted to JRXML).

use std::path::PathBuf;

fn examples() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}

#[test]
fn imports_conditions_and_repeated_rows() {
    let dir = examples();
    let source = std::fs::read_to_string(dir.join("invoices.ferro")).unwrap();
    let rows: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("invoices.json")).unwrap()).unwrap();
    let mut renderer = ferroreport::Renderer::new().resource_dir(&dir);
    let report = renderer.parse(&source).unwrap();
    let records = ferroreport::data::records_from_json(&report, &rows).unwrap();
    let doc = renderer.fill(&report, &records, Default::default()).unwrap();
    let csv = renderer.to_csv(&doc, &Default::default());
    assert_eq!(
        csv,
        "Invoices,,\nF-001 · Caribe SRL,,\nOverdue: send a reminder,,\nCable,2,7.00\nRouter,1,89.90\n\
         F-002 · Ana Pérez,,\nMouse,3,36.00\nF-003 · Luis Gómez,,\nOverdue: send a reminder,,\n"
    );
}

#[test]
fn an_import_needs_somewhere_to_look() {
    let source = "report \"R\"\n  import \"parts.ferro\"\n";
    assert!(ferroreport::template::to_jrxml(source).unwrap_err().to_string().contains("import"));
    let looping = |_: &str| Some(b"styles\n  import \"self.ferro\"\n".to_vec());
    let err = ferroreport::template::to_jrxml_with("report \"R\"\n  import \"self.ferro\"\n", Some(&looping)).unwrap_err();
    assert!(err.to_string().contains("16 deep"), "{err}");
}

#[test]
fn if_and_each_need_their_expression() {
    for bad in
        ["report \"R\"\n  detail\n    if\n      staticText : x\n", "report \"R\"\n  detail\n    each\n      field a String\n"]
    {
        assert!(ferroreport::template::to_jrxml(bad).is_err(), "{bad}");
    }
}
