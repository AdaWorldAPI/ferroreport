//! Subreports: data sources from nested JSON, return values, and the safety limits.

use ferroreport::Renderer;
use std::path::PathBuf;

const MAIN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<jasperReport xmlns="http://jasperreports.sourceforge.net/jasperreports" name="main" pageWidth="595" pageHeight="842" columnWidth="555" leftMargin="20" rightMargin="20" topMargin="20" bottomMargin="20">
	<field name="name" class="java.lang.String"/>
	<variable name="items" class="java.lang.Integer" calculation="System"/>
	<detail><band height="20">
		<textField><reportElement x="0" y="0" width="200" height="14"/><textFieldExpression><![CDATA[$F{name}]]></textFieldExpression></textField>
		<subreport><reportElement x="0" y="14" width="300" height="6"/>
			<dataSourceExpression><![CDATA[DATA_SOURCE]]></dataSourceExpression>
			<returnValue subreportVariable="REPORT_COUNT" toVariable="items"/>
			<subreportExpression><![CDATA[LOCATION]]></subreportExpression>
		</subreport>
	</band></detail>
	<summary><band height="14"><textField><reportElement x="0" y="0" width="200" height="14"/><textFieldExpression><![CDATA["last count " + $V{items}]]></textFieldExpression></textField></band></summary>
</jasperReport>"#;

const SUB: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<jasperReport xmlns="http://jasperreports.sourceforge.net/jasperreports" name="sub" pageWidth="300" pageHeight="842" columnWidth="300" leftMargin="0" rightMargin="0" topMargin="0" bottomMargin="0">
	<field name="sku" class="java.lang.String"/>
	<detail><band height="12"><textField><reportElement x="10" y="0" width="200" height="12"/><textFieldExpression><![CDATA["item " + $F{sku}]]></textFieldExpression></textField></band></detail>
</jasperReport>"#;

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ferroreport-sub-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("sub.jrxml"), SUB).unwrap();
    d
}

fn render(name: &str, data_source: &str, location: &str) -> ferroreport::Result<String> {
    let d = dir(name);
    let template = MAIN.replace("DATA_SOURCE", data_source).replace("LOCATION", location);
    std::fs::write(d.join("self.jrxml"), &template).unwrap();
    let rows = serde_json::json!([
        {"name": "A", "items": [{"sku": "a1"}, {"sku": "a2"}, {"sku": "a3"}]},
        {"name": "B", "items": [{"sku": "b1"}]}
    ]);
    let mut r = Renderer::new().resource_dir(&d);
    let report = ferroreport::parse(&template)?;
    let records = ferroreport::data::records_from_json(&report, &rows)?;
    let doc = r.fill(&report, &records, Default::default())?;
    let mut text = String::new();
    for page in &doc.pages {
        for item in &page.items {
            let item = match item {
                ferroreport::print::PrintItem::Cell(_, inner) => inner.as_ref(),
                other => other,
            };
            if let ferroreport::print::PrintItem::Text(t) = item {
                for l in &t.lines {
                    text.push_str(&l.text);
                    text.push('\n');
                }
            }
        }
    }
    Ok(text)
}

#[test]
fn json_sub_data_source_and_return_values() {
    let text = render(
        "json",
        r#"((net.sf.jasperreports.engine.data.JsonDataSource)$P{REPORT_DATA_SOURCE}).subDataSource("items")"#,
        r#""sub.jasper""#,
    )
    .unwrap();
    assert_eq!(text, "A\nitem a1\nitem a2\nitem a3\nB\nitem b1\nlast count 1\n");
}

#[test]
fn undeclared_field_in_data_source_is_an_error() {
    let text =
        render("collection", "new net.sf.jasperreports.engine.data.JRMapCollectionDataSource($F{items})", r#""sub.jasper""#);
    // `items` is not a declared field of the main report.
    assert!(text.is_err());
}

#[test]
fn runaway_recursion_is_stopped() {
    let err = render("recursion", "new net.sf.jasperreports.engine.JREmptyDataSource(1)", r#""self.jasper""#).unwrap_err();
    assert!(err.to_string().contains("nested more than"), "{err}");
}

#[test]
fn templates_outside_the_resource_dirs_are_refused() {
    for location in [r#""../sub.jasper""#, r#""/etc/passwd""#, r#""https://example.com/x.jasper""#] {
        let err = render("escape", "new net.sf.jasperreports.engine.JREmptyDataSource(1)", location).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("relative") || msg.contains("remote"), "{location}: {msg}");
    }
}

#[test]
fn resources_can_be_given_in_memory() {
    let template = MAIN
        .replace(
            "DATA_SOURCE",
            r#"((net.sf.jasperreports.engine.data.JsonDataSource)$P{REPORT_DATA_SOURCE}).subDataSource("items")"#,
        )
        .replace("LOCATION", r#""sub.jasper""#);
    let rows = serde_json::json!([{"name": "A", "items": [{"sku": "a1"}]}]);
    let pdf = Renderer::new()
        .resource("sub.jrxml", SUB.as_bytes().to_vec())
        .render_json(&template, &rows, &serde_json::json!({}))
        .unwrap();
    assert!(pdf.starts_with(b"%PDF"));
    // Paths that climb out are refused for in-memory resources too.
    let escaping = template.replace(r#""sub.jasper""#, r#""../sub.jasper""#);
    let err =
        Renderer::new().resource("sub.jrxml", SUB.as_bytes().to_vec()).render_json(&escaping, &rows, &serde_json::json!({}));
    assert!(err.is_err());
}
