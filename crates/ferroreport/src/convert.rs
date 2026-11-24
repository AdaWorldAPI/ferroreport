//! Rewrites templates between the formats ferroreport reads: JRXML as JasperReports 6 writes it
//! (`<staticText><reportElement .../>`), JRXML as JasperReports 7 writes it
//! (`<element kind="staticText" .../>`), and ferroreport's own template format (see
//! [`crate::template`]).
//!
//! The JRXML 6 to 7 rewrite follows what JasperReports 7 itself writes for the samples it ships:
//! the layout of an element moves from `<reportElement>`, `<textElement>`, `<font>` and
//! `<graphicElement>` onto the element, `isXxx` attributes lose their prefix, expressions are
//! called `<expression>`, charts become `<element kind="chart" chartType="...">` with one
//! `<dataset>` and one `<plot>`, components become `<element kind="component">` around a
//! `<component kind="...">`.

use crate::error::{Error, Result};
use crate::xml::{self, Node};

/// Which JRXML a template is written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JrxmlVersion {
    /// JasperReports 6 and earlier.
    V6,
    /// JasperReports 7.
    V7,
}

/// The JRXML version of a template.
pub fn jrxml_version(source: &str) -> Result<JrxmlVersion> {
    Ok(version_of(&xml::parse_raw(source)?))
}

fn version_of(root: &Node) -> JrxmlVersion {
    fn has_v6(n: &Node) -> bool {
        // JRXML 7 has no isXxx attributes and none of these elements.
        let old_attr =
            |c: &Node| c.attrs.iter().any(|(k, _)| k.len() > 2 && k.starts_with("is") && k.as_bytes()[2].is_ascii_uppercase());
        n.children.iter().any(|c| {
            matches!(local(&c.name), "reportElement" | "queryString" | "groupExpression" | "variableExpression")
                || old_attr(c)
                || has_v6(c)
        })
    }
    fn has_v7(n: &Node) -> bool {
        n.children.iter().any(|c| (c.name == "element" && c.attr("kind").is_some()) || c.name == "query" || has_v7(c))
    }
    if root.attr("xmlns").is_some() || (has_v6(root) && !has_v7(root)) {
        JrxmlVersion::V6
    } else {
        JrxmlVersion::V7
    }
}

/// Rewrites a JRXML template as JasperReports 7 writes it. A template that already is JRXML 7
/// comes back unchanged.
pub fn jrxml_to_v7(source: &str) -> Result<String> {
    let root = xml::parse_raw(source)?;
    match local(&root.name) {
        _ if version_of(&root) == JrxmlVersion::V7 => Ok(source.to_string()),
        "jasperReport" => Ok(write(&report(&root))),
        "jasperTemplate" => Ok(write(&style_template(&root))),
        other => Err(Error::Xml(format!("<{other}> is not a JRXML template (expected <jasperReport>) or style template"))),
    }
}

// ---------------------------------------------------------------------------------------------
// Small tree helpers

fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

fn node(name: &str) -> Node {
    Node { name: name.to_string(), ..Default::default() }
}

trait Build {
    fn set(&mut self, k: &str, v: &str);
    fn with(self, k: &str, v: &str) -> Self;
    fn push(&mut self, c: Node);
}

impl Build for Node {
    fn set(&mut self, k: &str, v: &str) {
        match self.attrs.iter_mut().find(|(a, _)| a == k) {
            Some((_, old)) => *old = v.to_string(),
            None => self.attrs.push((k.to_string(), v.to_string())),
        }
    }
    fn with(mut self, k: &str, v: &str) -> Self {
        self.set(k, v);
        self
    }
    fn push(&mut self, c: Node) {
        self.children.push(c);
    }
}

fn kid<'a>(n: &'a Node, name: &str) -> Option<&'a Node> {
    n.children.iter().find(|c| local(&c.name) == name)
}

/// `isBold` -> `bold`; namespace declarations are dropped (JRXML 7 has none).
fn plain_attr(k: &str) -> Option<String> {
    if k == "xmlns" || k.starts_with("xmlns:") || k.starts_with("xsi:") {
        return None;
    }
    let k = local(k);
    let b = k.as_bytes();
    if b.len() > 2 && k.starts_with("is") && b[2].is_ascii_uppercase() {
        let mut s = k[2..].to_string();
        s[..1].make_ascii_lowercase();
        return Some(s);
    }
    Some(k.to_string())
}

/// Copies `from`'s attributes onto `to`, renamed by `rename` (`None` keeps the name, an empty
/// name drops the attribute).
fn copy_attrs(from: &Node, to: &mut Node, rename: impl Fn(&str) -> Option<&'static str>) {
    for (k, v) in &from.attrs {
        let Some(plain) = plain_attr(k) else { continue };
        let name = rename(&plain).map(str::to_string).unwrap_or(plain);
        if !name.is_empty() {
            to.set(&name, &plain_value(&name, v));
        }
    }
}

/// Values JRXML 7 spells differently: the stretch types renamed in JasperReports 6.10 and
/// colors given by name.
fn plain_value(name: &str, v: &str) -> String {
    match (name, v) {
        ("stretchType", "RelativeToBandHeight") => "ContainerHeight".into(),
        ("stretchType", "RelativeToTallestObject") => "ElementGroupHeight".into(),
        _ if name.ends_with("olor") || name == "backcolor" || name == "forecolor" => named_color(v).unwrap_or(v).into(),
        _ => v.into(),
    }
}

/// The colors JRXML may name (those of `java.awt.Color`).
pub(crate) fn named_color(v: &str) -> Option<&'static str> {
    Some(match v.to_ascii_lowercase().as_str() {
        "black" => "#000000",
        "blue" => "#0000FF",
        "cyan" => "#00FFFF",
        "darkgray" | "dark_gray" => "#404040",
        "gray" => "#808080",
        "green" => "#00FF00",
        "lightgray" | "light_gray" => "#C0C0C0",
        "magenta" => "#FF00FF",
        "orange" => "#FFC800",
        "pink" => "#FFAFAF",
        "red" => "#FF0000",
        "white" => "#FFFFFF",
        "yellow" => "#FFFF00",
        _ => return None,
    })
}

/// A leaf whose text is an expression (or other text), renamed.
fn text_node(n: &Node, name: &str) -> Node {
    let mut out = node(name);
    // JRXML 7 expressions have no class: the value's class comes from what holds them.
    copy_attrs(n, &mut out, |k| if k == "class" && name.ends_with("xpression") { Some("") } else { None });
    out.text = n.text.clone();
    out
}

/// A node copied as is, with plain attribute names and every child copied the same way.
fn plain(n: &Node, name: &str) -> Node {
    let mut out = node(name);
    copy_attrs(n, &mut out, |_| None);
    if n.children.is_empty() {
        out.text = n.text.clone();
    }
    for c in &n.children {
        out.push(plain(c, local(&c.name)));
    }
    out
}

// ---------------------------------------------------------------------------------------------
// The report and its datasets

fn report(n: &Node) -> Node {
    let mut out = node("jasperReport");
    copy_attrs(n, &mut out, |k| match k {
        "schemaLocation" => Some(""),
        _ => None,
    });
    for c in &n.children {
        match local(&c.name) {
            "property" | "propertyExpression" | "template" | "reportFont" => out.push(plain(c, local(&c.name))),
            "import" => {
                let mut i = node("import");
                i.text = c.attr("value").unwrap_or_default().to_string();
                out.push(i);
            }
            "style" => out.push(style(c)),
            "subDataset" => out.push(dataset(c, "dataset")),
            "scriptlet" => out.push(scriptlet(c)),
            "parameter" | "field" | "variable" | "sortField" | "queryString" | "filterExpression" => {
                out.children.extend(dataset_part(c))
            }
            "group" => out.push(group(c)),
            "background" | "title" | "pageHeader" | "columnHeader" | "columnFooter" | "pageFooter" | "lastPageFooter"
            | "summary" | "noData" => {
                if let Some(b) = kid(c, "band") {
                    out.push(band(b, local(&c.name)));
                }
            }
            "detail" => {
                let mut d = node("detail");
                for b in &c.children {
                    match local(&b.name) {
                        "band" => d.push(band(b, "band")),
                        "part" => d.push(part(b)),
                        _ => {}
                    }
                }
                out.push(d);
            }
            other => out.push(plain(c, other)),
        }
    }
    out
}

fn scriptlet(n: &Node) -> Node {
    let mut out = node("scriptlet");
    copy_attrs(n, &mut out, |_| None);
    for c in &n.children {
        match local(&c.name) {
            "scriptletDescription" => out.push(text_node(c, "description")),
            other => out.push(plain(c, other)),
        }
    }
    out
}

/// What a report and a sub-dataset share: parameters, the query, fields, sort fields,
/// variables, the filter, groups.
fn dataset_part(c: &Node) -> Vec<Node> {
    match local(&c.name) {
        "parameter" => {
            let mut p = node("parameter");
            copy_attrs(c, &mut p, |_| None);
            for k in &c.children {
                match local(&k.name) {
                    "parameterDescription" => p.push(text_node(k, "description")),
                    other => p.push(plain(k, other)),
                }
            }
            vec![p]
        }
        "field" => {
            let mut f = node("field");
            copy_attrs(c, &mut f, |_| None);
            for k in &c.children {
                match local(&k.name) {
                    "fieldDescription" => f.push(text_node(k, "description")),
                    other => f.push(plain(k, other)),
                }
            }
            vec![f]
        }
        "variable" => {
            let mut v = node("variable");
            copy_attrs(c, &mut v, |_| None);
            for k in &c.children {
                match local(&k.name) {
                    "variableExpression" => v.push(text_node(k, "expression")),
                    other => v.push(plain(k, other)),
                }
            }
            vec![v]
        }
        "queryString" => {
            let mut q = node("query");
            q.set("language", c.attr("language").unwrap_or("sql"));
            q.text = c.text.clone();
            vec![q]
        }
        "sortField" | "filterExpression" => vec![plain(c, local(&c.name))],
        "group" => vec![group(c)],
        _ => Vec::new(),
    }
}

fn dataset(n: &Node, name: &str) -> Node {
    let mut out = node(name);
    copy_attrs(n, &mut out, |_| None);
    for c in &n.children {
        match local(&c.name) {
            "property" | "propertyExpression" => out.push(plain(c, local(&c.name))),
            "scriptlet" => out.push(scriptlet(c)),
            _ => out.children.extend(dataset_part(c)),
        }
    }
    out
}

fn group(n: &Node) -> Node {
    let mut out = node("group");
    copy_attrs(n, &mut out, |_| None);
    for c in &n.children {
        match local(&c.name) {
            "groupExpression" => out.push(text_node(c, "expression")),
            "groupHeader" | "groupFooter" => {
                let mut s = node(local(&c.name));
                for b in &c.children {
                    match local(&b.name) {
                        "band" => s.push(band(b, "band")),
                        "part" => s.push(part(b)),
                        _ => {}
                    }
                }
                out.push(s);
            }
            other => out.push(plain(c, other)),
        }
    }
    out
}

fn part(n: &Node) -> Node {
    let mut out = node("part");
    copy_attrs(n, &mut out, |_| None);
    for c in &n.children {
        match local(&c.name) {
            "reportElement" => {
                copy_attrs(c, &mut out, |_| None);
                for k in &c.children {
                    out.push(plain(k, local(&k.name)));
                }
            }
            "subreportPart" | "component" => out.push(component(c)),
            other => out.push(plain(c, other)),
        }
    }
    out
}

fn band(n: &Node, name: &str) -> Node {
    let mut out = node(name);
    copy_attrs(n, &mut out, |_| None);
    let mut elements = Vec::new();
    for c in &n.children {
        match local(&c.name) {
            "printWhenExpression" | "property" | "returnValue" => out.push(plain(c, local(&c.name))),
            _ => elements.extend(element(c)),
        }
    }
    out.children.extend(elements);
    out
}

// ---------------------------------------------------------------------------------------------
// Styles

fn style(n: &Node) -> Node {
    let mut out = node("style");
    copy_attrs(n, &mut out, style_attr);
    // The old hAlign and vAlign aligned both texts and images.
    for (old, image) in [("hAlign", "hImageAlign"), ("vAlign", "vImageAlign")] {
        if let (Some(v), None) = (n.attr(old), n.attr(image)) {
            out.set(image, v);
        }
    }
    for c in &n.children {
        match local(&c.name) {
            "conditionalStyle" => {
                let mut cs = node("conditionalStyle");
                if let Some(e) = kid(c, "conditionExpression") {
                    cs.push(text_node(e, "conditionExpression"));
                }
                if let Some(s) = kid(c, "style") {
                    let inner = style(s);
                    for (k, v) in inner.attrs {
                        cs.set(&k, &v);
                    }
                    cs.children.extend(inner.children);
                }
                out.push(cs);
            }
            "pen" => out.push(plain(c, "pen")),
            "box" => out.push(plain(c, "box")),
            "paragraph" => out.push(plain(c, "paragraph")),
            other => out.push(plain(c, other)),
        }
    }
    out
}

/// A style template (`.jrtx`): its styles and the templates it includes.
fn style_template(n: &Node) -> Node {
    let mut out = node("jasperTemplate");
    copy_attrs(n, &mut out, |_| None);
    for c in &n.children {
        match local(&c.name) {
            "style" => out.push(style(c)),
            other => out.push(plain(c, other)),
        }
    }
    out
}

fn style_attr(k: &str) -> Option<&'static str> {
    match k {
        "hAlign" => Some("hTextAlign"),
        "vAlign" => Some("vTextAlign"),
        "size" => Some("fontSize"),
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------
// Elements

const CHARTS: &[&str] = &[
    "areaChart",
    "bar3DChart",
    "barChart",
    "bubbleChart",
    "candlestickChart",
    "ganttChart",
    "highLowChart",
    "lineChart",
    "meterChart",
    "multiAxisChart",
    "pie3DChart",
    "pieChart",
    "scatterChart",
    "stackedAreaChart",
    "stackedBar3DChart",
    "stackedBarChart",
    "thermometerChart",
    "timeSeriesChart",
    "xyAreaChart",
    "xyBarChart",
    "xyLineChart",
];

/// One child of a band, frame or cell, as JRXML 7 elements (an element group's elements
/// stay in a group).
fn element(n: &Node) -> Vec<Node> {
    let tag = local(&n.name);
    let out = match tag {
        "staticText" | "textField" | "image" | "line" | "rectangle" | "ellipse" | "frame" | "subreport" | "break" => {
            simple_element(n, tag)
        }
        "elementGroup" => {
            let mut g = node("element").with("kind", "elementGroup");
            for c in &n.children {
                g.children.extend(element(c));
            }
            g
        }
        "componentElement" => component_element(n),
        "genericElement" => generic_element(n),
        "crosstab" => crosstab(n),
        t if CHARTS.contains(&t) => chart(n),
        _ => return Vec::new(),
    };
    vec![out]
}

/// `<element kind="...">` with what `<reportElement>` says.
fn element_head(n: &Node, kind: &str) -> (Node, Vec<Node>) {
    let mut out = node("element").with("kind", kind);
    let mut tail = Vec::new();
    if let Some(re) = kid(n, "reportElement") {
        copy_attrs(re, &mut out, |_| None);
        for c in &re.children {
            match local(&c.name) {
                "printWhenExpression" => out.push(plain(c, "printWhenExpression")),
                other => tail.push(plain(c, other)),
            }
        }
    }
    (out, tail)
}

/// Text layout: `<textElement>` and its `<font>` and `<paragraph>`.
fn text_element(te: &Node, out: &mut Node, after: &mut Vec<Node>) {
    copy_attrs(te, out, |k| match k {
        "textAlignment" => Some("hTextAlign"),
        "verticalAlignment" => Some("vTextAlign"),
        _ => None,
    });
    for c in &te.children {
        match local(&c.name) {
            "font" => copy_attrs(c, out, font_attr),
            "paragraph" => after.push(plain(c, "paragraph")),
            _ => {}
        }
    }
}

fn font_attr(k: &str) -> Option<&'static str> {
    match k {
        "size" => Some("fontSize"),
        "reportFont" => Some(""),
        _ => None,
    }
}

fn link_attr(k: &str) -> Option<&'static str> {
    match k {
        "hyperlinkType" => Some("linkType"),
        "hyperlinkTarget" => Some("linkTarget"),
        "hAlign" => Some("hImageAlign"),
        "vAlign" => Some("vImageAlign"),
        _ => None,
    }
}

fn simple_element(n: &Node, kind: &str) -> Node {
    let (mut out, props) = element_head(n, kind);
    let mut body = Vec::new();
    let mut after: Vec<Node> = Vec::new();
    // The element's own attributes.
    for (k, v) in &n.attrs {
        let Some(plain_name) = plain_attr(k) else { continue };
        match plain_name.as_str() {
            "stretchWithOverflow" => {
                if v == "true" && n.attr("textAdjust").is_none() {
                    out.set("textAdjust", "StretchHeight");
                }
            }
            "borderSplitType" => {
                let mut b = node("borderSplitType");
                b.text = v.clone();
                after.insert(0, b);
            }
            other => {
                let name = link_attr(other).unwrap_or(other);
                out.set(name, &plain_value(name, v));
            }
        }
    }
    let mut box_node = None;
    for c in &n.children {
        match local(&c.name) {
            "reportElement" => {}
            "box" => box_node = Some(plain(c, "box")),
            "textElement" => text_element(c, &mut out, &mut after),
            "graphicElement" => {
                copy_attrs(c, &mut out, |_| None);
                for p in &c.children {
                    after.push(plain(p, local(&p.name)));
                }
            }
            "text" => body.push(text_node(c, "text")),
            "textFieldExpression" | "imageExpression" | "subreportExpression" => body.push(text_node(c, "expression")),
            "subreportParameter" => body.push(parameter(c)),
            "frame" | "staticText" | "textField" | "image" | "line" | "rectangle" | "ellipse" | "subreport" | "break"
            | "elementGroup" | "componentElement" | "genericElement" | "crosstab" => body.extend(element(c)),
            t if CHARTS.contains(&t) => body.extend(element(c)),
            "hyperlinkParameter" => body.push(hyperlink_parameter(c)),
            other => body.push(plain(c, other)),
        }
    }
    out.children.extend(body);
    out.children.extend(props);
    out.children.extend(after);
    out.children.extend(box_node);
    out
}

/// `<subreportParameter>`, `<datasetParameter>` or `<crosstabParameter>`: a name and an
/// expression.
fn parameter(n: &Node) -> Node {
    let mut p = node("parameter");
    copy_attrs(n, &mut p, |_| None);
    for c in &n.children {
        match local(&c.name) {
            "subreportParameterExpression" | "datasetParameterExpression" | "parameterValueExpression" => {
                p.push(text_node(c, "expression"))
            }
            other => p.push(plain(c, other)),
        }
    }
    p
}

fn hyperlink_parameter(n: &Node) -> Node {
    let mut p = node("hyperlinkParameter");
    copy_attrs(n, &mut p, |_| None);
    for c in &n.children {
        match local(&c.name) {
            "hyperlinkParameterExpression" => p.push(text_node(c, "expression")),
            other => p.push(plain(c, other)),
        }
    }
    p
}

fn dataset_run(n: &Node) -> Node {
    let mut out = node("datasetRun");
    copy_attrs(n, &mut out, |_| None);
    for c in &n.children {
        match local(&c.name) {
            "datasetParameter" => out.push(parameter(c)),
            other => out.push(plain(c, other)),
        }
    }
    out
}

fn generic_element(n: &Node) -> Node {
    let (mut out, props) = element_head(n, "generic");
    copy_attrs(n, &mut out, |_| None);
    for c in &n.children {
        match local(&c.name) {
            "reportElement" => {}
            "genericElementType" => out.push(plain(c, "genericType")),
            "genericElementParameter" => {
                let mut p = node("parameter");
                copy_attrs(c, &mut p, |_| None);
                for k in &c.children {
                    match local(&k.name) {
                        "valueExpression" => p.push(text_node(k, "expression")),
                        other => p.push(plain(k, other)),
                    }
                }
                out.push(p);
            }
            other => out.push(plain(c, other)),
        }
    }
    out.children.extend(props);
    out
}

// ---------------------------------------------------------------------------------------------
// Components: tables, lists, barcodes and the rest

const BARCODE4J: &[&str] = &[
    "Codabar",
    "Code128",
    "Code39",
    "DataMatrix",
    "EAN128",
    "EAN13",
    "EAN8",
    "Interleaved2Of5",
    "PDF417",
    "POSTNET",
    "QRCode",
    "RoyalMailCustomer",
    "UPCA",
    "UPCE",
    "USPSIntelligentMail",
];

fn component_element(n: &Node) -> Node {
    let (mut out, props) = element_head(n, "component");
    for c in &n.children {
        if local(&c.name) != "reportElement" {
            let mut comp = component(c);
            // An icon label takes the style of its element.
            if let (Some(style), "iconLabel", None) = (out.attr("style"), local(&c.name), comp.attr("style")) {
                comp.set("style", style);
            }
            out.push(comp);
        }
    }
    out.children.extend(props);
    out
}

fn component(n: &Node) -> Node {
    let name = local(&n.name);
    let kind = if BARCODE4J.contains(&name) { format!("barcode4j:{name}") } else { name.to_string() };
    let mut out = node("component").with("kind", &kind);
    copy_attrs(n, &mut out, |_| None);
    match name {
        "table" => {
            for c in &n.children {
                match local(&c.name) {
                    "datasetRun" => out.push(dataset_run(c)),
                    "column" | "columnGroup" => out.push(table_column(c)),
                    "noData" => out.push(table_cell(c, "noData")),
                    other => out.push(plain(c, other)),
                }
            }
        }
        "list" => {
            for c in &n.children {
                match local(&c.name) {
                    "datasetRun" => out.push(dataset_run(c)),
                    "listContents" => {
                        let mut contents = node("contents");
                        copy_attrs(c, &mut contents, |_| None);
                        for e in &c.children {
                            contents.children.extend(element(e));
                        }
                        out.push(contents);
                    }
                    other => out.push(plain(c, other)),
                }
            }
        }
        "iconLabel" => {
            out.attrs.clear();
            out.set("kind", "iconLabel");
            copy_attrs(n, &mut out, |k| match k {
                "horizontalAlignment" => Some("hAlign"),
                "verticalAlignment" => Some("vAlign"),
                _ => None,
            });
            for c in &n.children {
                match local(&c.name) {
                    // <c:label><textField> -> <labelTextField>, <c:icon><textField> -> <iconTextField>
                    which @ ("label" | "icon") => {
                        if let Some(t) = kid(c, "textField") {
                            let mut f = simple_element(t, "textField");
                            f.name = format!("{which}TextField");
                            f.attrs.retain(|(k, _)| k != "kind");
                            out.push(f);
                        }
                    }
                    other => out.push(plain(c, other)),
                }
            }
        }
        _ => {
            for c in &n.children {
                out.push(component_child(c));
            }
        }
    }
    out
}

/// A child of some other component (a barcode's code, a spider chart's settings...): copied
/// with JRXML 7 names, its datasets and fonts rewritten.
fn component_child(c: &Node) -> Node {
    match local(&c.name) {
        "datasetRun" => dataset_run(c),
        "subreportExpression" => text_node(c, "expression"),
        "subreportParameter" => parameter(c),
        "chartSettings" => {
            let mut out = node("chartSettings");
            copy_attrs(c, &mut out, |_| None);
            chart_settings(c, &mut out);
            out
        }
        "spiderDataset" => chart_dataset(c, ""),
        "spiderPlot" => chart_plot(c),
        "font" => {
            let mut f = node("font");
            copy_attrs(c, &mut f, font_attr);
            f
        }
        other => {
            let mut out = node(other);
            copy_attrs(c, &mut out, |_| None);
            if c.children.is_empty() {
                out.text = c.text.clone();
            }
            for k in &c.children {
                out.push(component_child(k));
            }
            out
        }
    }
}

fn table_column(n: &Node) -> Node {
    let group = local(&n.name) == "columnGroup";
    let mut out = node("column").with("kind", if group { "group" } else { "single" });
    copy_attrs(n, &mut out, |_| None);
    for c in &n.children {
        match local(&c.name) {
            "column" | "columnGroup" => out.push(table_column(c)),
            "tableHeader" | "tableFooter" | "columnHeader" | "columnFooter" | "detailCell" => {
                out.push(table_cell(c, local(&c.name)))
            }
            "groupHeader" | "groupFooter" => {
                let mut g = node(local(&c.name));
                copy_attrs(c, &mut g, |_| None);
                for k in &c.children {
                    g.push(table_cell(k, "cell"));
                }
                out.push(g);
            }
            other => out.push(plain(c, other)),
        }
    }
    out
}

fn table_cell(n: &Node, name: &str) -> Node {
    let mut out = node(name);
    copy_attrs(n, &mut out, |_| None);
    let mut boxed = None;
    for c in &n.children {
        match local(&c.name) {
            "box" => boxed = Some(plain(c, "box")),
            "property" | "propertyExpression" | "printWhenExpression" => out.push(plain(c, local(&c.name))),
            _ => out.children.extend(element(c)),
        }
    }
    out.children.extend(boxed);
    out
}

// ---------------------------------------------------------------------------------------------
// Crosstabs

fn crosstab(n: &Node) -> Node {
    let (mut out, props) = element_head(n, "crosstab");
    copy_attrs(n, &mut out, |_| None);
    for c in &n.children {
        match local(&c.name) {
            "reportElement" => {}
            "crosstabParameter" => {
                // Crosstab parameters were strings unless they said otherwise; in JRXML 7 they
                // are objects.
                let mut p = parameter(c);
                if p.attr("class").is_none() {
                    p.set("class", "java.lang.String");
                }
                out.push(p);
            }
            "parametersMapExpression" => out.push(plain(c, "parametersMapExpression")),
            "crosstabDataset" => {
                let mut d = node("dataset");
                copy_attrs(c, &mut d, |_| None);
                for k in &c.children {
                    match local(&k.name) {
                        "dataset" => {
                            copy_attrs(k, &mut d, |_| None);
                            for r in &k.children {
                                match local(&r.name) {
                                    "datasetRun" => d.push(dataset_run(r)),
                                    other => d.push(plain(r, other)),
                                }
                            }
                        }
                        other => d.push(plain(k, other)),
                    }
                }
                out.push(d);
            }
            "titleCell" => {
                let mut t = node("titleCell");
                copy_attrs(c, &mut t, |_| None);
                if let Some(cc) = kid(c, "cellContents") {
                    t.push(cell_contents(cc, "cellContents"));
                }
                out.push(t);
            }
            "crosstabHeaderCell" => {
                if let Some(cc) = kid(c, "cellContents") {
                    out.push(cell_contents(cc, "headerCell"));
                }
            }
            "whenNoDataCell" => {
                if let Some(cc) = kid(c, "cellContents") {
                    out.push(cell_contents(cc, "whenNoDataCell"));
                }
            }
            "rowGroup" | "columnGroup" => {
                let mut g = node(local(&c.name));
                copy_attrs(c, &mut g, |k| if k == "headerPosition" { Some("position") } else { None });
                for k in &c.children {
                    match local(&k.name) {
                        "bucket" => {
                            let mut b = node("bucket");
                            copy_attrs(k, &mut b, |_| None);
                            for e in &k.children {
                                match local(&e.name) {
                                    "bucketExpression" => b.push(text_node(e, "expression")),
                                    other => b.push(plain(e, other)),
                                }
                            }
                            g.push(b);
                        }
                        "crosstabRowHeader" | "crosstabColumnHeader" => {
                            if let Some(cc) = kid(k, "cellContents") {
                                g.push(cell_contents(cc, "header"));
                            }
                        }
                        "crosstabTotalRowHeader" | "crosstabTotalColumnHeader" => {
                            if let Some(cc) = kid(k, "cellContents") {
                                g.push(cell_contents(cc, "totalHeader"));
                            }
                        }
                        "crosstabHeader" => {
                            if let Some(cc) = kid(k, "cellContents") {
                                g.push(cell_contents(cc, "crosstabHeader"));
                            }
                        }
                        other => g.push(plain(k, other)),
                    }
                }
                out.push(g);
            }
            "measure" => {
                let mut m = node("measure");
                copy_attrs(c, &mut m, |k| if k == "percentageOf" { Some("percentageType") } else { None });
                for k in &c.children {
                    match local(&k.name) {
                        "measureExpression" => m.push(text_node(k, "expression")),
                        other => m.push(plain(k, other)),
                    }
                }
                out.push(m);
            }
            "crosstabCell" => {
                let mut cell = node("cell");
                copy_attrs(c, &mut cell, |_| None);
                if let Some(cc) = kid(c, "cellContents") {
                    cell.push(cell_contents(cc, "contents"));
                }
                out.push(cell);
            }
            other => out.push(plain(c, other)),
        }
    }
    out.children.extend(props);
    out
}

fn cell_contents(n: &Node, name: &str) -> Node {
    let mut out = node(name);
    copy_attrs(n, &mut out, |_| None);
    let mut boxed = None;
    for c in &n.children {
        match local(&c.name) {
            "box" => boxed = Some(plain(c, "box")),
            "property" | "propertyExpression" => out.push(plain(c, local(&c.name))),
            _ => out.children.extend(element(c)),
        }
    }
    if let Some(b) = boxed {
        out.children.insert(0, b);
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Charts

fn chart(n: &Node) -> Node {
    let tag = local(&n.name);
    let chart_type = tag.strip_suffix("Chart").unwrap_or(tag);
    let mut out = node("element").with("kind", "chart").with("chartType", chart_type);
    let mut props = Vec::new();
    if let Some(ch) = kid(n, "chart") {
        let (head, tail) = element_head(ch, "chart");
        for (k, v) in head.attrs.iter().skip(1) {
            out.set(k, v);
        }
        out.children.extend(head.children);
        props = tail;
        copy_attrs(ch, &mut out, |_| None);
        props.extend(chart_settings(ch, &mut out));
    }
    for c in &n.children {
        let name = local(&c.name);
        if let Some(kind) = name.strip_suffix("Dataset") {
            out.push(chart_dataset(c, kind));
        } else if name.ends_with("Plot") {
            out.push(chart_plot(c));
        }
    }
    out.children.extend(props);
    out
}

/// A chart's title, subtitle and legend (`<chartTitle>`...) as attributes and children of
/// `out`; returns the chart's box, which goes last.
fn chart_settings(ch: &Node, out: &mut Node) -> Vec<Node> {
    let mut fonts = Vec::new();
    let mut exprs = Vec::new();
    let mut tail = Vec::new();
    for c in &ch.children {
        match local(&c.name) {
            "reportElement" => {}
            "chartTitle" => {
                copy_attrs(c, out, |k| match k {
                    "position" => Some("titlePosition"),
                    "color" => Some("titleColor"),
                    _ => None,
                });
                if let Some(f) = kid(c, "font") {
                    fonts.push(font(f, "titleFont"));
                }
                if let Some(e) = kid(c, "titleExpression") {
                    exprs.push(text_node(e, "titleExpression"));
                }
            }
            "chartSubtitle" => {
                copy_attrs(c, out, |k| match k {
                    "color" => Some("subtitleColor"),
                    _ => None,
                });
                if let Some(f) = kid(c, "font") {
                    fonts.push(font(f, "subtitleFont"));
                }
                if let Some(e) = kid(c, "subtitleExpression") {
                    exprs.push(text_node(e, "subtitleExpression"));
                }
            }
            "chartLegend" => {
                copy_attrs(c, out, |k| match k {
                    "textColor" => Some("legendColor"),
                    "backgroundColor" => Some("legendBackgroundColor"),
                    "position" => Some("legendPosition"),
                    _ => None,
                });
                if let Some(f) = kid(c, "font") {
                    fonts.push(font(f, "legendFont"));
                }
            }
            "box" => tail.push(plain(c, "box")),
            other => exprs.push(plain(c, other)),
        }
    }
    out.children.extend(fonts);
    out.children.extend(exprs);
    tail
}

fn font(n: &Node, name: &str) -> Node {
    let mut f = node(name);
    copy_attrs(n, &mut f, font_attr);
    f
}

fn chart_dataset(n: &Node, kind: &str) -> Node {
    let mut loose = Vec::new();
    let mut out = node("dataset");
    if !kind.is_empty() {
        out.set("kind", kind);
    }
    // A value dataset (meters, thermometers) was never reset unless it said so.
    if kind == "value" && kid(n, "dataset").and_then(|d| d.attr("resetType")).is_none() {
        out.set("resetType", "None");
    }
    copy_attrs(n, &mut out, |_| None);
    for c in &n.children {
        match local(&c.name) {
            "dataset" => {
                copy_attrs(c, &mut out, |_| None);
                for k in &c.children {
                    match local(&k.name) {
                        "datasetRun" => out.push(dataset_run(k)),
                        other => out.push(plain(k, other)),
                    }
                }
            }
            "categorySeries" | "pieSeries" | "xySeries" | "xyzSeries" | "timeSeries" | "timePeriodSeries" | "ganttSeries" => {
                out.push(series(c))
            }
            other => loose.push(series_child(c, &jackson_name(other))),
        }
    }
    // A pie dataset may give its key and value without a <pieSeries>: they make one series.
    if !loose.is_empty() {
        if kind == "pie" {
            let mut one = node("series");
            one.children = loose;
            out.push(one);
        } else {
            out.children.extend(loose);
        }
    }
    out
}

/// How JRXML 7 spells a name that starts with one lower-case letter and then a capital
/// (`xValueExpression` -> `xvalueExpression`).
fn jackson_name(name: &str) -> String {
    let b = name.as_bytes();
    if b.len() > 2 && b[0].is_ascii_lowercase() && b[1].is_ascii_uppercase() {
        let mut s = name.to_string();
        s[1..2].make_ascii_lowercase();
        return s;
    }
    name.to_string()
}

fn series(n: &Node) -> Node {
    let mut out = node("series");
    copy_attrs(n, &mut out, |_| None);
    for c in &n.children {
        out.push(series_child(c, &jackson_name(local(&c.name))));
    }
    out
}

fn series_child(c: &Node, name: &str) -> Node {
    match name {
        "itemHyperlink" | "sectionHyperlink" => {
            let mut h = node(name);
            copy_attrs(c, &mut h, link_attr);
            for k in &c.children {
                match local(&k.name) {
                    "hyperlinkParameter" => h.push(hyperlink_parameter(k)),
                    other => h.push(plain(k, other)),
                }
            }
            h
        }
        _ => plain(c, name),
    }
}

/// `xAxisLabelExpression` -> `xaxisLabelExpression`, as JRXML 7 spells the x/y axes.
fn axis_name(name: &str) -> String {
    jackson_name(name)
}

fn chart_plot(n: &Node) -> Node {
    let mut out = node("plot");
    let mut kids_out = Vec::new();
    let mut colors = Vec::new();
    for c in &n.children {
        let name = local(&c.name);
        match name {
            "plot" => {
                copy_attrs(c, &mut out, |_| None);
                for s in &c.children {
                    let mut sc = node(local(&s.name));
                    copy_attrs(s, &mut sc, |k| if k == "seriesOrder" { Some("order") } else { None });
                    colors.push(sc);
                }
            }
            n if n.ends_with("AxisFormat") => {
                let axis = axis_name(&n[..n.len() - "Format".len()]);
                if let Some(f) = kid(c, "axisFormat") {
                    for (k, v) in &f.attrs {
                        let Some(k) = plain_attr(k) else { continue };
                        let attr = match k.as_str() {
                            "axisLineColor" => "LineColor".to_string(),
                            other => {
                                let mut s = other.to_string();
                                s[..1].make_ascii_uppercase();
                                s
                            }
                        };
                        out.set(&format!("{axis}{attr}"), v);
                    }
                    for k in &f.children {
                        let kname = local(&k.name);
                        let mut s = kname.to_string();
                        s[..1].make_ascii_uppercase();
                        let fnode = kid(k, "font").unwrap_or(k);
                        kids_out.push(font(fnode, &format!("{axis}{s}")));
                    }
                }
            }
            "axis" => {
                // <axis position="..."><xxxChart> -> <axis><position>...</position><chart chartType="xxx">
                let mut a = node("axis");
                if let Some(p) = c.attr("position").filter(|p| *p != "leftOrTop") {
                    let mut pos = node("position");
                    pos.text = p.to_string();
                    a.push(pos);
                }
                for k in &c.children {
                    for mut e in element(k) {
                        e.name = "chart".into();
                        e.attrs.retain(|(k, _)| k != "kind");
                        a.push(e);
                    }
                }
                kids_out.push(a);
            }
            "itemLabel" | "valueDisplay" => {
                let mut i = node(name);
                copy_attrs(c, &mut i, |_| None);
                for k in &c.children {
                    i.push(component_child(k));
                }
                kids_out.push(i);
            }
            "lowRange" | "mediumRange" | "highRange" => {
                // <highRange><dataRange><lowExpression/>... -> <highRange><lowExpression/>...
                let mut r = node(name);
                for k in kid(c, "dataRange").map(|d| d.children.as_slice()).unwrap_or_default() {
                    r.push(plain(k, local(&k.name)));
                }
                kids_out.push(r);
            }
            "tickLabelFont" | "labelFont" => {
                let fnode = kid(c, "font").unwrap_or(c);
                kids_out.push(font(fnode, name));
            }
            "meterInterval" => {
                let mut i = node("interval");
                copy_attrs(c, &mut i, |k| if k == "color" { Some("backgroundColor") } else { None });
                for k in &c.children {
                    i.push(plain(k, local(&k.name)));
                }
                kids_out.push(i);
            }
            other => kids_out.push(component_child(&Node { name: axis_name(other), ..c.clone() })),
        }
    }
    copy_attrs(n, &mut out, |_| None);
    out.children.extend(colors);
    out.children.extend(kids_out);
    out
}

// ---------------------------------------------------------------------------------------------
// Writing

/// Writes a tree as JRXML: one element per line, tabs, expressions and texts in CDATA.
pub(crate) fn write(root: &Node) -> String {
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    write_node(root, 0, &mut out);
    out
}

fn write_node(n: &Node, depth: usize, out: &mut String) {
    for _ in 0..depth {
        out.push('\t');
    }
    out.push('<');
    out.push_str(&n.name);
    for (k, v) in &n.attrs {
        out.push(' ');
        out.push_str(k);
        out.push_str("=\"");
        escape_into(v, true, out);
        out.push('"');
    }
    let text = if n.children.is_empty() { n.text.as_str() } else { "" };
    if n.children.is_empty() && text.is_empty() {
        out.push_str("/>\n");
        return;
    }
    out.push('>');
    if !text.is_empty() {
        cdata_into(text, out);
    } else {
        out.push('\n');
        for c in &n.children {
            write_node(c, depth + 1, out);
        }
        for _ in 0..depth {
            out.push('\t');
        }
    }
    out.push_str("</");
    out.push_str(&n.name);
    out.push_str(">\n");
}

pub(crate) fn escape_into(s: &str, attr: bool, out: &mut String) {
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' if attr => out.push_str("&quot;"),
            '\n' if attr => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            '\t' if attr => out.push_str("&#9;"),
            c => out.push(c),
        }
    }
}

fn cdata_into(s: &str, out: &mut String) {
    // "]]>" cannot sit inside one CDATA section: split it across two.
    out.push_str("<![CDATA[");
    out.push_str(&s.replace("]]>", "]]]]><![CDATA[>"));
    out.push_str("]]>");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_v6_elements_as_v7() {
        let v6 = r#"<?xml version="1.0"?>
<jasperReport xmlns="http://jasperreports.sourceforge.net/jasperreports" name="r" pageWidth="595" pageHeight="842" columnWidth="555" leftMargin="20" rightMargin="20" topMargin="20" bottomMargin="20" isTitleNewPage="false">
  <queryString><![CDATA[select 1]]></queryString>
  <field name="a" class="java.lang.String"><fieldDescription><![CDATA[A]]></fieldDescription></field>
  <variable name="n" class="java.lang.Integer" calculation="Count"><variableExpression><![CDATA[$F{a}]]></variableExpression></variable>
  <detail><band height="20">
    <textField isStretchWithOverflow="true" isBlankWhenNull="true">
      <reportElement x="0" y="0" width="100" height="20" isRemoveLineWhenBlank="true"/>
      <textElement textAlignment="Right"><font size="12" isBold="true"/></textElement>
      <textFieldExpression>$F{a} + "]]&gt;"</textFieldExpression>
    </textField>
  </band></detail>
</jasperReport>"#;
        let v7 = jrxml_to_v7(v6).unwrap();
        assert!(v7.contains(r#"<jasperReport name="r""#), "{v7}");
        assert!(v7.contains(r#"titleNewPage="false""#));
        assert!(v7.contains(r#"<query language="sql"><![CDATA[select 1]]></query>"#));
        assert!(v7.contains(r#"<description><![CDATA[A]]></description>"#));
        assert!(v7.contains(
            r#"<element kind="textField" x="0" y="0" width="100" height="20" removeLineWhenBlank="true" textAdjust="StretchHeight" blankWhenNull="true" hTextAlign="Right" fontSize="12" bold="true">"#
        ), "{v7}");
        assert!(v7.contains(r#"<expression><![CDATA[$F{a} + "]]]]><![CDATA[>"]]></expression>"#), "{v7}");
        assert_eq!(jrxml_version(&v7).unwrap(), JrxmlVersion::V7);
        assert_eq!(jrxml_version(v6).unwrap(), JrxmlVersion::V6);
        // Both describe the same report.
        let a = crate::jrxml::parse_report(v6).unwrap();
        let b = crate::jrxml::parse_report(&v7).unwrap();
        assert_eq!(format!("{:?}", a.detail), format!("{:?}", b.detail));
        assert_eq!(jrxml_to_v7(&v7).unwrap(), v7);
    }
}
