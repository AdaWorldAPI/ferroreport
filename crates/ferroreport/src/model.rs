//! In-memory model of a report template, with styles already resolved onto each element.
use std::sync::Arc;

use crate::expr::Expr;
use crate::value::JavaType;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub const BLACK: Color = Color { r: 0, g: 0, b: 0 };
    pub const WHITE: Color = Color { r: 255, g: 255, b: 255 };
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Language {
    Java,
    Groovy,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WhenNoData {
    NoPages,
    BlankPage,
    AllSectionsNoDetail,
    NoDataSection,
}

#[derive(Debug, Clone)]
pub struct Parameter {
    pub name: String,
    pub class: JavaType,
    pub default: Option<Expr>,
}

#[derive(Debug, Clone)]
pub struct Field {
    pub name: String,
    pub class: JavaType,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Calculation {
    Nothing,
    /// The value is only set from outside (a subreport return value or a scriptlet).
    System,
    Count,
    DistinctCount,
    Sum,
    Average,
    Lowest,
    Highest,
    First,
    StandardDeviation,
    Variance,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ResetType {
    None,
    Report,
    Page,
    Column,
    Group(String),
}

#[derive(Debug, Clone)]
pub struct Variable {
    pub name: String,
    pub class: JavaType,
    pub calculation: Calculation,
    pub reset: ResetType,
    pub increment: ResetType,
    pub expression: Option<Expr>,
    pub initial: Option<Expr>,
}

#[derive(Debug, Clone)]
pub struct Group {
    pub name: String,
    pub expression: Option<Expr>,
    pub start_new_page: bool,
    pub start_new_column: bool,
    pub reprint_header_on_each_page: bool,
    pub min_height_to_start_new_page: f32,
    pub header: Vec<Band>,
    pub footer: Vec<Band>,
}

/// How a band may break across pages.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SplitType {
    /// Never split within the declared height, but split when content stretches (default).
    Stretch,
    /// Move the whole band to the next page instead of splitting it.
    Prevent,
    /// Split anywhere.
    Immediate,
}

#[derive(Debug, Clone)]
pub struct Band {
    pub height: f32,
    pub split_type: SplitType,
    pub print_when: Option<Expr>,
    pub elements: Vec<Element>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PositionType {
    Fix,
    Float,
    FixRelativeToTop,
    FixRelativeToBottom,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StretchType {
    NoStretch,
    RelativeToBandHeight,
    RelativeToTallestObject,
    ContainerHeight,
    ContainerBottom,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mode {
    Opaque,
    Transparent,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LineStyle {
    Solid,
    Dashed,
    Dotted,
    Double,
}

/// A stroke. `width == 0` means no stroke.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pen {
    pub width: f32,
    pub color: Color,
    pub style: LineStyle,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Padding {
    pub top: f32,
    pub left: f32,
    pub bottom: f32,
    pub right: f32,
}

/// Borders and padding of a text element or frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoxStyle {
    pub padding: Padding,
    pub top: Pen,
    pub left: Pen,
    pub bottom: Pen,
    pub right: Pen,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HAlign {
    Left,
    Center,
    Right,
    Justified,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VAlign {
    Top,
    Middle,
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LineSpacing {
    Single,
    OneAndHalf,
    Double,
    Proportional(f32),
    AtLeast(f32),
    Fixed(f32),
}

#[derive(Debug, Clone, PartialEq)]
pub struct FontSpec {
    pub name: String,
    pub size: f32,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextStyle {
    pub font: FontSpec,
    pub halign: HAlign,
    pub valign: VAlign,
    pub line_spacing: LineSpacing,
    pub rotation: Rotation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rotation {
    None,
    /// 90° counter-clockwise: reads bottom to top.
    Left,
    /// 90° clockwise: reads top to bottom.
    Right,
    UpsideDown,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EvaluationTime {
    Now,
    Report,
    Page,
    Column,
    Group,
    Band,
    Auto,
}

#[derive(Debug, Clone)]
pub enum TextContent {
    Static(String),
    Expression {
        expr: Option<Expr>,
        pattern: Option<String>,
        pattern_expr: Option<Expr>,
        blank_when_null: bool,
        evaluation_time: EvaluationTime,
        evaluation_group: Option<String>,
    },
}

#[derive(Debug, Clone)]
pub struct TextElement {
    pub content: TextContent,
    pub style: TextStyle,
    pub boxed: BoxStyle,
    pub stretch: bool,
    pub markup: crate::rich::Markup,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LineDirection {
    TopDown,
    BottomUp,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ScaleImage {
    Clip,
    FillFrame,
    RetainShape,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OnErrorType {
    Error,
    Blank,
    Icon,
}

#[derive(Debug, Clone)]
pub struct ImageElement {
    pub expr: Option<Expr>,
    pub scale: ScaleImage,
    pub halign: HAlign,
    pub valign: VAlign,
    pub boxed: BoxStyle,
    pub on_error: OnErrorType,
}

#[derive(Debug, Clone)]
pub enum ElementKind {
    Text(TextElement),
    Line {
        direction: LineDirection,
        pen: Pen,
    },
    Rectangle {
        radius: f32,
        pen: Pen,
    },
    Ellipse {
        pen: Pen,
    },
    Frame {
        children: Vec<Element>,
        boxed: BoxStyle,
    },
    Image(ImageElement),
    Subreport(Box<SubreportElement>),
    /// A table component (`jr:table`), filled like a subreport over a subdataset.
    Table(Box<TableElement>),
    /// A list component (`jr:list`), filled like a subreport over a subdataset.
    List(Box<ListElement>),
    Barcode(Box<BarcodeElement>),
    Chart(Box<crate::chart::ChartElement>),
    /// Already filled subreport content still to be printed (the continuation of a subreport
    /// split across pages).
    Content(Arc<Vec<crate::print::Segment>>),
    /// A page break (`<break>`).
    Break,
    /// A column break (`<break type="Column">`).
    ColumnBreak,
    /// An element this engine cannot render yet; it is skipped and reported by `doctor`.
    Unsupported(String),
}

#[derive(Debug, Clone)]
pub struct Element {
    pub key: Option<String>,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub position_type: PositionType,
    pub stretch_type: StretchType,
    pub mode: Mode,
    pub forecolor: Color,
    pub backcolor: Color,
    pub print_when: Option<Expr>,
    pub remove_line_when_blank: bool,
    /// Printed again on the continuation of a band split across pages.
    pub print_when_detail_overflows: bool,
    /// `false`: printed only when its value differs from the last time it was printed.
    pub print_repeated_values: bool,
    /// With repeated values suppressed, print anyway in the first band of each page.
    pub print_in_first_whole_band: bool,
    /// With repeated values suppressed, print anyway when this group changes.
    pub print_when_group_changes: Option<String>,
    /// Style variants selected by conditional-style expressions.
    pub conditional: Option<Box<ConditionalStyles>>,
    pub kind: ElementKind,
}

/// The style-dependent parts of an element.
#[derive(Debug, Clone)]
pub struct StyledParts {
    pub mode: Mode,
    pub forecolor: Color,
    pub backcolor: Color,
    pub kind: ElementKind,
}

/// Conditional styles of an element: `variants[mask]` is the element styled with the
/// conditional styles whose bits are set in `mask` (bit i = `conditions[i]` is true).
#[derive(Debug, Clone)]
pub struct ConditionalStyles {
    pub conditions: Vec<Expr>,
    pub variants: Vec<StyledParts>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Margins {
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
}

/// A parsed, style-resolved report template.
#[derive(Debug, Clone)]
pub struct Report {
    pub name: String,
    pub language: Language,
    pub page_width: f32,
    pub page_height: f32,
    pub column_width: f32,
    pub column_count: u32,
    pub column_spacing: f32,
    pub margins: Margins,
    pub when_no_data: WhenNoData,
    pub title_new_page: bool,
    pub summary_new_page: bool,
    pub summary_with_page_header_and_footer: bool,
    pub float_column_footer: bool,
    pub parameters: Vec<Parameter>,
    pub fields: Vec<Field>,
    pub variables: Vec<Variable>,
    pub groups: Vec<Group>,
    pub background: Option<Band>,
    pub title: Option<Band>,
    pub page_header: Option<Band>,
    pub column_header: Option<Band>,
    pub detail: Vec<Band>,
    pub column_footer: Option<Band>,
    pub page_footer: Option<Band>,
    pub last_page_footer: Option<Band>,
    pub summary: Option<Band>,
    pub no_data: Option<Band>,
    /// `<subDataset>` definitions, used by tables.
    pub datasets: Vec<Dataset>,
    /// Features found in the template that are not rendered (yet).
    /// Expressions read `$P{REPORT_DATA_SOURCE}` (records then keep their source JSON row).
    pub uses_data_source: bool,
    pub warnings: Vec<String>,
}

impl Report {
    pub fn field(&self, name: &str) -> Option<&Field> {
        self.fields.iter().find(|f| f.name == name)
    }
}

/// A barcode component (`jr:QRCode`, `jr:Code128`, `jr:EAN13`, ...).
#[derive(Debug, Clone)]
pub struct BarcodeElement {
    pub symbology: Symbology,
    pub expr: Option<Expr>,
    pub boxed: BoxStyle,
    /// Font of the human-readable text (from the element's style).
    pub font_name: Option<String>,
    pub font_size: Option<f32>,
}

#[derive(Debug, Clone)]
pub enum Symbology {
    Qr {
        level: crate::barcode::qr::Ecl,
        margin: Option<u32>,
        version: Option<u8>,
    },
    Code128 {
        module_width: Option<f64>,
        text_position: crate::barcode::TextPosition,
        quiet_zone: Option<f64>,
        vertical_quiet_zone: Option<f64>,
    },
    /// Code 39, Interleaved 2 of 5, Codabar, EAN-13, EAN-8, UPC-A.
    Linear(crate::barcode::linear::Spec),
}

impl Symbology {
    /// The component name used in JRXML.
    pub fn component(&self) -> &'static str {
        match self {
            Symbology::Qr { .. } => "QRCode",
            Symbology::Code128 { .. } => "Code128",
            Symbology::Linear(spec) => spec.kind.component(),
        }
    }
}

/// A `<subreport>`: another template filled with its own data for each print of the element.
#[derive(Debug, Clone)]
pub struct SubreportElement {
    /// Evaluates to the template location (`"lines.jasper"` resolves to `lines.jrxml`).
    pub expr: Option<Expr>,
    pub parameters: Vec<(String, Expr)>,
    pub parameters_map: Option<Expr>,
    pub data_source: Option<Expr>,
    /// A JDBC connection expression is present (not supported: the subreport gets no rows).
    pub connection: bool,
    pub returns: Vec<ReturnValue>,
}

/// Copies a subreport variable into a variable of the containing report.
#[derive(Debug, Clone)]
pub struct ReturnValue {
    pub from: String,
    pub to: String,
    pub calculation: Calculation,
}

/// A `<subDataset>`: the parameters, fields, variables and groups of a data source other than
/// the report's own (tables iterate over one).
#[derive(Debug, Clone)]
pub struct Dataset {
    pub name: String,
    pub parameters: Vec<Parameter>,
    pub fields: Vec<Field>,
    pub variables: Vec<Variable>,
    pub groups: Vec<Group>,
}

/// A `<datasetRun>`: the subdataset a table or list iterates over and where its rows and
/// parameters come from.
#[derive(Debug, Clone)]
pub struct DatasetRun {
    /// The subdataset (`None`: a dataset with no fields).
    pub dataset: Option<Dataset>,
    pub parameters: Vec<(String, Expr)>,
    pub parameters_map: Option<Expr>,
    pub data_source: Option<Expr>,
    /// Page height of the containing report (the report built for the run uses it).
    pub page_height: f32,
    /// Expression language of the containing report.
    pub language: Language,
}

/// A table component. JasperReports fills a table as a subreport built from its columns
/// (`TableReport`); [`crate::table`] builds the same report.
#[derive(Debug, Clone)]
pub struct TableElement {
    pub run: DatasetRun,
    pub columns: Vec<TableColumn>,
    pub when_no_data: WhenNoData,
    pub no_data: Option<TableCell>,
}

/// A list component (`jr:list`): its contents repeated for each row of the subdataset,
/// one below the other.
#[derive(Debug, Clone)]
pub struct ListElement {
    pub run: DatasetRun,
    /// The contents, as a band as tall as `<listContents height>`.
    pub contents: Band,
}

/// A table column, or a column group with `children`.
#[derive(Debug, Clone)]
pub struct TableColumn {
    pub width: f32,
    pub print_when: Option<Expr>,
    pub children: Vec<TableColumn>,
    pub is_group: bool,
    pub table_header: Option<TableCell>,
    pub table_footer: Option<TableCell>,
    pub column_header: Option<TableCell>,
    pub column_footer: Option<TableCell>,
    pub detail: Option<TableCell>,
    pub group_headers: Vec<(String, TableCell)>,
    pub group_footers: Vec<(String, TableCell)>,
}

/// A table cell: its content as one element at (0, 0), a frame carrying the cell's style and
/// borders, or the cell's only text element when Jasper uses it directly.
#[derive(Debug, Clone)]
pub struct TableCell {
    pub height: f32,
    pub element: Element,
}
