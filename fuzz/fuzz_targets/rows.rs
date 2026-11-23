//! Any bytes as the rows of a report (a JSON array or JSON Lines), streamed as the engine
//! streams a request's rows.
#![no_main]
use libfuzzer_sys::fuzz_target;

const TEMPLATE: &str = r#"<jasperReport name="r" pageWidth="300" pageHeight="200" columnWidth="300" leftMargin="0" rightMargin="0" topMargin="0" bottomMargin="0">
  <field name="s" class="java.lang.String"/><field name="n" class="java.math.BigDecimal"/>
  <field name="d" class="java.util.Date"/><field name="l" class="java.util.List"/><field name="b" class="java.lang.Boolean"/>
  <detail><band height="12"><textField isStretchWithOverflow="true"><reportElement x="0" y="0" width="300" height="12"/>
    <textFieldExpression><![CDATA[$F{s} + " " + $F{n} + " " + $F{d} + " " + $F{b}]]></textFieldExpression></textField></band></detail>
</jasperReport>"#;

fuzz_target!(|data: &[u8]| {
    let report = ferroreport::parse(TEMPLATE).unwrap();
    let limits = ferroreport::Limits { max_pages: Some(20), timeout: Some(std::time::Duration::from_secs(2)) };
    let rows: ferroreport::RowIter = Box::new(ferroreport::JsonRows::new(&report, data).max_row_bytes(1 << 16));
    let _ = ferroreport::Renderer::new().limits(limits).render_to(&report, rows, Default::default(), ferroreport::Format::Pdf, std::io::sink());
});
