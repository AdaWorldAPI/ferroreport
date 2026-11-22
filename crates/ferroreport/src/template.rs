//! ferroreport's own template format (`.ferro`): the same reports as JRXML, written as an
//! indented outline that reads like a document.
//!
//! ```text
//! // A list of employees.
//! report "Employees" lang=en
//!   style Base default fontName="DejaVu Sans" fontSize=10
//!   field name String
//!   field salary Double
//!   variable total Double calculation=Sum : $F{salary}
//!
//!   title 40
//!     h1 0,0 555x30 fontSize=18 : Employees
//!   columnHeader 20
//!     staticText 0,0 300x20 bold : Name
//!     staticText 300,0 255x20 bold hTextAlign=Right : Salary
//!   detail 20
//!     textField 0,0 300x20 : $F{name}
//!     textField 300,0 255x20 pattern=#,##0.00 hTextAlign=Right : $F{salary}
//!   summary 20
//!     textField 300,0 255x20 bold hTextAlign=Right : $V{total}
//! ```
//!
//! Each line is one node of a JasperReports 7 template: its name (for an element, its kind),
//! then its attributes as `name=value` (quoted when the value has spaces; a bare `name` means
//! `name=true`), then after ` : ` its text or main expression. Lines indented under it are its
//! children. A text that spans lines follows a line that ends in ` :`, indented one level
//! more; `:= "..."` gives one as a JSON string. Lines starting with `//` are comments.
//!
//! What makes it shorter than JRXML:
//!
//! - An element's position and size come first, as `x,y` and `widthxheight`.
//! - A band's height comes after its name (`title 40`); `detail 20` is a detail section with
//!   one band of that height (the same for `groupHeader` and `groupFooter`).
//! - Fields, parameters, variables, styles, groups and properties take their name (and class)
//!   as words: `field amount BigDecimal`. Common classes need no package.
//! - The main expression of a node goes after ` : ` (a text field's value, a static text's
//!   text, a variable's expression, a parameter's default value, a group's expression).
//! - The page is `page=A4` (A3, A5, Letter, Legal, or `595x842`), `landscape`, `margins=20`.
//!   Without them a report is A4 with 20-point margins.
//!
//! What makes it accessible by default: reports written in it produce tagged PDFs (for screen
//! readers) unless the report says `tagged=false`, in the language of `lang=` (or the
//! renderer's locale). `h1` to `h6` are headings (a static text tagged as one), `tag=th`
//! (or any other tag) says what an element is, and `alt="..."` describes an image.
//!
//! Anything JRXML 7 can say, the format can say: a node it has no shorthand for is written
//! with its plain name and attributes. [`to_jrxml`] and [`from_jrxml`] convert between the two
//! without losing anything.

use crate::convert::{self, JrxmlVersion};
use crate::error::{Error, Result};
use crate::xml::{self, Node};

/// The file extension of templates in this format.
pub const EXTENSION: &str = "ferro";

/// Whether `source` is in this format rather than XML.
pub fn is_template(source: &str) -> bool {
    let rest = source.trim_start_matches('\u{feff}');
    for line in rest.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with("//") {
            continue;
        }
        return !t.starts_with('<');
    }
    false
}

/// Compiles a template in this format to JRXML (as JasperReports 7 writes it).
pub fn to_jrxml(source: &str) -> Result<String> {
    let items = parse_lines(source)?;
    let mut roots = items.into_iter();
    let root = roots.next().ok_or_else(|| err(0, "the template is empty"))?;
    if let Some(extra) = roots.next() {
        return Err(err(extra.line, "a template has one `report` (or `styles`) at the top; indent the rest under it"));
    }
    let node = match root.head.as_str() {
        "report" | "jasperReport" => report(&root)?,
        "styles" | "jasperTemplate" => {
            let mut out = desugar(&root, "", None)?;
            out.name = "jasperTemplate".into();
            out
        }
        other => return Err(err(root.line, &format!("a template starts with `report`, not `{other}`"))),
    };
    Ok(convert::write(&node))
}

/// Writes a JRXML template (JasperReports 6 or 7) in this format.
pub fn from_jrxml(source: &str) -> Result<String> {
    let v7 = match convert::jrxml_version(source)? {
        JrxmlVersion::V6 => convert::jrxml_to_v7(source)?,
        JrxmlVersion::V7 => source.to_string(),
    };
    let root = xml::parse_raw(&v7)?;
    let mut out = Writer::default();
    match root.name.as_str() {
        "jasperReport" => out.report(&root),
        "jasperTemplate" => {
            out.line(0, "styles", &root, &[], None);
            for c in &root.children {
                out.node(c, "jasperTemplate", 1);
            }
        }
        other => return Err(Error::Xml(format!("<{other}> is not a JRXML template"))),
    }
    Ok(out.text)
}

fn err(line: usize, message: &str) -> Error {
    if line == 0 {
        Error::template(message)
    } else {
        Error::template(format!("line {line}: {message}"))
    }
}

// ---------------------------------------------------------------------------------------------
// Reading: lines into an outline

#[derive(Debug, Clone)]
struct Item {
    line: usize,
    head: String,
    tokens: Vec<Token>,
    text: Option<String>,
    kids: Vec<Item>,
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    /// A word, or a quoted string (`"..."`).
    Word(String),
    /// `name=value`.
    Pair(String, String),
}

fn indent_of(line: &str) -> usize {
    line.chars().take_while(|c| *c == ' ' || *c == '\t').map(|c| if c == '\t' { 4 } else { 1 }).sum()
}

fn parse_lines(source: &str) -> Result<Vec<Item>> {
    let lines: Vec<&str> = source.trim_start_matches('\u{feff}').lines().collect();
    // (indent, item) for the open items; finished ones go into their parent.
    let mut stack: Vec<(usize, Item)> = Vec::new();
    let mut roots = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let raw = lines[i].trim_end_matches('\r');
        let number = i + 1;
        i += 1;
        let content = raw.trim();
        if content.is_empty() || content.starts_with("//") {
            continue;
        }
        let indent = indent_of(raw);
        let (head, tokens, text) = parse_line(content).map_err(|m| err(number, &m))?;
        let text = match text {
            LineText::None => None,
            LineText::Inline(t) => Some(t),
            LineText::Block => {
                // The lines indented under this one, as they are (less this line's indent
                // and one level).
                let mut block: Vec<&str> = Vec::new();
                let mut cut = None;
                while i < lines.len() {
                    let l = lines[i].trim_end_matches('\r');
                    if !l.trim().is_empty() && indent_of(l) <= indent {
                        break;
                    }
                    if cut.is_none() && !l.trim().is_empty() {
                        cut = Some(indent_of(l));
                    }
                    block.push(l);
                    i += 1;
                }
                while block.last().is_some_and(|l| l.trim().is_empty()) {
                    block.pop();
                }
                let cut = cut.unwrap_or(indent + 2);
                let body: Vec<String> = block.iter().map(|l| strip_indent(l, cut)).collect();
                Some(body.join("\n"))
            }
        };
        let item = Item { line: number, head, tokens, text, kids: Vec::new() };
        while let Some((top, _)) = stack.last() {
            if *top >= indent {
                let (_, done) = stack.pop().expect("checked");
                attach(&mut stack, &mut roots, done);
            } else {
                break;
            }
        }
        stack.push((indent, item));
    }
    while let Some((_, done)) = stack.pop() {
        attach(&mut stack, &mut roots, done);
    }
    Ok(roots)
}

fn attach(stack: &mut [(usize, Item)], roots: &mut Vec<Item>, item: Item) {
    match stack.last_mut() {
        Some((_, parent)) => parent.kids.push(item),
        None => roots.push(item),
    }
}

/// `line` without its first `cut` columns of indentation (tabs count 4).
fn strip_indent(line: &str, cut: usize) -> String {
    let mut col = 0;
    for (i, c) in line.char_indices() {
        if col >= cut || (c != ' ' && c != '\t') {
            return line[i..].to_string();
        }
        col += if c == '\t' { 4 } else { 1 };
    }
    String::new()
}

enum LineText {
    None,
    Inline(String),
    Block,
}

fn parse_line(content: &str) -> std::result::Result<(String, Vec<Token>, LineText), String> {
    let mut rest = content;
    let mut head = None;
    let mut tokens = Vec::new();
    loop {
        rest = rest.trim_start();
        if rest.is_empty() {
            return Ok((head.unwrap_or_default(), tokens, LineText::None));
        }
        // The text: ` : text`, ` :` at the end (a block follows), or ` := "json"`.
        if head.is_some() {
            if let Some(json) = rest.strip_prefix(":=") {
                let json = json.trim();
                let text: String =
                    serde_json::from_str(json).map_err(|e| format!("after `:=` a JSON string is expected ({e})"))?;
                return Ok((head.unwrap_or_default(), tokens, LineText::Inline(text)));
            }
            if rest == ":" {
                return Ok((head.unwrap_or_default(), tokens, LineText::Block));
            }
            if let Some(t) = rest.strip_prefix(": ") {
                return Ok((head.unwrap_or_default(), tokens, LineText::Inline(t.to_string())));
            }
        }
        let (token, after) = read_token(rest)?;
        rest = after;
        match (&head, token) {
            (None, Token::Word(w)) => head = Some(w),
            (None, Token::Pair(..)) => return Err("a line starts with a name, not `name=value`".into()),
            (Some(_), t) => tokens.push(t),
        }
    }
}

fn read_token(s: &str) -> std::result::Result<(Token, &str), String> {
    if s.starts_with('"') {
        let (value, rest) = read_quoted(s)?;
        return Ok((Token::Word(value), rest));
    }
    let end = s.find([' ', '\t']).unwrap_or(s.len());
    let word = &s[..end];
    if let Some(eq) = word.find('=').filter(|i| *i > 0) {
        let key = &word[..eq];
        let after = &s[eq + 1..];
        if after.starts_with('"') {
            let (value, rest) = read_quoted(after)?;
            return Ok((Token::Pair(key.to_string(), value), rest));
        }
        let vend = after.find([' ', '\t']).unwrap_or(after.len());
        return Ok((Token::Pair(key.to_string(), after[..vend].to_string()), &after[vend..]));
    }
    Ok((Token::Word(word.to_string()), &s[end..]))
}

/// A JSON string at the start of `s`, and what follows it.
fn read_quoted(s: &str) -> std::result::Result<(String, &str), String> {
    let bytes = s.as_bytes();
    let mut i = 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'"' => {
                let value: String = serde_json::from_str(&s[..=i]).map_err(|e| format!("bad quoted value {}: {e}", &s[..=i]))?;
                return Ok((value, &s[i + 1..]));
            }
            _ => i += 1,
        }
    }
    Err(format!("a quoted value is not closed: {s}"))
}

// ---------------------------------------------------------------------------------------------
// Reading: the outline into JRXML 7

/// Element kinds (`<element kind="...">`); `h1`..`h6` are headings.
const ELEMENTS: &[&str] = &[
    "staticText",
    "textField",
    "image",
    "line",
    "rectangle",
    "ellipse",
    "frame",
    "subreport",
    "break",
    "elementGroup",
    "component",
    "generic",
    "crosstab",
    "chart",
];

/// Sections whose first word is a height.
const BANDS: &[&str] = &[
    "background",
    "title",
    "pageHeader",
    "columnHeader",
    "columnFooter",
    "pageFooter",
    "lastPageFooter",
    "summary",
    "noData",
    "band",
];

/// Sections that hold bands: `detail 20` is one band of that height.
const SECTIONS: &[&str] = &["detail", "groupHeader", "groupFooter"];

const TAG_PROPERTY: &str = "net.sf.jasperreports.export.accessibility.tag";
const TAGGED_PROPERTY: &str = "net.sf.jasperreports.export.pdf.tagged";
const LANGUAGE_PROPERTY: &str = "net.sf.jasperreports.export.pdf.tag.language";
const PDFA_PROPERTY: &str = "net.sf.jasperreports.export.pdfa.conformance";

/// Classes that may be written without their package.
const CLASSES: &[(&str, &str)] = &[
    ("String", "java.lang.String"),
    ("Integer", "java.lang.Integer"),
    ("Long", "java.lang.Long"),
    ("Short", "java.lang.Short"),
    ("Byte", "java.lang.Byte"),
    ("Double", "java.lang.Double"),
    ("Float", "java.lang.Float"),
    ("Boolean", "java.lang.Boolean"),
    ("Number", "java.lang.Number"),
    ("Object", "java.lang.Object"),
    ("Character", "java.lang.Character"),
    ("BigDecimal", "java.math.BigDecimal"),
    ("BigInteger", "java.math.BigInteger"),
    ("Date", "java.util.Date"),
    ("Timestamp", "java.sql.Timestamp"),
    ("Time", "java.sql.Time"),
    ("SqlDate", "java.sql.Date"),
    ("Collection", "java.util.Collection"),
    ("List", "java.util.List"),
    ("Map", "java.util.Map"),
    ("Image", "java.awt.Image"),
    ("InputStream", "java.io.InputStream"),
    ("DataSource", "net.sf.jasperreports.engine.JRDataSource"),
];

fn long_class(name: &str) -> String {
    CLASSES.iter().find(|(s, _)| *s == name).map(|(_, l)| l.to_string()).unwrap_or_else(|| name.to_string())
}

fn short_class(name: &str) -> String {
    CLASSES.iter().find(|(_, l)| *l == name).map(|(s, _)| s.to_string()).unwrap_or_else(|| name.to_string())
}

/// Page sizes by name, in points (portrait).
const PAGES: &[(&str, f32, f32)] =
    &[("A4", 595.0, 842.0), ("A3", 842.0, 1191.0), ("A5", 420.0, 595.0), ("Letter", 612.0, 792.0), ("Legal", 612.0, 1008.0)];

const DEFAULT_MARGIN: &str = "20";

/// The words a node takes before its flags, by node name (and the name of its parent).
fn slots(name: &str, parent: &str) -> &'static [&'static str] {
    match name {
        "field" | "variable" | "measure" => &["name", "class"],
        "parameter" if matches!(parent, "jasperReport" | "dataset" | "element") => &["name", "class"],
        "parameter" | "propertyExpression" | "style" | "group" | "sortField" | "scriptlet" | "rowGroup" | "columnGroup"
        | "hyperlinkParameter" => &["name"],
        "dataset" if parent == "jasperReport" => &["name"],
        "property" => &["name", "value"],
        "query" => &["language"],
        "bucket" => &["class"],
        _ => &[],
    }
}

/// The child a node's ` : text` goes into (`""`: the node's own text).
fn main_text(name: &str, kind: Option<&str>, parent: &str) -> Option<&'static str> {
    Some(match (name, kind) {
        ("element", Some("staticText")) => "text",
        ("element", Some("textField" | "image" | "subreport")) => "expression",
        ("parameter", _) if matches!(parent, "jasperReport" | "dataset") => "defaultValueExpression",
        ("field", _) => "description",
        ("variable" | "group" | "bucket" | "measure" | "parameter" | "hyperlinkParameter", _) => "expression",
        ("element" | "component" | "jasperReport" | "dataset" | "style" | "band" | "detail", _) => return None,
        _ => "",
    })
}

fn report(root: &Item) -> Result<Node> {
    let mut out = desugar(root, "", None)?;
    out.name = "jasperReport".into();
    // The page: page=A4 landscape margins=20, or the plain JRXML attributes.
    let mut page = PAGES[0];
    let mut landscape = false;
    let mut margins: Option<String> = None;
    let mut lang = None;
    let mut tagged = true;
    let mut pdfa = None;
    let mut attrs = Vec::new();
    for (k, v) in std::mem::take(&mut out.attrs) {
        match k.as_str() {
            "page" => {
                page = match PAGES.iter().find(|p| p.0.eq_ignore_ascii_case(&v)) {
                    Some(p) => *p,
                    None => {
                        let (w, h) = size(&v)
                            .ok_or_else(|| err(root.line, &format!("page={v}: say A4, A3, A5, Letter, Legal or WIDTHxHEIGHT")))?;
                        ("", w, h)
                    }
                }
            }
            "landscape" => landscape = v == "true",
            "margins" => margins = Some(v),
            "lang" => lang = Some(v),
            "tagged" => tagged = v != "false",
            "pdfa" => pdfa = Some(v),
            _ => attrs.push((k, v)),
        }
    }
    let has = |a: &[(String, String)], k: &str| a.iter().any(|(n, _)| n == k);
    let (mut w, mut h) = (page.1, page.2);
    if landscape {
        std::mem::swap(&mut w, &mut h);
        if !has(&attrs, "orientation") {
            attrs.push(("orientation".into(), "Landscape".into()));
        }
    }
    let set_default = |attrs: &mut Vec<(String, String)>, k: &str, v: String| {
        if !has(attrs, k) {
            attrs.push((k.to_string(), v));
        }
    };
    set_default(&mut attrs, "pageWidth", fmt(w));
    set_default(&mut attrs, "pageHeight", fmt(h));
    // margins=20, or 20,30 (top and bottom, left and right), or 20,30,20,30 (top, right,
    // bottom, left).
    let m: Vec<String> = margins.as_deref().unwrap_or(DEFAULT_MARGIN).split(',').map(|s| s.trim().to_string()).collect();
    let (top, right, bottom, left) = match m.as_slice() {
        [a] => (a.clone(), a.clone(), a.clone(), a.clone()),
        [v, hz] => (v.clone(), hz.clone(), v.clone(), hz.clone()),
        [t, r, b, l] => (t.clone(), r.clone(), b.clone(), l.clone()),
        _ => return Err(err(root.line, "margins= takes one, two or four numbers")),
    };
    set_default(&mut attrs, "topMargin", top);
    set_default(&mut attrs, "rightMargin", right);
    set_default(&mut attrs, "bottomMargin", bottom);
    set_default(&mut attrs, "leftMargin", left);
    if !has(&attrs, "columnWidth") {
        let num = |k: &str| attrs.iter().find(|(n, _)| n == k).and_then(|(_, v)| v.parse::<f32>().ok()).unwrap_or(0.0);
        let count = num("columnCount").max(1.0);
        let spacing = num("columnSpacing");
        let width = ((num("pageWidth") - num("leftMargin") - num("rightMargin") - spacing * (count - 1.0)) / count).floor();
        attrs.push(("columnWidth".into(), fmt(width)));
    }
    out.attrs = attrs;
    // Accessible by default: a tagged PDF, in the report's language.
    let mut props = Vec::new();
    let has_prop = |n: &Node, name: &str| n.children.iter().any(|c| c.name == "property" && c.attr("name") == Some(name));
    if tagged && !has_prop(&out, TAGGED_PROPERTY) {
        props.push(property(TAGGED_PROPERTY, "true"));
    }
    if let Some(lang) = lang.filter(|_| !has_prop(&out, LANGUAGE_PROPERTY)) {
        props.push(property(LANGUAGE_PROPERTY, &lang));
    }
    if let Some(p) = pdfa.filter(|_| !has_prop(&out, PDFA_PROPERTY)) {
        let value = if p.to_ascii_lowercase().starts_with("pdfa") { p } else { format!("pdfa{p}") };
        props.push(property(PDFA_PROPERTY, &value));
    }
    out.children.splice(0..0, props);
    order_report(&mut out);
    Ok(out)
}

fn fmt(v: f32) -> String {
    if v.fract() == 0.0 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

fn size(s: &str) -> Option<(f32, f32)> {
    let (w, h) = s.split_once('x')?;
    Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
}

fn property(name: &str, value: &str) -> Node {
    Node {
        name: "property".into(),
        attrs: vec![("name".into(), name.into()), ("value".into(), value.into())],
        ..Default::default()
    }
}

/// Puts the report's children in the order JasperReports 7 reads them in.
fn order_report(n: &mut Node) {
    const ORDER: &[&str] = &[
        "property",
        "propertyExpression",
        "import",
        "template",
        "style",
        "dataset",
        "scriptlet",
        "parameter",
        "query",
        "field",
        "sortField",
        "variable",
        "filterExpression",
        "group",
        "background",
        "title",
        "pageHeader",
        "columnHeader",
        "detail",
        "columnFooter",
        "pageFooter",
        "lastPageFooter",
        "summary",
        "noData",
    ];
    let rank = |c: &Node| ORDER.iter().position(|o| *o == c.name).unwrap_or(ORDER.len());
    n.children.sort_by_key(rank);
}

/// One outline node as JRXML 7, its parent being `parent` (with `parent_kind`).
fn desugar(item: &Item, parent: &str, parent_kind: Option<&str>) -> Result<Node> {
    let head = item.head.as_str();
    let mut out = Node::default();
    let mut heading = None;
    // What the node is.
    let in_component = parent == "element" && parent_kind == Some("component");
    if let Some(level) = head.strip_prefix('h').and_then(|l| l.parse::<u8>().ok()).filter(|l| (1..=6).contains(l)) {
        out.name = "element".into();
        out.attrs.push(("kind".into(), "staticText".into()));
        heading = Some(level);
    } else if in_component && !matches!(head, "property" | "propertyExpression" | "printWhenExpression") {
        out.name = "component".into();
        out.attrs.push(("kind".into(), head.into()));
    } else if ELEMENTS.contains(&head) && !clashes(head, parent) {
        out.name = "element".into();
        out.attrs.push(("kind".into(), head.into()));
    } else if head == "report" {
        out.name = "jasperReport".into();
    } else {
        out.name = head.to_string();
    }
    let name = out.name.clone();
    let kind = out.attr("kind").map(str::to_string);
    let is_element = name == "element";

    // Words and pairs.
    let slots = if head == "report" || head == "jasperReport" { &["name"][..] } else { slots(&name, parent) };
    let mut slot = 0;
    let mut extra = Vec::new();
    for t in &item.tokens {
        match t {
            Token::Pair(k, v) => match (is_element, k.as_str()) {
                (true, "tag") => extra.push(property(TAG_PROPERTY, v)),
                (true, "alt") => {
                    let mut e = Node { name: "hyperlinkTooltipExpression".into(), ..Default::default() };
                    e.text = java_string(v);
                    extra.push(e);
                }
                (_, "class") => out.attrs.push(("class".into(), long_class(v))),
                _ => out.attrs.push((k.clone(), v.clone())),
            },
            Token::Word(w) => {
                if slot < slots.len() {
                    let key = slots[slot];
                    let value = if key == "class" { long_class(w) } else { w.clone() };
                    out.attrs.push((key.to_string(), value));
                    slot += 1;
                } else if is_element && w.contains(',') && geometry(w, ',').is_some() {
                    let (x, y) = geometry(w, ',').expect("checked");
                    out.attrs.push(("x".into(), x));
                    out.attrs.push(("y".into(), y));
                } else if is_element && geometry(w, 'x').is_some() {
                    let (wd, ht) = geometry(w, 'x').expect("checked");
                    out.attrs.push(("width".into(), wd));
                    out.attrs.push(("height".into(), ht));
                } else if (BANDS.contains(&name.as_str()) || SECTIONS.contains(&name.as_str())) && w.parse::<f32>().is_ok() {
                    out.attrs.push(("height".into(), w.clone()));
                } else {
                    out.attrs.push((w.clone(), "true".into()));
                }
            }
        }
    }
    if let Some(level) = heading {
        extra.insert(0, property(TAG_PROPERTY, &format!("h{level}")));
    }

    // Children.
    let mut kids = Vec::new();
    for k in &item.kids {
        kids.push(desugar(k, &name, kind.as_deref())?);
    }

    // The text.
    if let Some(text) = &item.text {
        match main_text(&name, kind.as_deref(), parent) {
            Some("") => out.text = text.clone(),
            Some(child) => kids.insert(0, Node { name: child.into(), text: text.clone(), ..Default::default() }),
            None => return Err(err(item.line, &format!("`{head}` has no text after ` : `"))),
        }
    }

    // `detail 20`: a section with one band.
    if SECTIONS.contains(&name.as_str()) {
        if let Some(i) = out.attrs.iter().position(|(k, _)| k == "height") {
            let mut band = Node { name: "band".into(), ..Default::default() };
            band.attrs.push(out.attrs.remove(i));
            band.attrs.append(&mut out.attrs);
            band.children = kids;
            kids = vec![band];
        }
    }

    // Element children in the order JasperReports reads them: what comes before the
    // element's own content, then the rest.
    if is_element {
        let first: Vec<Node> = extra.into_iter().collect();
        let (props, rest): (Vec<Node>, Vec<Node>) =
            kids.into_iter().partition(|c| matches!(c.name.as_str(), "property" | "propertyExpression"));
        let mut all = rest;
        let tooltip = first.iter().any(|c| c.name == "hyperlinkTooltipExpression");
        let (tip, first): (Vec<Node>, Vec<Node>) = first.into_iter().partition(|c| c.name == "hyperlinkTooltipExpression");
        if tooltip {
            let at = all.iter().position(|c| c.name == "box").unwrap_or(all.len());
            all.splice(at..at, tip);
        }
        let at = all.iter().position(|c| c.name == "box").unwrap_or(all.len());
        all.splice(at..at, first.into_iter().chain(props));
        out.children = all;
    } else {
        out.children = extra;
        out.children.extend(kids);
    }
    Ok(out)
}

/// Whether `head` under `parent` is a plain node that shares its name with an element kind
/// (a multiple axis chart's `<chart>`, a book part's `<component>`).
fn clashes(head: &str, parent: &str) -> bool {
    (head == "chart" && parent == "axis") || (head == "component" && parent == "part")
}

/// `"12,30"` -> ("12", "30") for `,`; `"100x20"` -> ("100", "20") for `x`.
fn geometry(w: &str, sep: char) -> Option<(String, String)> {
    let (a, b) = w.split_once(sep)?;
    let ok = |s: &str| !s.is_empty() && s.parse::<f32>().is_ok();
    (ok(a) && ok(b)).then(|| (a.to_string(), b.to_string()))
}

/// A Java string literal for `s`.
fn java_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The text of a Java string literal, if `s` is exactly one with simple escapes.
fn java_literal(s: &str) -> Option<String> {
    let inner = s.trim().strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return None,
            '\\' => out.push(match chars.next()? {
                '"' => '"',
                '\\' => '\\',
                'n' => '\n',
                't' => '\t',
                'r' => '\r',
                _ => return None,
            }),
            c => out.push(c),
        }
    }
    Some(out)
}

// ---------------------------------------------------------------------------------------------
// Writing: JRXML 7 into the outline

#[derive(Default)]
struct Writer {
    text: String,
}

impl Writer {
    fn report(&mut self, root: &Node) {
        let mut attrs: Vec<(String, String)> = root.attrs.clone();
        let mut extra = Vec::new();
        let get = |a: &[(String, String)], k: &str| a.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
        let num = |a: &[(String, String)], k: &str| get(a, k).and_then(|v| v.parse::<f32>().ok());
        let landscape = get(&attrs, "orientation").as_deref() == Some("Landscape");
        // The page by name, when it has one.
        if let (Some(w), Some(h)) = (num(&attrs, "pageWidth"), num(&attrs, "pageHeight")) {
            let (pw, ph) = if landscape { (h, w) } else { (w, h) };
            let named = PAGES.iter().find(|p| p.1 == pw && p.2 == ph);
            attrs.retain(|(k, _)| k != "pageWidth" && k != "pageHeight" && !(landscape && k == "orientation"));
            match named {
                Some(p) if p.0 == "A4" => {}
                Some(p) => extra.push(("page".to_string(), p.0.to_string())),
                None => extra.push(("page".to_string(), format!("{}x{}", fmt(pw), fmt(ph)))),
            }
            if landscape {
                extra.push(("landscape".to_string(), "true".to_string()));
            }
        }
        // Margins, in one go. JRXML without them means 30 at the top and bottom and 20 at
        // the sides.
        let m: Vec<String> = [("topMargin", "30"), ("rightMargin", "20"), ("bottomMargin", "30"), ("leftMargin", "20")]
            .iter()
            .map(|(k, d)| get(&attrs, k).unwrap_or_else(|| d.to_string()))
            .collect();
        let said = if m.iter().all(|v| *v == m[0]) {
            m[0].clone()
        } else if m[0] == m[2] && m[1] == m[3] {
            format!("{},{}", m[0], m[1])
        } else {
            format!("{},{},{},{}", m[0], m[1], m[2], m[3])
        };
        attrs.retain(|(k, _)| !k.ends_with("Margin"));
        if said != DEFAULT_MARGIN {
            extra.push(("margins".to_string(), said));
        }
        // The column width, when it is what the page leaves.
        let page = |k: &str| {
            num(&root.attrs, k).unwrap_or(match k {
                "pageWidth" => 595.0,
                "leftMargin" | "rightMargin" => 20.0,
                _ => 0.0,
            })
        };
        let count = num(&root.attrs, "columnCount").unwrap_or(1.0).max(1.0);
        let left_over = ((page("pageWidth") - page("leftMargin") - page("rightMargin") - page("columnSpacing") * (count - 1.0))
            / count)
            .floor();
        if num(&root.attrs, "columnWidth") == Some(left_over) {
            attrs.retain(|(k, _)| k != "columnWidth");
        }
        // Accessibility: tagged unless the template does not say so.
        let mut skip = Vec::new();
        let prop = |name: &str| root.children.iter().position(|c| c.name == "property" && c.attr("name") == Some(name));
        match prop(TAGGED_PROPERTY) {
            Some(i) if root.children[i].attr("value").map(str::trim) == Some("true") => skip.push(i),
            _ => extra.push(("tagged".to_string(), "false".to_string())),
        }
        if let Some(i) = prop(LANGUAGE_PROPERTY) {
            if let Some(v) = root.children[i].attr("value") {
                extra.insert(0, ("lang".to_string(), v.to_string()));
                skip.push(i);
            }
        }
        if let Some(i) = prop(PDFA_PROPERTY) {
            if let Some(v) = root.children[i].attr("value") {
                let v = v.trim();
                extra.push(("pdfa".to_string(), v.strip_prefix("pdfa").unwrap_or(v).to_string()));
                skip.push(i);
            }
        }
        let name = get(&attrs, "name").unwrap_or_default();
        attrs.retain(|(k, _)| k != "name");
        let mut words = vec![quote(&name)];
        for (k, v) in extra.iter().chain(attrs.iter()) {
            words.push(pair(k, v, true));
        }
        self.text.push_str(&format!("report {}\n", words.join(" ")));
        for (i, c) in root.children.iter().enumerate() {
            if !skip.contains(&i) {
                self.node(c, "jasperReport", 1);
            }
        }
    }

    fn push_line(&mut self, depth: usize, s: &str) {
        for _ in 0..depth {
            self.text.push_str("  ");
        }
        self.text.push_str(s);
        self.text.push('\n');
    }

    /// Writes `n` (a child of `parent`) at `depth`.
    fn node(&mut self, n: &Node, parent: &str, depth: usize) {
        let kind = n.attr("kind");
        let is_element = n.name == "element";
        let in_component = parent == "component-element" && n.name == "component";
        let mut head = if is_element || in_component { kind.unwrap_or("element").to_string() } else { n.name.clone() };
        let mut words: Vec<String> = Vec::new();
        let mut attrs: Vec<(String, String)> =
            n.attrs.iter().filter(|(k, _)| !((is_element || in_component) && k == "kind")).cloned().collect();
        let mut children: Vec<&Node> = n.children.iter().collect();

        // Headings and tags.
        if is_element {
            if let Some(i) = children
                .iter()
                .position(|c| c.name == "property" && c.attr("name") == Some(TAG_PROPERTY) && c.children.is_empty())
            {
                let tag = children[i].attr("value").unwrap_or_default().to_string();
                children.remove(i);
                let level = tag.strip_prefix('h').and_then(|l| l.parse::<u8>().ok()).filter(|l| (1..=6).contains(l));
                match level {
                    Some(_) if kind == Some("staticText") => head = tag.clone(),
                    _ => attrs.push(("tag".into(), tag)),
                }
            }
            if kind == Some("image") {
                if let Some(i) = children.iter().position(|c| c.name == "hyperlinkTooltipExpression" && c.attrs.is_empty()) {
                    if let Some(alt) = java_literal(&children[i].text) {
                        children.remove(i);
                        attrs.push(("alt".into(), alt));
                    }
                }
            }
        }

        // Words: the name (and class), the position and size, the height.
        let slots = slots(&n.name, parent);
        let mut all_slots = true;
        for key in slots {
            match attrs.iter().position(|(k, _)| k == key) {
                Some(i) if all_slots => {
                    let (_, v) = attrs.remove(i);
                    words.push(quote(&if *key == "class" { short_class(&v) } else { v }));
                }
                _ => all_slots = false,
            }
        }
        if is_element {
            let take =
                |attrs: &mut Vec<(String, String)>, k: &str| attrs.iter().position(|(n, _)| n == k).map(|i| attrs.remove(i).1);
            let (x, y) = (take(&mut attrs, "x"), take(&mut attrs, "y"));
            if x.is_some() || y.is_some() {
                words.push(format!("{},{}", x.unwrap_or_else(|| "0".into()), y.unwrap_or_else(|| "0".into())));
            }
            let (w, h) = (take(&mut attrs, "width"), take(&mut attrs, "height"));
            match (w, h) {
                (Some(w), Some(h)) => words.push(format!("{w}x{h}")),
                (w, h) => {
                    attrs.extend(w.map(|w| ("width".to_string(), w)));
                    attrs.extend(h.map(|h| ("height".to_string(), h)));
                }
            }
        }
        let band_like = BANDS.contains(&n.name.as_str());
        if band_like {
            if let Some(i) = attrs.iter().position(|(k, _)| k == "height") {
                words.push(attrs.remove(i).1);
            }
        }
        // `detail 20`: a section of one band.
        let mut section_band = None;
        if SECTIONS.contains(&n.name.as_str()) && n.attrs.is_empty() && children.len() == 1 && children[0].name == "band" {
            let band = children[0];
            if let Some(h) = band.attr("height") {
                words.push(h.to_string());
                attrs.extend(band.attrs.iter().filter(|(k, _)| k != "height").cloned());
                section_band = Some(band);
            }
        }
        if let Some(band) = section_band {
            children = band.children.iter().collect();
        }
        for (k, v) in &attrs {
            let v = if k == "class" { short_class(v) } else { v.clone() };
            words.push(pair(k, &v, all_slots));
        }

        // The text: the main child's, or the node's own.
        let mut text = None;
        let leaf = children.is_empty();
        match main_text(&n.name, kind, parent) {
            Some("") if leaf && !n.text.is_empty() => text = Some(n.text.clone()),
            Some(child) if !child.is_empty() => {
                if let Some(i) = children.iter().position(|c| c.name == child && c.attrs.is_empty() && c.children.is_empty()) {
                    // The main child goes on this line only if it would be read back first.
                    let simple = !needs_block(&children[i].text);
                    if simple || children.len() == 1 {
                        text = Some(children[i].text.clone());
                        children.remove(i);
                    }
                }
            }
            _ => {}
        }
        let mut line = head.clone();
        for w in &words {
            line.push(' ');
            line.push_str(w);
        }
        match text {
            Some(t) if !needs_block(&t) => {
                line.push_str(" : ");
                line.push_str(&t);
                self.push_line(depth, &line);
            }
            Some(t) if exact_block(&t) && children.is_empty() => {
                line.push_str(" :");
                self.push_line(depth, &line);
                for l in t.split('\n') {
                    if l.is_empty() {
                        self.text.push('\n');
                    } else {
                        self.push_line(depth + 1, l);
                    }
                }
            }
            Some(t) => {
                line.push_str(" := ");
                line.push_str(&serde_json::to_string(&t).expect("strings serialize"));
                self.push_line(depth, &line);
            }
            None => self.push_line(depth, &line),
        }
        let child_parent = if is_element && kind == Some("component") { "component-element" } else { n.name.as_str() };
        for c in children {
            self.node(c, child_parent, depth + 1);
        }
    }

    fn line(&mut self, depth: usize, head: &str, n: &Node, skip: &[&str], text: Option<&str>) {
        let mut line = head.to_string();
        for (k, v) in &n.attrs {
            if !skip.contains(&k.as_str()) {
                line.push(' ');
                line.push_str(&pair(k, v, true));
            }
        }
        if let Some(t) = text {
            line.push_str(" : ");
            line.push_str(t);
        }
        self.push_line(depth, &line);
    }
}

/// A text that cannot go after ` : ` on one line.
fn needs_block(t: &str) -> bool {
    t.is_empty() || t.contains('\n') || t.contains('\r') || t.trim() != t
}

/// A text the block form gives back exactly: lines without trailing spaces, that do not
/// start or end with a blank line, that do not start with spaces.
fn exact_block(t: &str) -> bool {
    !t.is_empty()
        && !t.contains('\r')
        && !t.contains('\t')
        && !t.starts_with('\n')
        && !t.ends_with('\n')
        && t.split('\n').all(|l| l == l.trim_end())
        && !t.split('\n').next().is_some_and(|l| l.starts_with(' '))
}

/// `name=value`, quoted when needed; `name` alone for true (when `bare` allows it).
fn pair(k: &str, v: &str, bare: bool) -> String {
    if bare && v == "true" {
        return k.to_string();
    }
    format!("{k}={}", quote(v))
}

fn quote(v: &str) -> String {
    let plain = !v.is_empty()
        && !v.starts_with('"')
        && !v.starts_with(':')
        && !v.starts_with("//")
        && !v.chars().any(|c| c.is_whitespace() || c == '"' || c == '\\');
    if plain {
        v.to_string()
    } else {
        serde_json::to_string(v).expect("strings serialize")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EMPLOYEES: &str = r#"
// A list of employees.
report "Employees" lang=en
  style Base default fontName="DejaVu Sans" fontSize=10
  field name String
  field salary Double
  variable total Double calculation=Sum : $F{salary}

  title 40
    h1 0,0 555x30 fontSize=18 : Employees
  columnHeader 20
    staticText 0,0 300x20 bold : Name
    staticText 300,0 255x20 bold hTextAlign=Right : Salary
  detail 20
    textField 0,0 300x20 : $F{name}
    textField 300,0 255x20 pattern=#,##0.00 hTextAlign=Right : $F{salary}
    image 0,0 20x20 alt="A \"photo\"" :
      $P{photos}
        .get($F{name})
  summary 20
    textField 300,0 255x20 bold hTextAlign=Right : $V{total}
"#;

    #[test]
    fn compiles_to_jrxml_7() {
        assert!(is_template(EMPLOYEES));
        let x = to_jrxml(EMPLOYEES).unwrap();
        assert!(x.contains(r#"<jasperReport name="Employees" pageWidth="595" pageHeight="842" topMargin="20" rightMargin="20" bottomMargin="20" leftMargin="20" columnWidth="555">"#), "{x}");
        assert!(x.contains(r#"<property name="net.sf.jasperreports.export.pdf.tagged" value="true"/>"#));
        assert!(x.contains(r#"<property name="net.sf.jasperreports.export.pdf.tag.language" value="en"/>"#));
        assert!(x.contains(r#"<style name="Base" default="true" fontName="DejaVu Sans" fontSize="10"/>"#));
        assert!(x.contains(r#"<field name="salary" class="java.lang.Double"/>"#));
        assert!(x.contains(r#"<variable name="total" class="java.lang.Double" calculation="Sum">"#));
        assert!(x.contains(r#"<element kind="staticText" x="0" y="0" width="555" height="30" fontSize="18">"#));
        assert!(x.contains(r#"<property name="net.sf.jasperreports.export.accessibility.tag" value="h1"/>"#));
        assert!(x.contains(r#"<detail>"#) && x.contains(r#"<band height="20">"#));
        assert!(x.contains(r##"pattern="#,##0.00""##));
        assert!(x.contains("<expression><![CDATA[$P{photos}\n  .get($F{name})]]></expression>"), "{x}");
        assert!(x.contains(r#"<hyperlinkTooltipExpression><![CDATA["A \"photo\""]]></hyperlinkTooltipExpression>"#));
        let report = crate::jrxml::parse_report(&x).unwrap();
        assert!(report.pdf.tagged);
        assert_eq!(report.pdf.language.as_deref(), Some("en"));
        assert_eq!(report.fields.len(), 2);
        // The renderer reads the format directly.
        let direct = crate::jrxml::parse_report(EMPLOYEES).unwrap();
        assert_eq!(format!("{:?}", direct.detail), format!("{:?}", report.detail));
    }

    #[test]
    fn round_trips_through_jrxml() {
        let x = to_jrxml(EMPLOYEES).unwrap();
        let back = from_jrxml(&x).unwrap();
        assert_eq!(to_jrxml(&back).unwrap(), x, "{back}");
        assert!(back.starts_with("report Employees lang=en\n"), "{back}");
        assert!(back.contains("  detail 20\n    textField 0,0 300x20 : $F{name}\n"), "{back}");
        assert!(back.contains("    h1 0,0 555x30 fontSize=18 : Employees\n"), "{back}");
        assert!(back.contains(r#"image 0,0 20x20 alt="A \"photo\"" :"#), "{back}");
        assert!(back.contains("  field salary Double\n"), "{back}");
    }

    #[test]
    fn keeps_texts_exactly() {
        for text in ["", " padded ", "two\nlines", "tab\there", "trailing\n", "a ]]> b", "x : y", "\"quoted\""] {
            let src = format!("report r\n  title 20\n    staticText 0,0 10x10 := {}\n", serde_json::to_string(text).unwrap());
            let x = to_jrxml(&src).unwrap();
            let back = from_jrxml(&x).unwrap();
            let again = to_jrxml(&back).unwrap();
            assert_eq!(again, x, "{text:?}: {back}");
            let report = crate::jrxml::parse_report(&again).unwrap();
            let crate::model::ElementKind::Text(t) = &report.title.as_ref().unwrap().elements[0].kind else { panic!() };
            assert!(format!("{:?}", t.content).contains(&format!("{text:?}")), "{text:?} {:?}", t.content);
        }
    }

    #[test]
    fn reports_errors_with_line_numbers() {
        let e = to_jrxml("report r\n  title 20\n    staticText 0,0 10x10 x=\"open\n").unwrap_err();
        assert!(e.to_string().contains("line 3"), "{e}");
        let e = to_jrxml("report r\nreport s\n").unwrap_err();
        assert!(e.to_string().contains("line 2"), "{e}");
    }
}
