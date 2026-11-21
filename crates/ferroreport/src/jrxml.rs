//! JRXML template parser (JasperReports 6 and 7 schemas).
//!
//! Styles are resolved here, so the filler only ever sees concrete fonts, colors and pens.
//! Elements this engine cannot render are kept as [`ElementKind::Unsupported`] and listed in
//! [`Report::warnings`], so a template always loads and the `doctor` command can explain
//! what is missing.

use crate::error::{Error, Result};
use crate::expr::{parse_expr, Expr};
use crate::model::*;
use crate::value::JavaType;
use crate::xml::{self, Node};
use std::collections::HashMap;

/// Parses JRXML source into a [`Report`].
pub fn parse_report(source: &str) -> Result<Report> {
    let root = xml::parse(source)?;
    if root.name != "jasperReport" {
        return Err(Error::template(format!("root element is <{}>, expected <jasperReport>", root.name)));
    }
    let mut report = Parser::new(&root)?.report(&root)?;
    report.uses_data_source = source.contains("REPORT_DATA_SOURCE");
    Ok(report)
}

// ---------------------------------------------------------------------------------------------
// Attribute helpers

fn parse_f32(v: &str, what: &str) -> Result<f32> {
    v.trim().parse::<f32>().map_err(|_| Error::template(format!("invalid number `{v}` for {what}")))
}

fn parse_bool(v: &str) -> bool {
    v.trim().eq_ignore_ascii_case("true")
}

/// First attribute among `names` present on `node`.
fn attr<'a>(node: &'a Node, names: &[&str]) -> Option<&'a str> {
    names.iter().find_map(|n| node.attr(n))
}

fn parse_color(v: &str) -> Option<Color> {
    let v = v.trim();
    if let Some(hex) = v.strip_prefix('#') {
        let hex = if hex.len() == 3 { hex.chars().flat_map(|c| [c, c]).collect::<String>() } else { hex.to_string() };
        let n = u32::from_str_radix(&hex[hex.len().saturating_sub(6)..], 16).ok()?;
        return Some(Color { r: (n >> 16) as u8, g: (n >> 8) as u8, b: n as u8 });
    }
    let (r, g, b) = match v.to_ascii_lowercase().as_str() {
        "black" => (0, 0, 0),
        "white" => (255, 255, 255),
        "red" => (255, 0, 0),
        "green" => (0, 255, 0),
        "blue" => (0, 0, 255),
        "yellow" => (255, 255, 0),
        "cyan" => (0, 255, 255),
        "magenta" => (255, 0, 255),
        "orange" => (255, 200, 0),
        "pink" => (255, 175, 175),
        "gray" => (128, 128, 128),
        "lightgray" => (192, 192, 192),
        "darkgray" => (64, 64, 64),
        _ => return None,
    };
    Some(Color { r, g, b })
}

// ---------------------------------------------------------------------------------------------
// Style properties: every field optional so styles can be layered.

#[derive(Debug, Clone, Default)]
struct PenProps {
    width: Option<f32>,
    color: Option<Color>,
    style: Option<LineStyle>,
}

impl PenProps {
    fn read(node: &Node) -> PenProps {
        PenProps {
            width: node.attr("lineWidth").and_then(|v| v.trim().parse().ok()),
            color: node.attr("lineColor").and_then(parse_color),
            style: node.attr("lineStyle").map(|s| match s {
                "Dashed" => LineStyle::Dashed,
                "Dotted" => LineStyle::Dotted,
                "Double" => LineStyle::Double,
                _ => LineStyle::Solid,
            }),
        }
    }
    fn over(&self, base: &PenProps) -> PenProps {
        PenProps { width: self.width.or(base.width), color: self.color.or(base.color), style: self.style.or(base.style) }
    }
    fn resolve(&self, default_width: f32, default_color: Color) -> Pen {
        Pen {
            width: self.width.unwrap_or(default_width),
            color: self.color.unwrap_or(default_color),
            style: self.style.unwrap_or(LineStyle::Solid),
        }
    }
}

#[derive(Debug, Clone, Default)]
struct StyleProps {
    font_name: Option<String>,
    font_size: Option<f32>,
    bold: Option<bool>,
    italic: Option<bool>,
    underline: Option<bool>,
    strike: Option<bool>,
    forecolor: Option<Color>,
    backcolor: Option<Color>,
    mode: Option<Mode>,
    halign: Option<HAlign>,
    valign: Option<VAlign>,
    line_spacing: Option<String>,
    line_spacing_size: Option<f32>,
    pattern: Option<String>,
    blank_when_null: Option<bool>,
    radius: Option<f32>,
    /// all, top, left, bottom, right
    padding: [Option<f32>; 5],
    /// box pen, top, left, bottom, right
    box_pens: [PenProps; 5],
    graphic_pen: PenProps,
}

macro_rules! layer {
    ($self:ident, $base:ident, $($f:ident),*) => { StyleProps { $($f: $self.$f.clone().or($base.$f.clone()),)* ..Default::default() } };
}

impl StyleProps {
    /// `self` on top of `base`.
    fn over(&self, base: &StyleProps) -> StyleProps {
        let mut out = layer!(
            self,
            base,
            font_name,
            font_size,
            bold,
            italic,
            underline,
            strike,
            forecolor,
            backcolor,
            mode,
            halign,
            valign,
            line_spacing,
            line_spacing_size,
            pattern,
            blank_when_null,
            radius
        );
        for i in 0..5 {
            out.padding[i] = self.padding[i].or(base.padding[i]);
            out.box_pens[i] = self.box_pens[i].over(&base.box_pens[i]);
        }
        out.graphic_pen = self.graphic_pen.over(&base.graphic_pen);
        out
    }

    /// Reads font-ish attributes that may sit on `<font>`, `<style>` or a v7 `<element>`.
    fn read_font_attrs(&mut self, n: &Node) {
        if let Some(v) = attr(n, &["fontName"]) {
            self.font_name = Some(v.to_string());
        }
        if let Some(v) = attr(n, &["size", "fontSize"]).and_then(|v| v.trim().parse().ok()) {
            self.font_size = Some(v);
        }
        let b = |names: &[&str]| attr(n, names).map(parse_bool);
        self.bold = b(&["isBold", "bold"]).or(self.bold);
        self.italic = b(&["isItalic", "italic"]).or(self.italic);
        self.underline = b(&["isUnderline", "underline"]).or(self.underline);
        self.strike = b(&["isStrikeThrough", "strikeThrough"]).or(self.strike);
    }

    fn read_alignment(&mut self, n: &Node) {
        if let Some(v) = attr(n, &["hTextAlign", "textAlignment", "hAlign", "horizontalTextAlign"]) {
            self.halign = Some(match v {
                "Center" => HAlign::Center,
                "Right" => HAlign::Right,
                "Justified" => HAlign::Justified,
                _ => HAlign::Left,
            });
        }
        if let Some(v) = attr(n, &["vTextAlign", "verticalAlignment", "vAlign", "verticalTextAlign"]) {
            self.valign = Some(match v {
                "Middle" => VAlign::Middle,
                "Bottom" => VAlign::Bottom,
                _ => VAlign::Top,
            });
        }
    }

    fn read_colors(&mut self, n: &Node) {
        if let Some(c) = n.attr("forecolor").and_then(parse_color) {
            self.forecolor = Some(c);
        }
        if let Some(c) = n.attr("backcolor").and_then(parse_color) {
            self.backcolor = Some(c);
        }
        if let Some(m) = n.attr("mode") {
            self.mode = Some(if m == "Opaque" { Mode::Opaque } else { Mode::Transparent });
        }
    }

    fn read_box(&mut self, b: &Node) {
        let p = |names: &[&str]| attr(b, names).and_then(|v| v.trim().parse::<f32>().ok());
        self.padding[0] = p(&["padding"]).or(self.padding[0]);
        self.padding[1] = p(&["topPadding"]).or(self.padding[1]);
        self.padding[2] = p(&["leftPadding"]).or(self.padding[2]);
        self.padding[3] = p(&["bottomPadding"]).or(self.padding[3]);
        self.padding[4] = p(&["rightPadding"]).or(self.padding[4]);
        for (i, name) in ["pen", "topPen", "leftPen", "bottomPen", "rightPen"].iter().enumerate() {
            if let Some(pn) = b.child(name) {
                self.box_pens[i] = PenProps::read(pn).over(&self.box_pens[i]);
            }
        }
    }

    fn read_paragraph(&mut self, p: &Node) {
        if let Some(v) = p.attr("lineSpacing") {
            self.line_spacing = Some(v.to_string());
        }
        if let Some(v) = p.attr("lineSpacingSize").and_then(|v| v.trim().parse().ok()) {
            self.line_spacing_size = Some(v);
        }
    }

    /// Properties declared directly by a `<style>` element.
    fn from_style_node(n: &Node) -> StyleProps {
        let mut s = StyleProps::default();
        s.read_font_attrs(n);
        s.read_alignment(n);
        s.read_colors(n);
        if let Some(v) = n.attr("pattern") {
            s.pattern = Some(v.to_string());
        }
        if let Some(v) = attr(n, &["isBlankWhenNull", "blankWhenNull"]) {
            s.blank_when_null = Some(parse_bool(v));
        }
        if let Some(v) = n.attr("radius").and_then(|v| v.trim().parse().ok()) {
            s.radius = Some(v);
        }
        if let Some(pen) = n.child("pen") {
            s.graphic_pen = PenProps::read(pen);
        }
        if let Some(b) = n.child("box") {
            s.read_box(b);
        }
        if let Some(p) = n.child("paragraph") {
            s.read_paragraph(p);
        }
        s
    }

    fn line_spacing(&self) -> LineSpacing {
        let size = self.line_spacing_size.unwrap_or(1.0);
        match self.line_spacing.as_deref() {
            Some("1_1_2") => LineSpacing::OneAndHalf,
            Some("Double") => LineSpacing::Double,
            Some("Proportional") => LineSpacing::Proportional(size),
            Some("AtLeast") => LineSpacing::AtLeast(size),
            Some("Fixed") => LineSpacing::Fixed(size),
            _ => LineSpacing::Single,
        }
    }

    fn box_style(&self, forecolor: Color) -> BoxStyle {
        let all = self.padding[0].unwrap_or(0.0);
        let side = |i: usize| self.box_pens[i].over(&self.box_pens[0]).resolve(0.0, forecolor);
        BoxStyle {
            padding: Padding {
                top: self.padding[1].unwrap_or(all),
                left: self.padding[2].unwrap_or(all),
                bottom: self.padding[3].unwrap_or(all),
                right: self.padding[4].unwrap_or(all),
            },
            top: side(1),
            left: side(2),
            bottom: side(3),
            right: side(4),
        }
    }
}

// ---------------------------------------------------------------------------------------------

struct Parser {
    chart_count: usize,
    groovy: bool,
    styles: HashMap<String, StyleProps>,
    default_style: StyleProps,
    default_name: Option<String>,
    /// Conditional styles per style name: (condition, properties of the inner style).
    conditionals: HashMap<String, Vec<(Expr, StyleProps)>>,
    warnings: Vec<String>,
    /// `<subDataset>`s by name, parsed before the bands so tables can use them.
    datasets: HashMap<String, Dataset>,
    page_height: f32,
}

/// Most conditional styles one element may combine (variants are precomputed per subset).
const MAX_CONDITIONAL_STYLES: usize = 4;

impl Parser {
    fn new(root: &Node) -> Result<Parser> {
        let groovy = root.attr("language").map(|l| l.eq_ignore_ascii_case("groovy")).unwrap_or(false);
        // Resolve styles in declaration order; parents must be declared first in JRXML.
        let mut styles: HashMap<String, StyleProps> = HashMap::new();
        let mut default_style = StyleProps::default();
        let mut default_name = None;
        let mut warnings = Vec::new();
        let mut raw_conditionals: Vec<(String, String, StyleProps)> = Vec::new();
        for s in root.children_named("style") {
            let Some(name) = s.attr("name") else { continue };
            let mut props = StyleProps::from_style_node(s);
            if let Some(parent) = attr(s, &["style", "parentStyle"]) {
                if let Some(p) = styles.get(parent) {
                    props = props.over(p);
                }
            }
            for cs in s.children_named("conditionalStyle") {
                let cond = cs.child("conditionExpression").map(|c| c.text.clone()).unwrap_or_default();
                let inner = cs.child("style").map(StyleProps::from_style_node).unwrap_or_default();
                raw_conditionals.push((name.to_string(), cond, inner));
            }
            if attr(s, &["isDefault", "default"]).is_some_and(parse_bool) {
                default_style = props.clone();
                default_name = Some(name.to_string());
            }
            styles.insert(name.to_string(), props);
        }
        let mut parser = Parser {
            chart_count: 0,
            groovy,
            styles,
            default_style,
            default_name,
            conditionals: HashMap::new(),
            warnings: Vec::new(),
            datasets: HashMap::new(),
            page_height: attr(root, &["pageHeight"]).and_then(|v| v.trim().parse().ok()).unwrap_or(842.0),
        };
        for (name, cond, props) in raw_conditionals {
            if let Some(e) = parser.expr(&cond)? {
                parser.conditionals.entry(name).or_default().push((e, props));
            }
        }
        for (name, list) in &parser.conditionals {
            if list.len() > MAX_CONDITIONAL_STYLES {
                warnings.push(format!("style `{name}`: only the first {MAX_CONDITIONAL_STYLES} conditional styles are applied"));
            }
        }
        parser.warnings = warnings;
        Ok(parser)
    }

    fn expr(&self, text: &str) -> Result<Option<Expr>> {
        let t = text.trim();
        if t.is_empty() {
            return Ok(None);
        }
        parse_expr(t, self.groovy).map(Some).map_err(|message| Error::ExprParse { expr: t.to_string(), message })
    }

    fn child_expr(&self, n: &Node, names: &[&str]) -> Result<Option<Expr>> {
        match names.iter().find_map(|name| n.child(name)) {
            Some(c) => self.expr(&c.text),
            None => Ok(None),
        }
    }

    fn report(mut self, root: &Node) -> Result<Report> {
        let num = |names: &[&str], default: f32| -> Result<f32> {
            match attr(root, names) {
                Some(v) => parse_f32(v, names[0]),
                None => Ok(default),
            }
        };
        let page_width = num(&["pageWidth"], 595.0)?;
        let left = num(&["leftMargin"], 20.0)?;
        let right = num(&["rightMargin"], 20.0)?;
        // Subdatasets first: tables in the bands refer to them.
        for d in root.children_named("subDataset").chain(root.children_named("dataset")) {
            let dataset = self.dataset(d)?;
            if d.child("queryString").is_some() || d.child("query").is_some() {
                self.warnings.push(format!(
                    "subdataset `{}`: its query is ignored; tables get rows from their dataSourceExpression",
                    dataset.name
                ));
            }
            self.datasets.insert(dataset.name.clone(), dataset);
        }
        let Dataset { parameters, fields, variables, groups, .. } = self.dataset(root)?;
        for unsupported in ["scriptlet", "template"] {
            if root.child(unsupported).is_some() {
                self.warnings.push(format!("<{unsupported}> is not supported yet"));
            }
        }
        if root.child("queryString").is_some() || root.child("query").is_some() {
            self.warnings.push("embedded SQL query is ignored; data comes from the provided data source".into());
        }
        let band =
            |p: &mut Parser, name: &str| -> Result<Option<Band>> { Ok(p.section_bands(root.child(name))?.into_iter().next()) };
        let report = Report {
            name: root.attr("name").unwrap_or("report").to_string(),
            language: if self.groovy { Language::Groovy } else { Language::Java },
            page_width,
            page_height: num(&["pageHeight"], 842.0)?,
            landscape: attr(root, &["orientation"]) == Some("Landscape"),
            column_width: num(&["columnWidth"], page_width - left - right)?,
            column_count: num(&["columnCount"], 1.0)? as u32,
            column_spacing: num(&["columnSpacing"], 0.0)?,
            margins: Margins { left, right, top: num(&["topMargin"], 30.0)?, bottom: num(&["bottomMargin"], 30.0)? },
            when_no_data: match attr(root, &["whenNoDataType"]) {
                Some("BlankPage") => WhenNoData::BlankPage,
                Some("AllSectionsNoDetail") => WhenNoData::AllSectionsNoDetail,
                Some("NoDataSection") => WhenNoData::NoDataSection,
                _ => WhenNoData::NoPages,
            },
            title_new_page: attr(root, &["isTitleNewPage", "titleNewPage"]).is_some_and(parse_bool),
            summary_new_page: attr(root, &["isSummaryNewPage", "summaryNewPage"]).is_some_and(parse_bool),
            summary_with_page_header_and_footer: attr(
                root,
                &["isSummaryWithPageHeaderAndFooter", "summaryWithPageHeaderAndFooter"],
            )
            .is_some_and(parse_bool),
            float_column_footer: attr(root, &["isFloatColumnFooter", "floatColumnFooter"]).is_some_and(parse_bool),
            parameters,
            fields,
            variables,
            groups,
            background: band(&mut self, "background")?,
            title: band(&mut self, "title")?,
            page_header: band(&mut self, "pageHeader")?,
            column_header: band(&mut self, "columnHeader")?,
            detail: self.section_bands(root.child("detail"))?,
            column_footer: band(&mut self, "columnFooter")?,
            page_footer: band(&mut self, "pageFooter")?,
            last_page_footer: band(&mut self, "lastPageFooter")?,
            summary: band(&mut self, "summary")?,
            no_data: band(&mut self, "noData")?,
            datasets: Vec::new(),
            uses_data_source: false,
            warnings: Vec::new(),
        };
        if report.column_count > 1 && attr(root, &["printOrder"]) == Some("Horizontal") {
            self.warnings.push("printOrder=\"Horizontal\" is rendered as \"Vertical\"".into());
        }
        let mut report = report;
        let mut datasets: Vec<Dataset> = self.datasets.drain().map(|(_, d)| d).collect();
        datasets.sort_by(|a, b| a.name.cmp(&b.name));
        report.datasets = datasets;
        self.warnings.dedup();
        report.warnings = self.warnings;
        Ok(report)
    }

    /// Parameters, fields, variables and groups of the report or of a `<subDataset>`.
    fn dataset(&mut self, node: &Node) -> Result<Dataset> {
        let mut parameters = Vec::new();
        for p in node.children_named("parameter") {
            parameters.push(Parameter {
                name: p.attr("name").unwrap_or_default().to_string(),
                class: JavaType::from_class_name(p.attr("class").unwrap_or("java.lang.String")),
                default: self.child_expr(p, &["defaultValueExpression"])?,
            });
        }
        let fields = node
            .children_named("field")
            .map(|f| Field {
                name: f.attr("name").unwrap_or_default().to_string(),
                class: JavaType::from_class_name(f.attr("class").unwrap_or("java.lang.String")),
            })
            .collect();
        let mut variables = Vec::new();
        for v in node.children_named("variable") {
            variables.push(self.variable(v)?);
        }
        let mut groups = Vec::new();
        for g in node.children_named("group") {
            groups.push(Group {
                name: g.attr("name").unwrap_or_default().to_string(),
                expression: self.child_expr(g, &["groupExpression", "expression"])?,
                start_new_page: attr(g, &["isStartNewPage", "startNewPage"]).is_some_and(parse_bool),
                start_new_column: attr(g, &["isStartNewColumn", "startNewColumn"]).is_some_and(parse_bool),
                reprint_header_on_each_page: attr(g, &["isReprintHeaderOnEachPage", "reprintHeaderOnEachPage"])
                    .is_some_and(parse_bool),
                min_height_to_start_new_page: attr(g, &["minHeightToStartNewPage"]).and_then(|v| v.parse().ok()).unwrap_or(0.0),
                header: self.section_bands(g.child("groupHeader"))?,
                footer: self.section_bands(g.child("groupFooter"))?,
            });
        }
        Ok(Dataset { name: node.attr("name").unwrap_or_default().to_string(), parameters, fields, variables, groups })
    }

    /// A `<datasetRun>` (of a table or list).
    fn dataset_run(&mut self, run: Option<&Node>, what: &str) -> Result<DatasetRun> {
        let dataset = match run.and_then(|r| r.attr("subDataset")) {
            Some(name) => match self.datasets.get(name) {
                Some(d) => Some(d.clone()),
                None => {
                    self.warnings.push(format!("{what}: unknown subdataset `{name}`"));
                    None
                }
            },
            None => None,
        };
        let mut parameters = Vec::new();
        let (mut parameters_map, mut data_source) = (None, None);
        if let Some(r) = run {
            for c in r.children.iter().filter(|c| c.name == "datasetParameter" || c.name == "parameter") {
                if let Some(e) = self.child_expr(c, &["datasetParameterExpression", "expression"])? {
                    parameters.push((c.attr("name").unwrap_or_default().to_string(), e));
                }
            }
            if r.child("connectionExpression").is_some() && r.child("dataSourceExpression").is_none() {
                self.warnings.push(format!("{what}s with a connectionExpression get no rows (only data sources are supported)"));
            }
            parameters_map = self.child_expr(r, &["parametersMapExpression"])?;
            data_source = self.child_expr(r, &["dataSourceExpression"])?;
        }
        Ok(DatasetRun {
            dataset,
            parameters,
            parameters_map,
            data_source,
            page_height: self.page_height,
            language: if self.groovy { Language::Groovy } else { Language::Java },
        })
    }

    /// A crosstab (`<crosstab>`, or `<element kind="crosstab">` in JRXML 7).
    fn crosstab(&mut self, n: &Node) -> Result<CrosstabElement> {
        let dataset = n.child("crosstabDataset").and_then(|d| d.child("dataset")).or_else(|| n.child("dataset"));
        let run = match dataset {
            Some(d) => {
                if matches!(attr(d, &["resetType"]), Some(t) if t != "Report") {
                    self.warnings.push("crosstab datasets reset by group or page use all the rows printed so far".into());
                }
                match d.child("datasetRun") {
                    Some(r) => Some(self.dataset_run(Some(r), "crosstab")?),
                    None => None,
                }
            }
            None => None,
        };
        let mut parameters = Vec::new();
        for p in n.children.iter().filter(|c| c.name == "crosstabParameter" || c.name == "parameter") {
            if let Some(e) = self.child_expr(p, &["parameterValueExpression", "expression"])? {
                parameters.push((p.attr("name").unwrap_or_default().to_string(), e));
            }
        }
        if n.child("parametersMapExpression").is_some() {
            self.warnings.push("crosstab parametersMapExpression is ignored".into());
        }
        let mut row_groups = Vec::new();
        let mut column_groups = Vec::new();
        for g in &n.children {
            let row = g.name == "rowGroup";
            if !row && g.name != "columnGroup" {
                continue;
            }
            let size_attr = if row { "width" } else { "height" };
            let size = g.attr(size_attr).map(|v| parse_f32(v, size_attr)).transpose()?.unwrap_or(0.0);
            let total_position = match g.attr("totalPosition") {
                Some("Start") => TotalPosition::Start,
                Some("End") => TotalPosition::End,
                _ => TotalPosition::None,
            };
            let header_position = match g.attr("headerPosition") {
                Some("Middle" | "Center") => HeaderPosition::Middle,
                Some("Bottom" | "Right") => HeaderPosition::End,
                Some("Stretch") => HeaderPosition::Stretch,
                _ => HeaderPosition::Start,
            };
            let bucket = g.child("bucket");
            if bucket.is_some_and(|b| b.child("comparatorExpression").is_some()) {
                self.warnings.push("crosstab bucket comparators are ignored (values are sorted naturally)".into());
            }
            let header_node = |names: &[&str]| {
                g.children
                    .iter()
                    .find(|c| names.contains(&c.name.as_str()))
                    .and_then(|h| h.child("cellContents").or_else(|| h.child("contents")).or(Some(h)))
            };
            let (header_names, total_names): (&[&str], &[&str]) = if row {
                (&["crosstabRowHeader", "header"], &["crosstabTotalRowHeader", "totalHeader"])
            } else {
                (&["crosstabColumnHeader", "header"], &["crosstabTotalColumnHeader", "totalHeader"])
            };
            let header = match header_node(header_names) {
                Some(c) => self.cell_contents(c)?,
                None => None,
            };
            let total_header = match header_node(total_names) {
                Some(c) => self.cell_contents(c)?,
                None => None,
            };
            let group = CrosstabGroup {
                name: g.attr("name").unwrap_or_default().to_string(),
                size,
                total_position,
                header_position,
                bucket: match bucket {
                    Some(b) => self.child_expr(b, &["bucketExpression", "expression"])?,
                    None => None,
                },
                descending: bucket.and_then(|b| b.attr("order")) == Some("Descending"),
                order_by: match bucket {
                    Some(b) => self.child_expr(b, &["orderByExpression"])?,
                    None => None,
                },
                class: JavaType::from_class_name(bucket.and_then(|b| b.attr("class")).unwrap_or("java.lang.String")),
                header,
                total_header,
            };
            if row {
                row_groups.push(group);
            } else {
                column_groups.push(group);
            }
        }
        let mut measures = Vec::new();
        for m in n.children_named("measure") {
            let calculation = match m.attr("calculation") {
                Some("Count") => Calculation::Count,
                Some("DistinctCount") => Calculation::DistinctCount,
                Some("Sum") => Calculation::Sum,
                Some("Average") => Calculation::Average,
                Some("Lowest") => Calculation::Lowest,
                Some("Highest") => Calculation::Highest,
                Some("First") => Calculation::First,
                Some("StandardDeviation") => Calculation::StandardDeviation,
                Some("Variance") => Calculation::Variance,
                _ => Calculation::Nothing,
            };
            measures.push(CrosstabMeasure {
                name: m.attr("name").unwrap_or_default().to_string(),
                class: JavaType::from_class_name(m.attr("class").unwrap_or("java.lang.Object")),
                calculation,
                expr: self.child_expr(m, &["measureExpression", "expression"])?,
                percentage: m.attr("percentageOf") == Some("GrandTotal"),
            });
        }
        let mut cells = Vec::new();
        for c in n.children_named("crosstabCell").chain(n.children_named("cell")) {
            let num = |a: &str| c.attr(a).map(|v| parse_f32(v, a)).transpose();
            let contents = match c.child("cellContents").or_else(|| c.child("contents")) {
                Some(cc) => self.cell_contents(cc)?,
                None => None,
            };
            cells.push(CrosstabCell {
                row_total: c.attr("rowTotalGroup").map(str::to_string),
                column_total: c.attr("columnTotalGroup").map(str::to_string),
                width: num("width")?,
                height: num("height")?,
                contents,
            });
        }
        let special = |name: &str, alt: &str| {
            n.child(name).or_else(|| n.child(alt)).and_then(|h| h.child("cellContents").or_else(|| h.child("contents")))
        };
        let header_cell = match special("crosstabHeaderCell", "headerCell") {
            Some(c) => self.cell_contents(c)?,
            None => None,
        };
        let no_data_cell = match special("whenNoDataCell", "whenNoDataCell") {
            Some(c) => self.cell_contents(c)?,
            None => None,
        };
        let flag = |names: &[&str]| attr(n, names).map(parse_bool).unwrap_or(true);
        Ok(CrosstabElement {
            run,
            parameters,
            row_groups,
            column_groups,
            measures,
            cells,
            header_cell,
            no_data_cell,
            column_break_offset: attr(n, &["columnBreakOffset"]).and_then(|v| v.trim().parse().ok()).unwrap_or(10.0),
            repeat_column_headers: flag(&["isRepeatColumnHeaders", "repeatColumnHeaders"]),
            repeat_row_headers: flag(&["isRepeatRowHeaders", "repeatRowHeaders"]),
        })
    }

    /// The `<cellContents>` of a crosstab cell as a frame (sized when the cell is placed).
    fn cell_contents(&mut self, c: &Node) -> Result<Option<Element>> {
        let mut re_attrs = vec![
            ("x".to_string(), "0".to_string()),
            ("y".to_string(), "0".to_string()),
            ("width".to_string(), "0".to_string()),
            ("height".to_string(), "0".to_string()),
        ];
        for a in ["style", "mode", "backcolor"] {
            if let Some(v) = c.attr(a) {
                re_attrs.push((a.to_string(), v.to_string()));
            }
        }
        // Cell contents are transparent unless they say otherwise (or their style does).
        if c.attr("mode").is_none() && c.attr("style").is_none() {
            re_attrs.push(("mode".to_string(), "Transparent".to_string()));
        }
        let mut children =
            vec![Node { name: "reportElement".into(), attrs: re_attrs, children: Vec::new(), text: String::new() }];
        children.extend(c.children.iter().cloned());
        let frame_node = Node { name: "frame".into(), attrs: Vec::new(), children, text: String::new() };
        Ok(Some(self.element(&frame_node, "frame")?))
    }

    /// A list component (`jr:list`, or `<component kind="list">` in JRXML 7).
    fn list(&mut self, l: &Node) -> Result<ListElement> {
        let run = self.dataset_run(l.child("datasetRun"), "list")?;
        if attr(l, &["printOrder"]) == Some("Horizontal") {
            self.warnings.push("lists with printOrder=\"Horizontal\" are rendered vertically".into());
        }
        let contents = match l.child("listContents").or_else(|| l.child("contents")) {
            Some(c) => self.band(c)?,
            None => Band { height: 0.0, split_type: SplitType::Stretch, print_when: None, elements: Vec::new() },
        };
        Ok(ListElement { run, contents })
    }

    /// A table component (`jr:table`, or `<component kind="table">` in JRXML 7).
    fn table(&mut self, t: &Node, element_width: f32) -> Result<TableElement> {
        let run = self.dataset_run(t.child("datasetRun"), "table")?;
        let mut columns = Vec::new();
        for c in &t.children {
            if let Some(col) = self.table_column(c)? {
                columns.push(col);
            }
        }
        let no_data = match t.child("noData") {
            Some(n) => Some(self.table_cell(n, element_width)?),
            None => None,
        };
        Ok(TableElement {
            run,
            columns,
            when_no_data: match t.attr("whenNoDataType") {
                Some("AllSectionsNoDetail") => WhenNoData::AllSectionsNoDetail,
                Some("NoDataCell") => WhenNoData::NoDataSection,
                _ => WhenNoData::NoPages,
            },
            no_data,
        })
    }

    /// `<jr:column>` / `<jr:columnGroup>` (JRXML 6) or `<column kind="single|group">` (JRXML 7).
    fn table_column(&mut self, c: &Node) -> Result<Option<TableColumn>> {
        let is_group = match (c.name.as_str(), c.attr("kind")) {
            ("columnGroup", _) | ("column", Some("group")) => true,
            ("column", _) => false,
            _ => return Ok(None),
        };
        let width = c.attr("width").map(|w| parse_f32(w, "column width")).transpose()?.unwrap_or(0.0);
        let mut children = Vec::new();
        if is_group {
            for sub in &c.children {
                if let Some(col) = self.table_column(sub)? {
                    children.push(col);
                }
            }
        }
        let cell = |p: &mut Parser, name: &str| -> Result<Option<TableCell>> {
            match c.child(name) {
                Some(n) => p.table_cell(n, width).map(Some),
                None => Ok(None),
            }
        };
        let table_header = cell(self, "tableHeader")?;
        let table_footer = cell(self, "tableFooter")?;
        let column_header = cell(self, "columnHeader")?;
        let column_footer = cell(self, "columnFooter")?;
        let detail = cell(self, "detailCell")?;
        let group_cells = |p: &mut Parser, name: &str| -> Result<Vec<(String, TableCell)>> {
            let mut out = Vec::new();
            for g in c.children_named(name) {
                // JRXML 6 wraps the cell: <groupHeader groupName><cell>; JRXML 7 may not.
                let node = g.child("cell").unwrap_or(g);
                out.push((g.attr("groupName").unwrap_or_default().to_string(), p.table_cell(node, width)?));
            }
            Ok(out)
        };
        let group_headers = group_cells(self, "groupHeader")?;
        let group_footers = group_cells(self, "groupFooter")?;
        Ok(Some(TableColumn {
            width,
            print_when: self.child_expr(c, &["printWhenExpression"])?,
            children,
            is_group,
            table_header,
            table_footer,
            column_header,
            column_footer,
            detail,
            group_headers,
            group_footers,
        }))
    }

    /// A cell becomes a frame with the cell's style and box, holding the cell's elements, as in
    /// Jasper's `TableReport.createCell`. A cell holding only a text element that fills it,
    /// with no box and no background of its own, is that text element (`createCellElement`).
    fn table_cell(&mut self, c: &Node, width: f32) -> Result<TableCell> {
        let height = c.attr("height").map(|h| parse_f32(h, "cell height")).transpose()?.unwrap_or(0.0);
        let mut re_attrs = vec![
            ("x".to_string(), "0".to_string()),
            ("y".to_string(), "0".to_string()),
            ("width".to_string(), width.to_string()),
            ("height".to_string(), height.to_string()),
            ("positionType".to_string(), "Float".to_string()),
            ("stretchType".to_string(), "RelativeToTallestObject".to_string()),
        ];
        if let Some(style) = c.attr("style") {
            re_attrs.push(("style".to_string(), style.to_string()));
        }
        let mut children =
            vec![Node { name: "reportElement".into(), attrs: re_attrs, children: Vec::new(), text: String::new() }];
        children.extend(c.children.iter().filter(|k| k.name != "printWhenExpression").cloned());
        let frame_node = Node { name: "frame".into(), attrs: Vec::new(), children, text: String::new() };
        let frame = self.element(&frame_node, "frame")?;
        if let ElementKind::Frame { children, boxed } = &frame.kind {
            let no_box = [boxed.top, boxed.left, boxed.bottom, boxed.right].iter().all(|p| p.width <= 0.0)
                && boxed.padding == Padding::default();
            if let [only] = children.as_slice() {
                let fills = only.x == 0.0 && only.y == 0.0 && only.width == width && only.height == height;
                let cell_transparent = frame.mode == Mode::Transparent;
                if fills && no_box && cell_transparent && matches!(only.kind, ElementKind::Text(_)) {
                    let mut el = only.clone();
                    el.stretch_type = StretchType::RelativeToTallestObject;
                    return Ok(TableCell { height, element: el });
                }
            }
        }
        Ok(TableCell { height, element: frame })
    }

    fn variable(&self, v: &Node) -> Result<Variable> {
        let reset = |kind: Option<&str>, group: Option<&str>, default: ResetType| match kind {
            Some("None") => ResetType::None,
            Some("Report") => ResetType::Report,
            Some("Page") => ResetType::Page,
            Some("Column") => ResetType::Column,
            Some("Group") => ResetType::Group(group.unwrap_or_default().to_string()),
            _ => default,
        };
        Ok(Variable {
            name: v.attr("name").unwrap_or_default().to_string(),
            class: JavaType::from_class_name(v.attr("class").unwrap_or("java.lang.String")),
            calculation: calculation(v.attr("calculation")),
            reset: reset(v.attr("resetType"), v.attr("resetGroup"), ResetType::Report),
            increment: reset(v.attr("incrementType"), v.attr("incrementGroup"), ResetType::None),
            expression: self.child_expr(v, &["variableExpression", "expression"])?,
            initial: self.child_expr(v, &["initialValueExpression"])?,
        })
    }
}

/// The barcode4j/ZXing component inside a `componentElement` (JRXML 6) or a
/// `<element kind="component">` (JRXML 7), when this engine can draw it.
fn barcode_node(n: &Node) -> Option<&Node> {
    const SUPPORTED: &[&str] = &["QRCode", "Code128", "Code39", "Interleaved2Of5", "Codabar", "EAN13", "EAN8", "UPCA"];
    n.children.iter().find(|c| {
        SUPPORTED.contains(&c.name.as_str())
            || (c.name == "component" && c.attr("kind").is_some_and(|k| SUPPORTED.contains(&k.rsplit(':').next().unwrap_or(k))))
    })
}

/// The table component inside a `componentElement` (JRXML 6) or a `<element kind="component">`
/// (JRXML 7).
fn table_node(n: &Node) -> Option<&Node> {
    component_node(n, "table")
}

/// The component called `name` inside a component element (JRXML 6 `<jr:name>` or JRXML 7
/// `<component kind="name">`).
fn component_node<'a>(n: &'a Node, name: &str) -> Option<&'a Node> {
    n.children.iter().find(|c| c.name == name || (c.name == "component" && c.attr("kind") == Some(name)))
}

fn font_from(node: Option<&Node>, base: &FontSpec) -> FontSpec {
    let mut f = base.clone();
    if let Some(n) = node.and_then(|n| n.child("font")) {
        if let Some(v) = n.attr("fontName") {
            f.name = v.to_string();
        }
        if let Some(v) = attr(n, &["size", "fontSize"]).and_then(|v| v.trim().parse().ok()) {
            f.size = v;
        }
        f.bold = attr(n, &["isBold", "bold"]).map(parse_bool).unwrap_or(f.bold);
        f.italic = attr(n, &["isItalic", "italic"]).map(parse_bool).unwrap_or(f.italic);
    }
    f
}

fn calculation(name: Option<&str>) -> Calculation {
    match name.unwrap_or("Nothing") {
        "Count" => Calculation::Count,
        "DistinctCount" => Calculation::DistinctCount,
        "Sum" => Calculation::Sum,
        "Average" => Calculation::Average,
        "Lowest" => Calculation::Lowest,
        "Highest" => Calculation::Highest,
        "First" => Calculation::First,
        "StandardDeviation" => Calculation::StandardDeviation,
        "Variance" => Calculation::Variance,
        "System" => Calculation::System,
        _ => Calculation::Nothing,
    }
}

impl Parser {
    /// Bands of a section. JRXML 6 nests `<band>` elements; JRXML 7 may put `height` on the
    /// section itself.
    fn section_bands(&mut self, section: Option<&Node>) -> Result<Vec<Band>> {
        let Some(section) = section else { return Ok(Vec::new()) };
        let bands: Vec<&Node> = section.children_named("band").collect();
        if bands.is_empty() {
            if section.attr("height").is_some() {
                return Ok(vec![self.band(section)?]);
            }
            return Ok(Vec::new());
        }
        bands.into_iter().map(|b| self.band(b)).collect()
    }

    fn band(&mut self, b: &Node) -> Result<Band> {
        Ok(Band {
            height: b.attr("height").map(|h| parse_f32(h, "band height")).transpose()?.unwrap_or(0.0),
            split_type: match b.attr("splitType") {
                Some("Prevent") => SplitType::Prevent,
                Some("Immediate") => SplitType::Immediate,
                _ => SplitType::Stretch,
            },
            print_when: self.child_expr(b, &["printWhenExpression"])?,
            elements: self.elements(b)?,
        })
    }

    fn chart(&mut self, n: &Node, tag: &str, font: FontSpec, boxed: BoxStyle) -> Result<crate::chart::ChartElement> {
        use crate::chart::*;
        let kind = ChartKind::from_tag(tag).expect("checked by the caller");
        let chart = n.child("chart");
        let flag = |node: Option<&Node>, names: &[&str], default: bool| {
            node.and_then(|c| attr(c, names)).map(parse_bool).unwrap_or(default)
        };
        let color = |node: Option<&Node>, name: &str| node.and_then(|c| c.attr(name)).and_then(parse_color);
        let title = chart.and_then(|c| c.child("chartTitle"));
        let subtitle = chart.and_then(|c| c.child("chartSubtitle"));
        let legend = chart.and_then(|c| c.child("chartLegend"));
        let evaluation_time = match chart.and_then(|c| c.attr("evaluationTime")) {
            Some("Report") | Some("Auto") => EvaluationTime::Report,
            Some("Page") => EvaluationTime::Page,
            Some("Column") => EvaluationTime::Column,
            Some("Group") => EvaluationTime::Group,
            _ => EvaluationTime::Now,
        };
        if chart.and_then(|c| c.attr("customizerClass")).is_some() {
            self.warnings.push("chart customizers (customizerClass) are ignored".into());
        }

        // Dataset.
        let ds_node = n.children.iter().find(|c| c.name.ends_with("Dataset"));
        let ds = ds_node.and_then(|d| d.child("dataset"));
        let group_attr = |name: &str| ds.and_then(|d| d.attr(name)).unwrap_or_default().to_string();
        let reset = match ds.and_then(|d| d.attr("resetType")) {
            Some("Group") => ResetType::Group(group_attr("resetGroup")),
            Some("Page") => ResetType::Page,
            Some("None") => ResetType::None,
            _ => ResetType::Report,
        };
        let increment = match ds.and_then(|d| d.attr("incrementType")) {
            Some("Group") => Increment::Group(group_attr("incrementGroup")),
            Some("Page") => Increment::Page,
            Some("Report") => Increment::Report,
            _ => Increment::Record,
        };
        if ds.and_then(|d| d.child("datasetRun")).is_some() {
            self.warnings.push("chart datasets run on a subdataset (datasetRun) are empty".into());
        }
        let mut series = Vec::new();
        if let Some(d) = ds_node {
            let read = |p: &mut Parser, s: &Node| -> Result<SeriesDef> {
                Ok(SeriesDef {
                    series: p.child_expr(s, &["seriesExpression"])?,
                    key: p.child_expr(s, &["keyExpression"])?,
                    category: p.child_expr(s, &["categoryExpression"])?,
                    value: p.child_expr(s, &["valueExpression"])?,
                    x: p.child_expr(s, &["xValueExpression"])?,
                    y: p.child_expr(s, &["yValueExpression"])?,
                })
            };
            let nested: Vec<&Node> =
                d.children.iter().filter(|c| matches!(c.name.as_str(), "pieSeries" | "categorySeries" | "xySeries")).collect();
            if nested.is_empty() {
                series.push(read(self, d)?);
            } else {
                for s in nested {
                    series.push(read(self, s)?);
                }
            }
        }

        // Plot.
        let plot_node = n.children.iter().find(|c| c.name.ends_with("Plot"));
        let plot = plot_node.and_then(|p| p.child("plot"));
        let mut series_colors: Vec<(i32, Color)> = plot
            .map(|p| {
                p.children
                    .iter()
                    .filter(|c| c.name == "seriesColor")
                    .filter_map(|c| Some((c.attr("seriesOrder")?.parse().ok()?, parse_color(c.attr("color")?)?)))
                    .collect()
            })
            .unwrap_or_default();
        series_colors.sort_by_key(|(o, _)| *o);
        let value_format = plot_node
            .and_then(|p| p.child("valueAxisFormat").or_else(|| p.child("rangeAxisFormat")))
            .and_then(|f| f.child("axisFormat"))
            .and_then(|f| f.attr("tickLabelMask"))
            .map(str::to_string);
        let label = |p: &mut Parser, names: &[&str]| match plot_node {
            Some(pn) => p.child_expr(pn, names),
            None => Ok(None),
        };
        self.chart_count += 1;
        Ok(ChartElement {
            id: self.chart_count,
            kind,
            horizontal: plot.and_then(|p| p.attr("orientation")) == Some("Horizontal"),
            title: match title {
                Some(t) => self.child_expr(t, &["titleExpression"])?,
                None => None,
            },
            title_font: font_from(title, &font),
            title_color: color(title, "color").unwrap_or(Color::BLACK),
            subtitle: match subtitle {
                Some(t) => self.child_expr(t, &["subtitleExpression"])?,
                None => None,
            },
            subtitle_font: font_from(subtitle, &font),
            subtitle_color: color(subtitle, "color").unwrap_or(Color::BLACK),
            show_legend: flag(chart, &["isShowLegend", "showLegend"], true),
            legend_font: font_from(legend, &font),
            legend_color: color(legend, "textColor").unwrap_or(Color::BLACK),
            legend_back: color(legend, "backgroundColor"),
            legend_position: match legend.and_then(|l| l.attr("position")) {
                Some("Top") => Edge::Top,
                Some("Left") => Edge::Left,
                Some("Right") => Edge::Right,
                _ => Edge::Bottom,
            },
            font,
            evaluation_time,
            evaluation_group: chart.and_then(|c| c.attr("evaluationGroup")).map(str::to_string),
            dataset: DatasetDef {
                reset,
                increment,
                increment_when: match ds {
                    Some(d) => self.child_expr(d, &["incrementWhenExpression"])?,
                    None => None,
                },
                series,
            },
            category_axis_label: label(
                self,
                &["categoryAxisLabelExpression", "xAxisLabelExpression", "timeAxisLabelExpression"],
            )?,
            value_axis_label: label(self, &["valueAxisLabelExpression", "yAxisLabelExpression"])?,
            value_min: label(self, &["rangeAxisMinValueExpression"])?,
            value_max: label(self, &["rangeAxisMaxValueExpression"])?,
            series_colors: series_colors.into_iter().map(|(_, c)| c).collect(),
            show_labels: flag(plot_node, &["isShowLabels", "showLabels"], true),
            show_lines: flag(plot_node, &["isShowLines", "showLines"], true),
            show_shapes: flag(plot_node, &["isShowShapes", "showShapes"], true),
            backcolor: color(chart.and_then(|c| c.child("reportElement")), "backcolor")
                .filter(|_| chart.and_then(|c| c.child("reportElement")).and_then(|r| r.attr("mode")) == Some("Opaque")),
            plot_backcolor: color(plot, "backcolor"),
            value_format,
            boxed,
        })
    }

    fn elements(&mut self, container: &Node) -> Result<Vec<Element>> {
        const ELEMENT_TAGS: &[&str] = &[
            "staticText",
            "textField",
            "line",
            "rectangle",
            "ellipse",
            "frame",
            "image",
            "subreport",
            "break",
            "crosstab",
            "componentElement",
            "genericElement",
        ];
        let mut out = Vec::new();
        for c in &container.children {
            let kind = if c.name == "element" {
                c.attr("kind").unwrap_or("unknown")
            } else if ELEMENT_TAGS.contains(&c.name.as_str()) || c.name.ends_with("Chart") || c.name.ends_with("Chart3D") {
                c.name.as_str()
            } else if c.name == "elementGroup" {
                // Element groups only affect stretch dependencies; flatten them.
                out.extend(self.elements(c)?);
                continue;
            } else {
                continue;
            };
            out.push(self.element(c, kind)?);
        }
        Ok(out)
    }

    fn element(&mut self, n: &Node, kind: &str) -> Result<Element> {
        // JRXML 6 keeps geometry in <reportElement>; JRXML 7 puts it on <element>.
        // Charts keep their geometry in <chart><reportElement>.
        let re = n.child("reportElement").or_else(|| n.child("chart").and_then(|c| c.child("reportElement"))).unwrap_or(n);
        let num = |name: &str| -> Result<f32> { re.attr(name).map(|v| parse_f32(v, name)).transpose().map(|v| v.unwrap_or(0.0)) };

        // Style chain: named style, else the report default style.
        let base = match re.attr("style") {
            Some(name) => match self.styles.get(name) {
                Some(s) => s.clone(),
                None => {
                    self.warnings.push(format!("unknown style `{name}`"));
                    StyleProps::default()
                }
            },
            None => self.default_style.clone(),
        };
        let mut own = StyleProps::default();
        own.read_colors(re);
        if let Some(te) = n.child("textElement") {
            own.read_alignment(te);
            if let Some(f) = te.child("font") {
                own.read_font_attrs(f);
            }
            if let Some(p) = te.child("paragraph") {
                own.read_paragraph(p);
            }
        }
        if n.name == "element" {
            own.read_font_attrs(n);
            own.read_alignment(n);
            if let Some(p) = n.child("paragraph") {
                own.read_paragraph(p);
            }
        }
        if let Some(b) = n.child("box").or_else(|| n.child("chart").and_then(|c| c.child("box"))) {
            own.read_box(b);
        }
        if let Some(v) = n.attr("pattern") {
            own.pattern = Some(v.to_string());
        }
        if let Some(v) = attr(n, &["isBlankWhenNull", "blankWhenNull"]) {
            own.blank_when_null = Some(parse_bool(v));
        }
        if let Some(v) = n.attr("radius").and_then(|v| v.trim().parse().ok()) {
            own.radius = Some(v);
        }
        if let Some(pen) = n.child("graphicElement").and_then(|g| g.child("pen")).or_else(|| n.child("pen")) {
            own.graphic_pen = PenProps::read(pen);
        }
        let parts = self.styled_parts(n, kind, &own.over(&base))?;

        // Conditional styles of the element's style (or of the default style): one variant per
        // combination of conditions, chosen at fill time.
        let style_name = re.attr("style").map(str::to_string).or_else(|| self.default_name.clone());
        let conds: Vec<(Expr, StyleProps)> = style_name
            .and_then(|name| self.conditionals.get(&name).cloned())
            .unwrap_or_default()
            .into_iter()
            .take(MAX_CONDITIONAL_STYLES)
            .collect();
        let conditional = if conds.is_empty() {
            None
        } else {
            let mut variants = Vec::with_capacity(1 << conds.len());
            for mask in 0..(1usize << conds.len()) {
                let mut props = base.clone();
                for (i, (_, cprops)) in conds.iter().enumerate() {
                    if mask & (1 << i) != 0 {
                        props = cprops.over(&props);
                    }
                }
                variants.push(self.styled_parts(n, kind, &own.over(&props))?);
            }
            Some(Box::new(ConditionalStyles { conditions: conds.into_iter().map(|(e, _)| e).collect(), variants }))
        };

        Ok(Element {
            key: re.attr("key").map(str::to_string),
            x: num("x")?,
            y: num("y")?,
            width: num("width")?,
            height: num("height")?,
            position_type: match re.attr("positionType") {
                Some("Float") => PositionType::Float,
                Some("FixRelativeToBottom") => PositionType::FixRelativeToBottom,
                Some("FixRelativeToTop") => PositionType::FixRelativeToTop,
                _ => PositionType::Fix,
            },
            stretch_type: match re.attr("stretchType") {
                Some("RelativeToBandHeight") => StretchType::RelativeToBandHeight,
                Some("RelativeToTallestObject") => StretchType::RelativeToTallestObject,
                Some("ContainerHeight") => StretchType::ContainerHeight,
                Some("ContainerBottom") => StretchType::ContainerBottom,
                _ => StretchType::NoStretch,
            },
            mode: parts.mode,
            forecolor: parts.forecolor,
            backcolor: parts.backcolor,
            print_when: self.child_expr(re, &["printWhenExpression"])?,
            remove_line_when_blank: attr(re, &["isRemoveLineWhenBlank", "removeLineWhenBlank"]).is_some_and(parse_bool),
            print_when_detail_overflows: attr(re, &["isPrintWhenDetailOverflows", "printWhenDetailOverflows"])
                .is_some_and(parse_bool),
            print_repeated_values: !attr(re, &["isPrintRepeatedValues", "printRepeatedValues"]).is_some_and(|v| !parse_bool(v)),
            print_in_first_whole_band: attr(re, &["isPrintInFirstWholeBand", "printInFirstWholeBand"]).is_some_and(parse_bool),
            print_when_group_changes: re.attr("printWhenGroupChanges").map(str::to_string),
            conditional,
            kind: parts.kind,
        })
    }

    /// Builds the style-dependent parts of an element from its resolved properties.
    fn styled_parts(&mut self, n: &Node, kind: &str, props: &StyleProps) -> Result<StyledParts> {
        let forecolor = props.forecolor.unwrap_or(Color::BLACK);
        let is_graphic = matches!(kind, "rectangle" | "ellipse");
        // Images are transparent unless a mode is given (Jasper 6 default).
        let mode = props.mode.unwrap_or(if is_graphic { Mode::Opaque } else { Mode::Transparent });

        let element_kind = match kind {
            "staticText" | "textField" => {
                let font = FontSpec {
                    name: props.font_name.clone().unwrap_or_else(|| "SansSerif".into()),
                    size: props.font_size.unwrap_or(10.0),
                    bold: props.bold.unwrap_or(false),
                    italic: props.italic.unwrap_or(false),
                    underline: props.underline.unwrap_or(false),
                    strike: props.strike.unwrap_or(false),
                };
                let rotation = match n.child("textElement").and_then(|t| t.attr("rotation")).or_else(|| n.attr("rotation")) {
                    Some("Left") => Rotation::Left,
                    Some("Right") => Rotation::Right,
                    Some("UpsideDown") => Rotation::UpsideDown,
                    _ => Rotation::None,
                };
                let style = TextStyle {
                    font,
                    halign: props.halign.unwrap_or(HAlign::Left),
                    valign: props.valign.unwrap_or(VAlign::Top),
                    line_spacing: props.line_spacing(),
                    rotation,
                };
                let te_node = n.child("textElement");
                let markup = match attr(n, &["markup"]).or_else(|| te_node.and_then(|t| t.attr("markup"))) {
                    Some("styled") => crate::rich::Markup::Styled,
                    Some("html") => crate::rich::Markup::Html,
                    Some(m) if m != "none" && !m.is_empty() => {
                        self.warnings.push(format!("markup=\"{m}\" is rendered as plain text"));
                        crate::rich::Markup::None
                    }
                    _ if te_node.and_then(|t| t.attr("isStyledText")).is_some_and(parse_bool) => crate::rich::Markup::Styled,
                    _ => crate::rich::Markup::None,
                };
                let stretch = attr(n, &["isStretchWithOverflow"]).is_some_and(parse_bool)
                    || attr(n, &["textAdjust"]).is_some_and(|v| v == "StretchHeight");
                let content = if kind == "staticText" {
                    TextContent::Static(n.child("text").map(|t| t.text.clone()).unwrap_or_default())
                } else {
                    let evaluation_time = match n.attr("evaluationTime").unwrap_or("Now") {
                        "Report" => EvaluationTime::Report,
                        "Page" => EvaluationTime::Page,
                        "Column" => EvaluationTime::Column,
                        "Group" => EvaluationTime::Group,
                        "Band" => EvaluationTime::Band,
                        "Auto" => EvaluationTime::Auto,
                        _ => EvaluationTime::Now,
                    };
                    if matches!(evaluation_time, EvaluationTime::Auto) {
                        self.warnings.push("evaluationTime=\"Auto\" is treated as \"Report\"".into());
                    }
                    TextContent::Expression {
                        expr: self.child_expr(n, &["textFieldExpression", "expression"])?,
                        pattern: props.pattern.clone(),
                        pattern_expr: self.child_expr(n, &["patternExpression"])?,
                        blank_when_null: props.blank_when_null.unwrap_or(false),
                        evaluation_time,
                        evaluation_group: n.attr("evaluationGroup").map(str::to_string),
                    }
                };
                ElementKind::Text(TextElement { content, style, boxed: props.box_style(forecolor), stretch, markup })
            }
            "line" => ElementKind::Line {
                direction: if n.attr("direction") == Some("BottomUp") { LineDirection::BottomUp } else { LineDirection::TopDown },
                pen: props.graphic_pen.resolve(1.0, forecolor),
            },
            "rectangle" => {
                ElementKind::Rectangle { radius: props.radius.unwrap_or(0.0), pen: props.graphic_pen.resolve(1.0, forecolor) }
            }
            "ellipse" => ElementKind::Ellipse { pen: props.graphic_pen.resolve(1.0, forecolor) },
            "frame" => ElementKind::Frame { children: self.elements(n)?, boxed: props.box_style(forecolor) },
            "image" => {
                let scale = match attr(n, &["scaleImage"]) {
                    Some("Clip") => ScaleImage::Clip,
                    Some("FillFrame") => ScaleImage::FillFrame,
                    Some("RetainShape") | None => ScaleImage::RetainShape,
                    Some(other) => {
                        self.warnings.push(format!("scaleImage=\"{other}\" is rendered as RetainShape"));
                        ScaleImage::RetainShape
                    }
                };
                let halign = match attr(n, &["hImageAlign", "hAlign", "horizontalImageAlign", "horizontalAlignment"]) {
                    Some("Center") => HAlign::Center,
                    Some("Right") => HAlign::Right,
                    _ => HAlign::Left,
                };
                let valign = match attr(n, &["vImageAlign", "vAlign", "verticalImageAlign", "verticalAlignment"]) {
                    Some("Middle") => VAlign::Middle,
                    Some("Bottom") => VAlign::Bottom,
                    _ => VAlign::Top,
                };
                let on_error = match attr(n, &["onErrorType"]) {
                    Some("Blank") => OnErrorType::Blank,
                    Some("Icon") => OnErrorType::Icon,
                    _ => OnErrorType::Error,
                };
                ElementKind::Image(ImageElement {
                    expr: self.child_expr(n, &["imageExpression", "expression"])?,
                    scale,
                    halign,
                    valign,
                    boxed: props.box_style(forecolor),
                    on_error,
                })
            }
            "subreport" => {
                // JRXML 6: <subreportParameter name><subreportParameterExpression>;
                // JRXML 7: <parameter name><expression>.
                let mut parameters = Vec::new();
                for c in n.children.iter().filter(|c| c.name == "subreportParameter" || c.name == "parameter") {
                    let name = c.attr("name").unwrap_or_default().to_string();
                    if let Some(e) = self.child_expr(c, &["subreportParameterExpression", "expression"])? {
                        parameters.push((name, e));
                    }
                }
                let returns = n
                    .children
                    .iter()
                    .filter(|c| c.name == "returnValue")
                    .filter_map(|c| {
                        Some(ReturnValue {
                            from: c.attr("subreportVariable").or(c.attr("fromVariable"))?.to_string(),
                            to: c.attr("toVariable")?.to_string(),
                            calculation: calculation(c.attr("calculation")),
                        })
                    })
                    .collect();
                let connection = n.child("connectionExpression").is_some();
                if connection {
                    self.warnings
                        .push("subreports with a connectionExpression get no rows (only data sources are supported)".into());
                }
                ElementKind::Subreport(Box::new(SubreportElement {
                    expr: self.child_expr(n, &["subreportExpression", "expression"])?,
                    parameters,
                    parameters_map: self.child_expr(n, &["parametersMapExpression"])?,
                    data_source: self.child_expr(n, &["dataSourceExpression"])?,
                    connection,
                    returns,
                }))
            }
            tag if crate::chart::ChartKind::from_tag(tag).is_some() => {
                let base_font = FontSpec {
                    name: props.font_name.clone().unwrap_or_else(|| "SansSerif".into()),
                    size: props.font_size.unwrap_or(10.0),
                    bold: false,
                    italic: false,
                    underline: false,
                    strike: false,
                };
                ElementKind::Chart(Box::new(self.chart(n, tag, base_font, props.box_style(forecolor))?))
            }
            "componentElement" | "component" if component_node(n, "list").is_some() => {
                let l = component_node(n, "list").unwrap_or(n);
                ElementKind::List(Box::new(self.list(l)?))
            }
            "componentElement" | "component" if table_node(n).is_some() => {
                let t = table_node(n).unwrap_or(n);
                let re = n.child("reportElement").unwrap_or(n);
                let width = re.attr("width").and_then(|w| w.trim().parse().ok()).unwrap_or(0.0);
                ElementKind::Table(Box::new(self.table(t, width)?))
            }
            "componentElement" | "component" if barcode_node(n).is_some() => {
                let c = barcode_node(n).unwrap_or(n);
                let name = c.attr("kind").map(|k| k.rsplit(':').next().unwrap_or(k)).filter(|_| c.name == "component");
                let name = name.unwrap_or(c.name.as_str());
                let num = |a: &str| c.attr(a).and_then(|v| v.trim().parse::<f64>().ok());
                let symbology = if name == "QRCode" {
                    let level = c.attr("errorCorrectionLevel").and_then(crate::barcode::qr::Ecl::from_name);
                    Symbology::Qr {
                        level: level.unwrap_or(crate::barcode::qr::Ecl::L),
                        margin: c.attr("margin").and_then(|v| v.trim().parse().ok()),
                        version: c.attr("qrVersion").and_then(|v| v.trim().parse().ok()),
                    }
                } else {
                    if c.attr("orientation").is_some_and(|o| !matches!(o, "0" | "up")) {
                        self.warnings.push("rotated barcodes are drawn unrotated".into());
                    }
                    let text_position = match c.attr("textPosition") {
                        Some("none") => crate::barcode::TextPosition::None,
                        Some("top") => crate::barcode::TextPosition::Top,
                        _ => crate::barcode::TextPosition::Bottom,
                    };
                    match crate::barcode::linear::Kind::from_component(name) {
                        Some(kind) => {
                            use crate::barcode::linear::{Checksum, Spec};
                            let flag = |a: &str| c.attr(a).map(parse_bool).unwrap_or(false);
                            let checksum = match c.attr("checksumMode") {
                                Some(m) => Checksum::from_name(m).unwrap_or_else(|| {
                                    self.warnings.push(format!("unknown barcode checksumMode `{m}`, using auto"));
                                    Checksum::Auto
                                }),
                                None => Checksum::Auto,
                            };
                            let display_start_stop = flag("displayStartStop");
                            Symbology::Linear(Spec {
                                module_width: num("moduleWidth"),
                                text_position,
                                quiet_zone: num("quietZone"),
                                vertical_quiet_zone: num("verticalQuietZone"),
                                checksum,
                                wide_factor: num("wideFactor"),
                                // barcode4j shows the check character whenever it shows start/stop.
                                display_checksum: flag("displayChecksum") || display_start_stop,
                                display_start_stop,
                                extended: flag("extendedCharSetEnabled"),
                                intercharacter_gap: num("intercharGapWidth"),
                                ..Spec::new(kind)
                            })
                        }
                        None => Symbology::Code128 {
                            module_width: num("moduleWidth"),
                            text_position,
                            quiet_zone: num("quietZone"),
                            vertical_quiet_zone: num("verticalQuietZone"),
                        },
                    }
                };
                ElementKind::Barcode(Box::new(BarcodeElement {
                    symbology,
                    expr: self.child_expr(c, &["codeExpression", "expression"])?,
                    boxed: props.box_style(forecolor),
                    font_name: props.font_name.clone(),
                    font_size: props.font_size,
                }))
            }
            "crosstab" => ElementKind::Crosstab(Box::new(self.crosstab(n)?)),
            "break" => {
                self.warnings.push("page breaks (<break>) are applied after the band that contains them".into());
                if attr(n, &["type"]) == Some("Column") {
                    ElementKind::ColumnBreak
                } else {
                    ElementKind::Break
                }
            }
            other => {
                let label = if other == "componentElement" || other == "component" {
                    n.children
                        .iter()
                        .find(|c| c.name != "reportElement")
                        .map(|c| c.name.clone())
                        .unwrap_or_else(|| other.to_string())
                } else {
                    other.to_string()
                };
                self.warnings.push(format!("<{label}> elements are not supported yet and are skipped"));
                ElementKind::Unsupported(label)
            }
        };

        Ok(StyledParts { mode, forecolor, backcolor: props.backcolor.unwrap_or(Color::WHITE), kind: element_kind })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const V6: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<jasperReport xmlns="http://jasperreports.sourceforge.net/jasperreports" name="t" pageWidth="595" pageHeight="842" columnWidth="555" leftMargin="20" rightMargin="20" topMargin="20" bottomMargin="20">
  <style name="Base" isDefault="true" fontName="DejaVu Sans" fontSize="9"/>
  <style name="Head" style="Base" isBold="true" forecolor="#336699"/>
  <field name="name" class="java.lang.String"/>
  <variable name="total" class="java.lang.Double" calculation="Sum"><variableExpression><![CDATA[$F{amount}]]></variableExpression></variable>
  <title><band height="30">
    <staticText><reportElement style="Head" x="0" y="0" width="200" height="20"/><textElement textAlignment="Center"/><text><![CDATA[Hi]]></text></staticText>
    <textField isStretchWithOverflow="true" pattern="#,##0.00"><reportElement x="0" y="0" width="100" height="10"/><textFieldExpression><![CDATA[$V{total}]]></textFieldExpression></textField>
    <genericElement><reportElement x="0" y="0" width="10" height="10"/><genericElementType namespace="x" name="y"/></genericElement>
  </band></title>
</jasperReport>"##;

    #[test]
    fn parses_v6_with_style_inheritance() {
        let r = parse_report(V6).unwrap();
        let band = r.title.as_ref().unwrap();
        let ElementKind::Text(t) = &band.elements[0].kind else { panic!() };
        assert_eq!(t.style.font.name, "DejaVu Sans");
        assert_eq!(t.style.font.size, 9.0);
        assert!(t.style.font.bold);
        assert_eq!(t.style.halign, HAlign::Center);
        assert_eq!(band.elements[0].forecolor, Color { r: 0x33, g: 0x66, b: 0x99 });
        let ElementKind::Text(tf) = &band.elements[1].kind else { panic!() };
        assert!(tf.stretch);
        assert!(matches!(&tf.content, TextContent::Expression { pattern: Some(p), .. } if p == "#,##0.00"));
        assert!(matches!(band.elements[2].kind, ElementKind::Unsupported(_)));
        assert!(r.warnings.iter().any(|w| w.contains("genericElement")));
    }

    const V7: &str = r##"<jasperReport name="t7" language="java" pageWidth="595" pageHeight="842" columnWidth="555" leftMargin="20" rightMargin="20" topMargin="20" bottomMargin="20">
  <style name="Base" default="true" fontName="DejaVu Sans" fontSize="11.0"/>
  <field name="name" class="java.lang.String"/>
  <title height="40">
    <element kind="textField" x="0" y="0" width="300" height="20" bold="true" hTextAlign="Right" textAdjust="StretchHeight">
      <expression><![CDATA["Hello " + $F{name}]]></expression>
    </element>
  </title>
  <detail><band height="20"><element kind="staticText" x="0" y="0" width="100" height="20"><text><![CDATA[x]]></text></element></band></detail>
</jasperReport>"##;

    #[test]
    fn parses_v7_layout() {
        let r = parse_report(V7).unwrap();
        let title = r.title.as_ref().unwrap();
        assert_eq!(title.height, 40.0);
        let ElementKind::Text(t) = &title.elements[0].kind else { panic!() };
        assert!(t.style.font.bold && t.stretch);
        assert_eq!(t.style.font.size, 11.0);
        assert_eq!(t.style.halign, HAlign::Right);
        assert_eq!(r.detail.len(), 1);
        assert_eq!(title.elements[0].width, 300.0);
    }
}
