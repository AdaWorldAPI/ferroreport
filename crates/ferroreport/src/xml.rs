//! A tiny owned DOM built on top of `quick-xml`.
//!
//! JRXML files are small (a few hundred KB at most), so materialising the tree keeps the
//! template parser simple and lets it look ahead freely. No DTDs, external entities or
//! processing instructions are ever resolved, which keeps hostile templates harmless.

use crate::error::{Error, Result};
use quick_xml::events::Event;
use quick_xml::Reader;

/// An XML element with its attributes, child elements and concatenated text content.
#[derive(Debug, Clone, Default)]
pub struct Node {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
    pub text: String,
}

impl Node {
    /// Value of attribute `name`, if present.
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    /// First child element called `name`.
    pub fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name == name)
    }

    /// All child elements called `name`.
    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }
}

/// Strips a namespace prefix (`jr:table` -> `table`).
fn local(name: &str) -> String {
    match name.rfind(':') {
        Some(i) => name[i + 1..].to_string(),
        None => name.to_string(),
    }
}

fn predefined_entity(name: &str) -> Option<&'static str> {
    Some(match name {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        _ => return None,
    })
}

/// Parses a whole document and returns its root element.
pub fn parse(source: &str) -> Result<Node> {
    let mut reader = Reader::from_str(source);
    reader.config_mut().trim_text(false);
    let mut stack: Vec<Node> = Vec::new();
    let mut root: Option<Node> = None;
    let err = |e: &dyn std::fmt::Display, pos: u64| Error::Xml(format!("{e} (byte {pos})"));

    loop {
        let pos = reader.buffer_position();
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                stack.push(open(&e, pos)?);
            }
            Ok(Event::Empty(e)) => {
                let node = open(&e, pos)?;
                attach(&mut stack, &mut root, node);
            }
            Ok(Event::End(_)) => {
                let node = stack.pop().ok_or_else(|| Error::Xml("unbalanced end tag".into()))?;
                attach(&mut stack, &mut root, node);
            }
            Ok(Event::Text(t)) => {
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&t.xml10_content());
                }
            }
            Ok(Event::CData(t)) => {
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&t.xml10_content());
                }
            }
            Ok(Event::GeneralRef(r)) => {
                let resolved = match r.resolve_char_ref().map_err(|e| err(&e, pos))? {
                    Some(c) => c.to_string(),
                    None => {
                        let name = r.xml10_content();
                        predefined_entity(&name).ok_or_else(|| Error::Xml(format!("unknown entity &{name};")))?.to_string()
                    }
                };
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&resolved);
                }
            }
            Ok(Event::DocType(_)) => {
                return Err(Error::Xml("DOCTYPE declarations are not allowed".into()));
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(e) => return Err(err(&e, pos)),
        }
    }
    if !stack.is_empty() {
        return Err(Error::Xml("unexpected end of document".into()));
    }
    root.ok_or_else(|| Error::Xml("empty document".into()))
}

fn open(e: &quick_xml::events::BytesStart<'_>, pos: u64) -> Result<Node> {
    let mut node = Node { name: local(e.name().as_ref()), ..Default::default() };
    for a in e.attributes() {
        let a = a.map_err(|x| Error::Xml(format!("{x} (byte {pos})")))?;
        let key = local(a.key.as_ref());
        #[allow(deprecated)]
        let value = a.unescape_value().map_err(|x| Error::Xml(format!("{x} (byte {pos})")))?;
        node.attrs.push((key, value.into_owned()));
    }
    Ok(node)
}

fn attach(stack: &mut [Node], root: &mut Option<Node>, node: Node) {
    match stack.last_mut() {
        Some(parent) => parent.children.push(node),
        None => *root = Some(node),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cdata_entities_and_namespaces() {
        let doc = r#"<?xml version="1.0"?><a x="1&amp;2"><jr:b>t&lt;1 <![CDATA[$F{x} > 2]]></jr:b><c/></a>"#;
        let root = parse(doc).unwrap();
        assert_eq!(root.attr("x"), Some("1&2"));
        assert_eq!(root.child("b").unwrap().text, "t<1 $F{x} > 2");
        assert!(root.child("c").is_some());
    }

    #[test]
    fn rejects_doctype() {
        assert!(parse("<!DOCTYPE a [<!ENTITY x SYSTEM \"file:///etc/passwd\">]><a>&x;</a>").is_err());
    }
}
