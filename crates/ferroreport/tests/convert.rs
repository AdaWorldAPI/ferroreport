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

#[test]
fn pdfa_documents_say_what_they_conform_to() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/02-invoice");
    let template = std::fs::read_to_string(dir.join("report.jrxml")).unwrap();
    let rows: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("data.json")).unwrap()).unwrap();
    let params: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("params.json")).unwrap()).unwrap();
    let pdf = ferroreport::Renderer::new()
        .pdfa(ferroreport::pdf::PdfA::parse("pdfa3b"))
        .attach(ferroreport::pdf::Attachment {
            name: "invoice.xml".into(),
            data: std::sync::Arc::new(b"<invoice/>".to_vec()),
            mime: "text/xml".into(),
            description: Some("The invoice, for machines".into()),
            relationship: "Data".into(),
        })
        .render_json(&template, &rows, &params)
        .unwrap();
    let text = String::from_utf8_lossy(&pdf);
    assert!(text.contains("<pdfaid:part>3</pdfaid:part>"));
    assert!(text.contains("<pdfaid:conformance>B</pdfaid:conformance>"));
    assert!(text.contains("/OutputIntents"));
    assert!(text.contains("/GTS_PDFA1"));
    assert!(text.contains("/AFRelationship /Data"));
    assert!(text.contains("/EmbeddedFiles"));
    assert!(text.contains("/ID [<"));
    // PDF/A-2 takes no files that are not PDF/A themselves.
    let refused = ferroreport::Renderer::new()
        .pdfa(ferroreport::pdf::PdfA::parse("2b"))
        .attach(ferroreport::pdf::Attachment {
            name: "a.xml".into(),
            data: std::sync::Arc::new(Vec::new()),
            mime: "text/xml".into(),
            description: None,
            relationship: "Data".into(),
        })
        .render_json(&template, &rows, &params);
    assert!(refused.is_err());
    // A text in a PDF standard font cannot be in a PDF/A document.
    let plain = r#"<jasperReport xmlns="http://jasperreports.sourceforge.net/jasperreports" name="p" whenNoDataType="AllSectionsNoDetail" pageWidth="200" pageHeight="100" columnWidth="200" leftMargin="0" rightMargin="0" topMargin="0" bottomMargin="0">
<title><band height="20"><staticText><reportElement x="0" y="0" width="100" height="20"/><textElement><font fontName="Arial"/></textElement><text><![CDATA[Hello]]></text></staticText></band></title></jasperReport>"#;
    let e = ferroreport::Renderer::new()
        .pdfa(ferroreport::pdf::PdfA::parse("2b"))
        .render_json(plain, &serde_json::json!([]), &serde_json::json!({}))
        .unwrap_err();
    assert!(e.to_string().contains("PDF/A needs every font"), "{e}");
    assert_eq!(ferroreport::pdf::PdfA::parse("PDF/A-1b"), Some(ferroreport::pdf::PdfA { part: 2, level: 'B' }));
    assert_eq!(ferroreport::pdf::PdfA::parse("4f"), None);
}
