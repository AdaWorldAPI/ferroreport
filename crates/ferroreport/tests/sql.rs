//! Reports filled from a database: the main query, a subreport and a table with a connection,
//! on a connection that answers from memory and records what it was asked.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ferroreport::sql::{Connection, SqlRows, SqlValue};
use ferroreport::{Format, Renderer, Value};

struct Fake {
    asked: Mutex<Vec<(String, Vec<SqlValue>)>>,
}

impl Connection for Fake {
    fn query<'a>(&'a self, sql: &str, params: &[SqlValue]) -> ferroreport::Result<SqlRows<'a>> {
        self.asked.lock().unwrap().push((sql.to_string(), params.to_vec()));
        let rows: Vec<Vec<(String, serde_json::Value)>> = if sql.contains("FROM department") {
            vec![
                vec![("ID".into(), 1.into()), ("NAME".into(), "Ventas".into())],
                vec![("ID".into(), 2.into()), ("NAME".into(), "Finanzas".into())],
            ]
        } else {
            let dept = match params.first() {
                Some(SqlValue::Int(d)) => *d,
                _ => 0,
            };
            (0..dept + 1)
                .map(|i| vec![("employee".into(), format!("Persona {dept}.{i}").into()), ("salary".into(), "1234.50".into())])
                .collect()
        };
        Ok(Box::new(rows.into_iter().map(Ok)))
    }

    fn placeholder(&self, n: usize) -> String {
        format!("${n}")
    }
}

const MAIN: &str = r#"<jasperReport xmlns="http://jasperreports.sourceforge.net/jasperreports" name="depts" pageWidth="300" pageHeight="400" columnWidth="300" leftMargin="0" rightMargin="0" topMargin="0" bottomMargin="0">
  <parameter name="MIN" class="java.lang.Integer"><defaultValueExpression><![CDATA[1]]></defaultValueExpression></parameter>
  <parameter name="ORDER" class="java.lang.String"><defaultValueExpression><![CDATA["name"]]></defaultValueExpression></parameter>
  <queryString><![CDATA[SELECT id, name FROM department WHERE id >= $P{MIN} ORDER BY $P!{ORDER}]]></queryString>
  <field name="id" class="java.lang.Integer"/>
  <field name="name" class="java.lang.String"/>
  <detail>
    <band height="60">
      <textField><reportElement x="0" y="0" width="300" height="20"/><textFieldExpression><![CDATA[$F{name}]]></textFieldExpression></textField>
      <subreport>
        <reportElement x="0" y="20" width="300" height="40"/>
        <subreportParameter name="DEPT"><subreportParameterExpression><![CDATA[$F{id}]]></subreportParameterExpression></subreportParameter>
        <connectionExpression><![CDATA[$P{REPORT_CONNECTION}]]></connectionExpression>
        <subreportExpression><![CDATA["people.jasper"]]></subreportExpression>
      </subreport>
    </band>
  </detail>
</jasperReport>"#;

const PEOPLE: &str = r#"<jasperReport xmlns="http://jasperreports.sourceforge.net/jasperreports" name="people" pageWidth="300" pageHeight="400" columnWidth="300" leftMargin="0" rightMargin="0" topMargin="0" bottomMargin="0">
  <parameter name="DEPT" class="java.lang.Integer"/>
  <queryString><![CDATA[SELECT employee, salary FROM employee WHERE dept = $P{DEPT}]]></queryString>
  <field name="employee" class="java.lang.String"/>
  <field name="salary" class="java.math.BigDecimal"/>
  <detail>
    <band height="20">
      <textField><reportElement x="0" y="0" width="300" height="20"/><textFieldExpression><![CDATA[$F{employee} + " " + $F{salary}]]></textFieldExpression></textField>
    </band>
  </detail>
</jasperReport>"#;

#[test]
fn fills_reports_and_subreports_from_their_queries() {
    let fake = Arc::new(Fake { asked: Mutex::new(Vec::new()) });
    let mut renderer = Renderer::new().connection(Some(fake.clone()));
    renderer.add_resource("people.jrxml", PEOPLE.as_bytes().to_vec());
    let report = renderer.parse(MAIN).unwrap();
    let mut csv = Vec::new();
    let params: HashMap<String, Value> = HashMap::new();
    let stats = renderer.render_query_to(&report, params, Format::Csv, &mut csv).unwrap();
    assert_eq!(stats.rows, 2);
    let csv = String::from_utf8(csv).unwrap();
    assert!(csv.contains("Ventas") && csv.contains("Finanzas"), "{csv}");
    assert!(csv.contains("Persona 1.1 1234.50") && csv.contains("Persona 2.2 1234.50"), "{csv}");
    let asked = fake.asked.lock().unwrap();
    // The main query: the default of MIN bound, ORDER pasted in.
    assert_eq!(asked[0], ("SELECT id, name FROM department WHERE id >= $1 ORDER BY name".to_string(), vec![SqlValue::Int(1)]));
    // One subreport query per department, with its id bound.
    assert_eq!(asked[1].1, vec![SqlValue::Int(1)]);
    assert_eq!(asked[2].1, vec![SqlValue::Int(2)]);
    assert_eq!(asked.len(), 3);
}

#[test]
fn render_query_to_needs_a_connection_and_a_query() {
    let mut renderer = Renderer::new();
    let report = renderer.parse(MAIN).unwrap();
    assert!(renderer.render_query_to(&report, HashMap::new(), Format::Csv, Vec::new()).is_err());
}
