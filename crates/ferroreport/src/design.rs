//! Templates as a JSON tree, for editors such as the visual designer.
//!
//! [`to_tree`] reads a template in any format the engine knows (JRXML 6 or 7, or `.ferro`) and
//! gives it as JasperReports 7 writes it, one JSON object per XML element:
//!
//! ```json
//! { "tag": "element", "attrs": [["kind", "textField"], ["x", "0"], ["y", "0"]],
//!   "children": [ { "tag": "expression", "attrs": [], "children": [], "text": "$F{name}" } ] }
//! ```
//!
//! Attributes are `[name, value]` pairs, in the order they are written (JSON objects do not keep
//! an order everywhere); [`from_tree`] also takes them as an object. An editor changes the tree and [`from_tree`] writes it back as JRXML 7 (which
//! [`crate::template::from_jrxml`] turns into `.ferro`). Nothing is lost on the way: parts the
//! editor does not know stay in the tree as they were.

use serde_json::{json, Value};

use crate::convert::{self, JrxmlVersion};
use crate::error::{Error, Result};
use crate::xml::{self, Node};

/// Deepest nesting [`from_tree`] accepts, as for templates read from text.
const MAX_DEPTH: usize = 100;

/// Reads a template (JRXML 6 or 7, or `.ferro`) as a JSON tree of its JRXML 7 form. `load`
/// reads the files a `.ferro` template imports (`import "parts.ferro"`).
pub fn to_tree(source: &str, load: Option<crate::jrxml::Loader>) -> Result<Value> {
    let v7 = if crate::template::is_template(source) {
        crate::template::to_jrxml_with(source, load)?
    } else {
        match convert::jrxml_version(source)? {
            JrxmlVersion::V6 => convert::jrxml_to_v7(source)?,
            JrxmlVersion::V7 => source.to_string(),
        }
    };
    let root = xml::parse_raw(&v7)?;
    Ok(node_to_json(&root))
}

/// Writes a JSON tree (as [`to_tree`] gives it) as JRXML, the way JasperReports 7 writes it.
pub fn from_tree(tree: &Value) -> Result<String> {
    let root = json_to_node(tree, 0)?;
    if root.name != "jasperReport" && root.name != "jasperTemplate" {
        return Err(Error::Xml(format!("the top of the tree is <{}>, not <jasperReport>", root.name)));
    }
    Ok(convert::write(&root))
}

fn node_to_json(n: &Node) -> Value {
    let attrs: Vec<Value> = n.attrs.iter().map(|(k, v)| json!([k, v])).collect();
    let mut out = json!({
        "tag": n.name,
        "attrs": attrs,
        "children": n.children.iter().map(node_to_json).collect::<Vec<_>>(),
    });
    // Text between child elements is only the indentation of the file.
    if n.children.is_empty() && !n.text.is_empty() {
        out["text"] = Value::String(n.text.clone());
    }
    out
}

fn is_name(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ':'))
}

fn json_to_node(v: &Value, depth: usize) -> Result<Node> {
    if depth > MAX_DEPTH {
        return Err(Error::Xml(format!("elements nested more than {MAX_DEPTH} levels deep")));
    }
    let obj = v.as_object().ok_or_else(|| Error::Xml("each node of the tree is an object".into()))?;
    let name = obj.get("tag").and_then(Value::as_str).unwrap_or_default();
    if !is_name(name) {
        return Err(Error::Xml(format!("`{name}` is not an element name")));
    }
    let mut node = Node { name: name.to_string(), ..Default::default() };
    if let Some(attrs) = obj.get("attrs") {
        let pairs: Vec<(&str, &Value)> = match attrs {
            Value::Object(m) => m.iter().map(|(k, v)| (k.as_str(), v)).collect(),
            Value::Array(list) => list
                .iter()
                .map(|p| match p.as_array().map(Vec::as_slice) {
                    Some([Value::String(k), v]) => Ok((k.as_str(), v)),
                    _ => Err(Error::Xml(format!("<{name}>: each attribute is a [name, value] pair"))),
                })
                .collect::<Result<_>>()?,
            _ => return Err(Error::Xml(format!("<{name}>: `attrs` is a list of [name, value] pairs"))),
        };
        for (k, v) in pairs {
            if !is_name(k) {
                return Err(Error::Xml(format!("<{name}>: `{k}` is not an attribute name")));
            }
            let value = match v {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                Value::Null => continue,
                _ => return Err(Error::Xml(format!("<{name}>: attribute `{k}` is not text"))),
            };
            node.attrs.retain(|(name, _)| name != k);
            node.attrs.push((k.to_string(), value));
        }
    }
    if let Some(kids) = obj.get("children") {
        let kids = kids.as_array().ok_or_else(|| Error::Xml(format!("<{name}>: `children` is a list")))?;
        for k in kids {
            node.children.push(json_to_node(k, depth + 1)?);
        }
    }
    match obj.get("text") {
        Some(Value::String(s)) => node.text = s.clone(),
        None | Some(Value::Null) => {}
        Some(_) => return Err(Error::Xml(format!("<{name}>: `text` is a string"))),
    }
    Ok(node)
}

#[cfg(test)]
mod tests {
    use super::*;

    const V6: &str = r#"<?xml version="1.0"?>
<jasperReport xmlns="http://jasperreports.sourceforge.net/jasperreports" name="r" pageWidth="595" pageHeight="842" columnWidth="555" leftMargin="20" rightMargin="20" topMargin="20" bottomMargin="20">
  <field name="a" class="java.lang.String"/>
  <detail><band height="20">
    <textField><reportElement x="0" y="0" width="100" height="20"/><textFieldExpression>$F{a} + "&lt;]]&gt;"</textFieldExpression></textField>
  </band></detail>
</jasperReport>"#;

    #[test]
    fn reads_jrxml_6_as_a_tree_of_jrxml_7() {
        let t = to_tree(V6, None).unwrap();
        assert_eq!(t["tag"], "jasperReport");
        assert_eq!(t["attrs"][0], json!(["name", "r"]));
        let detail = t["children"].as_array().unwrap().iter().find(|c| c["tag"] == "detail").unwrap();
        let el = &detail["children"][0]["children"][0];
        assert_eq!(el["tag"], "element");
        assert_eq!(el["attrs"][0], json!(["kind", "textField"]));
        assert_eq!(el["children"][0]["text"], r#"$F{a} + "<]]>""#);
        // Indentation is not text.
        assert!(detail.get("text").is_none());
    }

    #[test]
    fn writes_the_tree_back_without_losing_anything() {
        let v7 = convert::jrxml_to_v7(V6).unwrap();
        let t = to_tree(&v7, None).unwrap();
        assert_eq!(from_tree(&t).unwrap(), v7);
        // And it renders the same as the original.
        let a = crate::jrxml::parse_report(V6).unwrap();
        let b = crate::jrxml::parse_report(&from_tree(&t).unwrap()).unwrap();
        assert_eq!(format!("{:?}", a.detail), format!("{:?}", b.detail));
    }

    #[test]
    fn reads_ferro_with_imports() {
        let src = "report r\n  import \"parts.ferro\"\n  field a String\n  detail 20\n    use money 0,0 100x20 value=$F{a}\n";
        let parts = b"styles\n  def money value\n    textField pattern=#,##0.00 : {{value}}\n".to_vec();
        assert!(to_tree(src, None).is_err());
        let load = |name: &str| (name == "parts.ferro").then(|| parts.clone());
        let t = to_tree(src, Some(&load)).unwrap();
        let jrxml = from_tree(&t).unwrap();
        assert!(jrxml.contains(r##"pattern="#,##0.00""##), "{jrxml}");
        assert!(crate::template::from_jrxml(&jrxml).unwrap().starts_with("report r"));
    }

    #[test]
    fn edits_made_to_the_tree_are_written() {
        let mut t = to_tree(V6, None).unwrap();
        t["attrs"] = json!([["name", "edited"], ["pageWidth", 842], ["unused", null], ["floatColumnFooter", true]]);
        let out = from_tree(&t).unwrap();
        assert!(out.contains(r#"<jasperReport name="edited" pageWidth="842" floatColumnFooter="true">"#), "{out}");
        assert!(!out.contains("unused"));
        // Attributes as an object work too.
        t["attrs"] = json!({"name": "o"});
        assert!(from_tree(&t).unwrap().contains(r#"<jasperReport name="o">"#));
    }

    #[test]
    fn rejects_trees_that_are_not_templates() {
        assert!(from_tree(&json!({"tag": "html", "attrs": {}, "children": []})).is_err());
        assert!(from_tree(&json!({"tag": "jasperReport", "attrs": {"a b": "1"}})).is_err());
        assert!(from_tree(&json!({"tag": "jasperReport", "children": [{"tag": "x><y"}]})).is_err());
        assert!(from_tree(&json!({"tag": "jasperReport", "attrs": [["name", {"x": 1}]]})).is_err());
        let mut deep = json!({"tag": "band"});
        for _ in 0..200 {
            deep = json!({"tag": "frame", "children": [deep]});
        }
        assert!(from_tree(&json!({"tag": "jasperReport", "children": [deep]})).is_err());
        // Values are escaped, not pasted.
        let out = from_tree(&json!({"tag": "jasperReport", "attrs": [["name", "\"/><x a=\""]]})).unwrap();
        assert!(out.contains("&quot;/&gt;&lt;x a=&quot;"), "{out}");
    }
}
