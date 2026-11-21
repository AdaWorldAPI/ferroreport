//! Charts (`<pieChart>`, `<barChart>`, `<lineChart>`, ...): datasets collected while filling
//! and vector drawing that follows JFreeChart's default look as used by JasperReports
//! (palette, margins, tick-unit choice, legend and pie labels).
//!
//! Charts are drawn as vector graphics with real, selectable text. They reproduce Jasper's
//! layout closely but are not pixel-identical to JFreeChart.

#![allow(clippy::too_many_arguments, clippy::type_complexity)]

use crate::expr::Expr;
use crate::fonts::{FaceId, FontSet};
use crate::format::{format_value, Locale};
use crate::model::{BoxStyle, Color, EvaluationTime, FontSpec, Pen, ResetType};
use crate::print::{PathOp, PrintItem, PrintLine, PrintText};
use crate::value::Value;
use std::collections::HashMap;
use std::f64::consts::PI;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChartKind {
    Pie,
    Pie3D,
    Bar,
    Bar3D,
    StackedBar,
    StackedBar3D,
    Line,
    Area,
    StackedArea,
    XyLine,
    XyArea,
    Scatter,
}

impl ChartKind {
    pub fn from_tag(tag: &str) -> Option<ChartKind> {
        Some(match tag {
            "pieChart" => ChartKind::Pie,
            "pie3DChart" => ChartKind::Pie3D,
            "barChart" => ChartKind::Bar,
            "bar3DChart" => ChartKind::Bar3D,
            "stackedBarChart" => ChartKind::StackedBar,
            "stackedBar3DChart" => ChartKind::StackedBar3D,
            "lineChart" => ChartKind::Line,
            "areaChart" => ChartKind::Area,
            "stackedAreaChart" => ChartKind::StackedArea,
            "xyLineChart" => ChartKind::XyLine,
            "xyAreaChart" => ChartKind::XyArea,
            "scatterChart" => ChartKind::Scatter,
            _ => return None,
        })
    }

    fn is_pie(self) -> bool {
        matches!(self, ChartKind::Pie | ChartKind::Pie3D)
    }
    fn is_xy(self) -> bool {
        matches!(self, ChartKind::XyLine | ChartKind::XyArea | ChartKind::Scatter)
    }
    fn is_stacked(self) -> bool {
        matches!(self, ChartKind::StackedBar | ChartKind::StackedBar3D | ChartKind::StackedArea)
    }
    fn is_bar(self) -> bool {
        matches!(self, ChartKind::Bar | ChartKind::Bar3D | ChartKind::StackedBar | ChartKind::StackedBar3D)
    }
    fn is_3d(self) -> bool {
        matches!(self, ChartKind::Bar3D | ChartKind::StackedBar3D | ChartKind::Pie3D)
    }
}

/// When a dataset takes a new value.
#[derive(Debug, Clone, PartialEq)]
pub enum Increment {
    /// Every record (JasperReports' `incrementType="None"`).
    Record,
    /// When the group ends.
    Group(String),
    /// When the page ends.
    Page,
    /// Once, at the end of the report.
    Report,
}

#[derive(Debug, Clone, Default)]
pub struct SeriesDef {
    pub series: Option<Expr>,
    pub key: Option<Expr>,
    pub category: Option<Expr>,
    pub value: Option<Expr>,
    pub x: Option<Expr>,
    pub y: Option<Expr>,
}

#[derive(Debug, Clone)]
pub struct DatasetDef {
    pub reset: ResetType,
    pub increment: Increment,
    pub increment_when: Option<Expr>,
    pub series: Vec<SeriesDef>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Edge {
    Top,
    Bottom,
    Left,
    Right,
}

#[derive(Debug, Clone)]
pub struct ChartElement {
    /// Unique within its report: identifies the chart's dataset while filling.
    pub id: usize,
    pub kind: ChartKind,
    pub horizontal: bool,
    pub title: Option<Expr>,
    pub title_font: FontSpec,
    pub title_color: Color,
    pub subtitle: Option<Expr>,
    pub subtitle_font: FontSpec,
    pub subtitle_color: Color,
    pub show_legend: bool,
    pub legend_font: FontSpec,
    pub legend_color: Color,
    pub legend_back: Option<Color>,
    pub legend_position: Edge,
    pub font: FontSpec,
    pub evaluation_time: EvaluationTime,
    pub evaluation_group: Option<String>,
    pub dataset: DatasetDef,
    pub category_axis_label: Option<Expr>,
    pub value_axis_label: Option<Expr>,
    pub value_min: Option<Expr>,
    pub value_max: Option<Expr>,
    pub series_colors: Vec<Color>,
    pub show_labels: bool,
    pub show_lines: bool,
    pub show_shapes: bool,
    pub backcolor: Option<Color>,
    pub plot_backcolor: Option<Color>,
    pub value_format: Option<String>,
    pub boxed: BoxStyle,
}

/// Values collected for one chart.
#[derive(Debug, Clone, Default)]
pub struct DataState {
    pub pie: Vec<(String, f64)>,
    pub series: Vec<String>,
    pub categories: Vec<String>,
    pub values: HashMap<(usize, usize), f64>,
    pub xy: Vec<(String, Vec<(f64, f64)>)>,
}

impl DataState {
    pub fn add_pie(&mut self, key: String, value: f64) {
        // JasperReports refuses duplicate keys; adding them up is more useful.
        match self.pie.iter_mut().find(|(k, _)| *k == key) {
            Some(entry) => entry.1 += value,
            None => self.pie.push((key, value)),
        }
    }

    pub fn add_category(&mut self, series: String, category: String, value: f64) {
        let s = self.series.iter().position(|x| *x == series).unwrap_or_else(|| {
            self.series.push(series);
            self.series.len() - 1
        });
        let c = self.categories.iter().position(|x| *x == category).unwrap_or_else(|| {
            self.categories.push(category);
            self.categories.len() - 1
        });
        // Like JFreeChart's setValue: the last value for a (series, category) pair wins.
        self.values.insert((s, c), value);
    }

    pub fn add_xy(&mut self, series: String, x: f64, y: f64) {
        match self.xy.iter_mut().find(|(k, _)| *k == series) {
            Some(entry) => entry.1.push((x, y)),
            None => self.xy.push((series, vec![(x, y)])),
        }
    }
}

/// Texts evaluated when the chart is drawn.
#[derive(Debug, Clone, Default)]
pub struct ChartTexts {
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub category_label: Option<String>,
    pub value_label: Option<String>,
    pub value_min: Option<f64>,
    pub value_max: Option<f64>,
}

/// JFreeChart's default series colors.
const PALETTE: [u32; 16] = [
    0xFF5555, 0x5555FF, 0x55FF55, 0xFFFF55, 0xFF55FF, 0x55FFFF, 0xFFAFAF, 0x808080, 0xC00000, 0x0000C0, 0x00C000, 0xC0C000,
    0xC000C0, 0x00C0C0, 0x404040, 0xFF4040,
];

fn rgb(n: u32) -> Color {
    Color { r: (n >> 16) as u8, g: (n >> 8) as u8, b: n as u8 }
}

fn darker(c: Color, f: f32) -> Color {
    Color { r: (c.r as f32 * f) as u8, g: (c.g as f32 * f) as u8, b: (c.b as f32 * f) as u8 }
}

const GRID: Color = Color { r: 0xC0, g: 0xC0, b: 0xC0 };
const OUTLINE: Color = Color { r: 0x80, g: 0x80, b: 0x80 };

struct Canvas<'a> {
    fonts: &'a mut FontSet,
    items: Vec<PrintItem>,
}

#[derive(Clone, Copy)]
enum Anchor {
    Left,
    Center,
    Right,
}

impl Canvas<'_> {
    fn face(&mut self, font: &FontSpec) -> FaceId {
        self.fonts.resolve(font)
    }
    fn line_height(&mut self, font: &FontSpec) -> f32 {
        let id = self.face(font);
        self.fonts.face(id).line_height(font.size)
    }
    fn width(&mut self, font: &FontSpec, text: &str) -> f32 {
        let id = self.face(font);
        self.fonts.face(id).text_width(text, font.size)
    }
    /// Text with its line box top at `top`.
    fn text(&mut self, font: &FontSpec, color: Color, text: &str, x: f32, top: f32, anchor: Anchor) {
        if text.is_empty() {
            return;
        }
        let id = self.face(font);
        let face = self.fonts.face(id);
        let width = face.text_width(text, font.size);
        let ascent = face.ascent(font.size);
        let x = match anchor {
            Anchor::Left => x,
            Anchor::Center => x - width / 2.0,
            Anchor::Right => x - width,
        };
        self.items.push(PrintItem::Text(PrintText {
            face: id,
            size: font.size,
            color,
            underline: false,
            strike: false,
            lines: vec![PrintLine { text: text.to_string(), x, baseline: top + ascent, width, word_spacing: 0.0 }],
        }));
    }
    fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, fill: Option<Color>, stroke: Option<Pen>) {
        self.items.push(PrintItem::Rect { x, y, w, h, fill, stroke, radius: 0.0, path: false });
    }
    fn path(&mut self, ops: Vec<PathOp>, fill: Option<Color>, stroke: Option<(Color, f32)>, dash: Option<[f32; 2]>) {
        self.items.push(PrintItem::Path { ops: Arc::new(ops), fill, stroke, dash });
    }
    fn line(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, color: Color, width: f32, dash: Option<[f32; 2]>) {
        self.path(vec![PathOp::Move(x1 as f64, y1 as f64), PathOp::Line(x2 as f64, y2 as f64)], None, Some((color, width)), dash);
    }
}

fn pen(color: Color, width: f32) -> Pen {
    Pen { width, color, style: crate::model::LineStyle::Solid }
}

/// An elliptical arc from angle `a0` to `a1` (radians, y down) as cubic curves.
fn arc(ops: &mut Vec<PathOp>, cx: f64, cy: f64, rx: f64, ry: f64, a0: f64, a1: f64, first: bool) {
    let segments = ((a1 - a0).abs() / (PI / 2.0)).ceil().max(1.0) as usize;
    let step = (a1 - a0) / segments as f64;
    let k = 4.0 / 3.0 * (step / 4.0).tan();
    let p = |a: f64| (cx + rx * a.cos(), cy + ry * a.sin());
    if first {
        let (x, y) = p(a0);
        ops.push(PathOp::Move(x, y));
    }
    for i in 0..segments {
        let (t0, t1) = (a0 + step * i as f64, a0 + step * (i + 1) as f64);
        let (x0, y0) = p(t0);
        let (x3, y3) = p(t1);
        let c1 = (x0 - k * rx * t0.sin(), y0 + k * ry * t0.cos());
        let c2 = (x3 + k * rx * t1.sin(), y3 - k * ry * t1.cos());
        ops.push(PathOp::Cubic(c1.0, c1.1, c2.0, c2.1, x3, y3));
    }
}

/// JFreeChart's standard tick units: 1, 2.5 and 5 times powers of ten.
fn tick_candidates() -> impl Iterator<Item = f64> {
    (-8..=12).flat_map(|e| [1.0, 2.5, 5.0].into_iter().map(move |m| m * 10f64.powi(e)))
}

struct Axis {
    lower: f64,
    upper: f64,
    unit: f64,
}

impl Axis {
    fn ticks(&self) -> Vec<f64> {
        let mut out = Vec::new();
        let mut v = (self.lower / self.unit).ceil() * self.unit;
        while v <= self.upper + self.unit * 1e-9 {
            out.push(if v.abs() < self.unit * 1e-9 { 0.0 } else { v });
            v += self.unit;
        }
        out
    }
}

fn auto_range(min: f64, max: f64, include_zero: bool, fixed: (Option<f64>, Option<f64>)) -> (f64, f64) {
    let (mut lo, mut hi) = (min, max);
    if !lo.is_finite() || !hi.is_finite() {
        lo = 0.0;
        hi = 1.0;
    }
    if include_zero {
        lo = lo.min(0.0);
        hi = hi.max(0.0);
    }
    let range = hi - lo;
    let (lo, hi) = if range <= 0.0 {
        (lo - 0.5, hi + 0.5)
    } else {
        let lo2 = if include_zero && lo >= 0.0 { lo } else { lo - 0.05 * range };
        let hi2 = if include_zero && hi <= 0.0 { hi } else { hi + 0.05 * range };
        (lo2, hi2)
    };
    (fixed.0.unwrap_or(lo), fixed.1.unwrap_or(hi))
}

/// Width of the widest tick label for a tick unit (as JFreeChart estimates it, from the
/// range bounds rounded to the unit).
#[allow(clippy::too_many_arguments)]
fn widest_tick(cv: &mut Canvas, lower: f64, upper: f64, unit: f64, fmt: Option<&str>, locale: &Locale, font: &FontSpec) -> f32 {
    let hi = (upper / unit).floor() * unit;
    let lo = (lower / unit).ceil() * unit;
    cv.width(font, &tick_label(hi, unit, fmt, locale)).max(cv.width(font, &tick_label(lo, unit, fmt, locale)))
}

fn choose_unit(lower: f64, upper: f64, length: f32, mut min_spacing: impl FnMut(f64) -> f32) -> f64 {
    let range = (upper - lower).max(f64::MIN_POSITIVE);
    for unit in tick_candidates() {
        let px = (unit / range) as f32 * length;
        if px >= min_spacing(unit) {
            return unit;
        }
    }
    range
}

fn fmt_number(v: f64, pattern: Option<&str>, locale: &Locale) -> String {
    format_value(&Value::Double(v), Some(pattern.unwrap_or("#,##0.###")), locale)
}

/// Tick labels show as many decimals as the tick unit has (2.5 → "5.0"), like JFreeChart's
/// standard tick units.
fn tick_label(v: f64, unit: f64, pattern: Option<&str>, locale: &Locale) -> String {
    if pattern.is_some() {
        return fmt_number(v, pattern, locale);
    }
    let mut decimals = 0;
    while decimals < 8 && ((unit * 10f64.powi(decimals)).round() - unit * 10f64.powi(decimals)).abs() > 1e-9 {
        decimals += 1;
    }
    let p = if decimals == 0 { "#,##0".to_string() } else { format!("#,##0.{}", "0".repeat(decimals as usize)) };
    format_value(&Value::Double(v), Some(&p), locale)
}

/// Draws a chart in the box `x, y, w, h`.
#[allow(clippy::too_many_arguments)]
pub fn render(
    ch: &ChartElement,
    data: &DataState,
    texts: &ChartTexts,
    fonts: &mut FontSet,
    locale: &Locale,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
) -> Vec<PrintItem> {
    let mut cv = Canvas { fonts, items: Vec::new() };
    let p = ch.boxed.padding;
    let (x, y, w, h) = (x + p.left, y + p.top, (w - p.left - p.right).max(0.0), (h - p.top - p.bottom).max(0.0));
    if let Some(bg) = ch.backcolor {
        cv.rect(x, y, w, h, Some(bg), None);
    }
    let color_of = |i: usize| ch.series_colors.get(i).copied().unwrap_or_else(|| rgb(PALETTE[i % PALETTE.len()]));

    // Title and subtitle on top.
    let mut top = y;
    for (text, font, color) in
        [(&texts.title, &ch.title_font, ch.title_color), (&texts.subtitle, &ch.subtitle_font, ch.subtitle_color)]
    {
        if let Some(t) = text.as_deref().filter(|t| !t.is_empty()) {
            let lh = cv.line_height(font);
            cv.text(font, color, t, x + w / 2.0, top + 1.0, Anchor::Center);
            top += lh + 2.0;
        }
    }

    // Legend items.
    let legend: Vec<(String, usize)> = if !ch.show_legend {
        Vec::new()
    } else if ch.kind.is_pie() {
        data.pie.iter().enumerate().map(|(i, (k, _))| (k.clone(), i)).collect()
    } else if ch.kind.is_xy() {
        data.xy.iter().enumerate().map(|(i, (k, _))| (k.clone(), i)).collect()
    } else {
        data.series.iter().enumerate().map(|(i, k)| (k.clone(), i)).collect()
    };
    let mut bottom = y + h;
    if !legend.is_empty() {
        let lf = ch.legend_font.clone();
        let lh = cv.line_height(&lf);
        let widths: Vec<f32> = legend.iter().map(|(k, _)| cv.width(&lf, k)).collect();
        let line_chart = matches!(ch.kind, ChartKind::Line | ChartKind::XyLine | ChartKind::Scatter) && ch.show_lines;
        let shape_w = if line_chart { 14.0 } else { 8.0 };
        let item_w = |i: usize| shape_w + 4.6 + widths[i];
        let total: f32 = (0..legend.len()).map(item_w).sum::<f32>() + 6.0 * (legend.len() as f32 - 1.0) + 7.0;
        let box_w = total.min(w - 2.0);
        let box_h = lh + 7.0;
        let (bx, by) = match ch.legend_position {
            Edge::Top => (x + (w - box_w) / 2.0, top + 1.0),
            _ => (x + (w - box_w) / 2.0, y + h - 1.5 - box_h),
        };
        if let Some(back) = ch.legend_back {
            cv.rect(bx, by, box_w, box_h, Some(back), None);
        }
        cv.rect(bx, by, box_w, box_h, None, Some(pen(Color::BLACK, 1.0)));
        let mut ix = bx + 3.5;
        let cy = by + box_h / 2.0;
        for (i, (label, series)) in legend.iter().enumerate() {
            let color = color_of(*series);
            if ch.kind.is_pie() {
                let mut ops = Vec::new();
                arc(&mut ops, (ix + 4.0) as f64, cy as f64, 4.0, 4.0, 0.0, 2.0 * PI, true);
                ops.push(PathOp::Close);
                cv.path(ops, Some(color), Some((OUTLINE, 0.5)), None);
            } else if line_chart {
                cv.line(ix, cy, ix + 14.0, cy, color, 1.0, None);
                if ch.show_shapes {
                    shape(&mut cv, *series, ix + 7.0, cy, color);
                }
            } else if ch.kind == ChartKind::Scatter {
                shape(&mut cv, *series, ix + 4.0, cy, color);
            } else {
                cv.rect(ix, cy - 4.0, 8.0, 8.0, Some(color), None);
            }
            cv.text(&lf, ch.legend_color, label, ix + shape_w + 4.6, cy - lh / 2.0, Anchor::Left);
            ix += item_w(i) + 6.0;
            if ix > bx + box_w {
                break;
            }
        }
        match ch.legend_position {
            Edge::Top => top = by + box_h + 2.0,
            _ => bottom = by - 2.0,
        }
    }

    // Plot area (JFreeChart's default plot insets: 4 top/bottom, 8 left/right).
    let (px, py, pw, ph) = (x + 8.0, top + 4.0, (w - 16.0).max(1.0), (bottom - top - 8.0).max(1.0));
    if ch.kind.is_pie() {
        pie(&mut cv, ch, data, px, py, pw, ph, &color_of);
    } else {
        xy_or_category(&mut cv, ch, data, texts, locale, px, py, pw, ph, &color_of);
    }
    cv.items
}

/// The shape JFreeChart's default drawing supplier gives a series (size 6).
fn shape(cv: &mut Canvas, series: usize, cx: f32, cy: f32, color: Color) {
    let (cx, cy) = (cx as f64, cy as f64);
    let mut ops = Vec::new();
    let poly = |pts: &[(f64, f64)], ops: &mut Vec<PathOp>| {
        ops.push(PathOp::Move(cx + pts[0].0, cy + pts[0].1));
        for p in &pts[1..] {
            ops.push(PathOp::Line(cx + p.0, cy + p.1));
        }
        ops.push(PathOp::Close);
    };
    match series % 10 {
        0 => poly(&[(-3.0, -3.0), (3.0, -3.0), (3.0, 3.0), (-3.0, 3.0)], &mut ops),
        1 => {
            arc(&mut ops, cx, cy, 3.0, 3.0, 0.0, 2.0 * PI, true);
            ops.push(PathOp::Close);
        }
        2 => poly(&[(0.0, -3.0), (3.0, 3.0), (-3.0, 3.0)], &mut ops),
        3 => poly(&[(0.0, -3.0), (3.0, 0.0), (0.0, 3.0), (-3.0, 0.0)], &mut ops),
        4 => poly(&[(-3.0, -1.5), (3.0, -1.5), (3.0, 1.5), (-3.0, 1.5)], &mut ops),
        5 => poly(&[(-3.0, -3.0), (3.0, -3.0), (0.0, 3.0)], &mut ops),
        6 => {
            arc(&mut ops, cx, cy, 3.0, 1.5, 0.0, 2.0 * PI, true);
            ops.push(PathOp::Close);
        }
        7 => poly(&[(-3.0, -3.0), (3.0, 0.0), (-3.0, 3.0)], &mut ops),
        8 => poly(&[(-1.5, -3.0), (1.5, -3.0), (1.5, 3.0), (-1.5, 3.0)], &mut ops),
        _ => poly(&[(3.0, -3.0), (3.0, 3.0), (-3.0, 0.0)], &mut ops),
    }
    cv.path(ops, Some(color), None, None);
}

#[allow(clippy::too_many_arguments)]
fn pie(
    cv: &mut Canvas,
    ch: &ChartElement,
    data: &DataState,
    px: f32,
    py: f32,
    pw: f32,
    ph: f32,
    color_of: &dyn Fn(usize) -> Color,
) {
    let total: f64 = data.pie.iter().map(|(_, v)| v.max(0.0)).sum();
    if total <= 0.0 {
        return;
    }
    let three_d = ch.kind.is_3d();
    let r = (pw.min(ph) as f64) * 0.84 / 2.0;
    let (cx, cy) = ((px + pw / 2.0) as f64, (py + ph / 2.0) as f64);
    let (rx, ry, depth) = if three_d { (r, r * 0.75, r * 0.2) } else { (r, r, 0.0) };
    let cy = cy - depth / 2.0;
    // Slices clockwise from twelve o'clock.
    let mut slices = Vec::new();
    let mut a = -PI / 2.0;
    for (i, (key, v)) in data.pie.iter().enumerate() {
        let sweep = 2.0 * PI * v.max(0.0) / total;
        slices.push((i, key.clone(), a, a + sweep));
        a += sweep;
    }
    let wedge = |cx: f64, cy: f64, a0: f64, a1: f64| {
        let mut ops = vec![PathOp::Move(cx, cy)];
        let (sx, sy) = (cx + rx * a0.cos(), cy + ry * a0.sin());
        ops.push(PathOp::Line(sx, sy));
        arc(&mut ops, cx, cy, rx, ry, a0, a1, false);
        ops.push(PathOp::Close);
        ops
    };
    if three_d {
        // The visible side of the pie, front half.
        for &(i, _, a0, a1) in &slices {
            let (s0, s1) = (a0.max(0.0), a1.min(PI));
            if s1 > s0 {
                let mut ops = Vec::new();
                arc(&mut ops, cx, cy, rx, ry, s0, s1, true);
                ops.push(PathOp::Line(cx + rx * s1.cos(), cy + depth + ry * s1.sin()));
                arc(&mut ops, cx, cy + depth, rx, ry, s1, s0, false);
                ops.push(PathOp::Close);
                cv.path(ops, Some(darker(color_of(i), 0.7)), Some((OUTLINE, 0.5)), None);
            }
            // Slices that wrap past 2π.
            let (s0, s1) = ((a0 - 2.0 * PI).max(0.0), (a1 - 2.0 * PI).min(PI));
            if s1 > s0 {
                let mut ops = Vec::new();
                arc(&mut ops, cx, cy, rx, ry, s0, s1, true);
                ops.push(PathOp::Line(cx + rx * s1.cos(), cy + depth + ry * s1.sin()));
                arc(&mut ops, cx, cy + depth, rx, ry, s1, s0, false);
                ops.push(PathOp::Close);
                cv.path(ops, Some(darker(color_of(i), 0.7)), Some((OUTLINE, 0.5)), None);
            }
        }
    } else {
        // JFreeChart's shadow: the pie offset by 4 points in gray.
        for &(_, _, a0, a1) in &slices {
            cv.path(wedge(cx + 4.0, cy + 4.0, a0, a1), Some(OUTLINE), None, None);
        }
    }
    for &(i, _, a0, a1) in &slices {
        cv.path(wedge(cx, cy, a0, a1), Some(color_of(i)), Some((OUTLINE, 0.5)), None);
    }

    if !ch.show_labels {
        return;
    }
    // Labels in boxes on both sides, linked to their slices.
    let font = ch.font.clone();
    let lh = cv.line_height(&font);
    let box_h = lh + 4.0;
    let mut sides: [Vec<(f64, f64, f64, String, f64)>; 2] = [Vec::new(), Vec::new()];
    for (_, key, a0, a1) in &slices {
        let mid = (a0 + a1) / 2.0;
        let right = mid.cos() >= 0.0;
        let start = (cx + 0.88 * rx * mid.cos(), cy + 0.88 * ry * mid.sin());
        let elbow =
            (cx + 1.03 * rx * mid.cos(), cy + 1.03 * ry * mid.sin() + if three_d && mid.sin() > 0.0 { depth } else { 0.0 });
        sides[right as usize].push((elbow.1, start.0, start.1, key.clone(), elbow.0));
    }
    for (side, labels) in sides.iter_mut().enumerate() {
        labels.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        // Spread labels so their boxes do not overlap.
        let min_gap = box_h as f64 + 2.0;
        for i in 1..labels.len() {
            if labels[i].0 - labels[i - 1].0 < min_gap {
                labels[i].0 = labels[i - 1].0 + min_gap;
            }
        }
        for (ey, sx, sy, key, ex) in labels.iter() {
            let tw = cv.width(&font, key) as f64;
            let bw = tw + 3.0;
            let edge = if side == 1 { cx + 1.083 * rx } else { cx - 1.083 * rx };
            let bx = if side == 1 { edge } else { edge - bw };
            let by = ey - box_h as f64 / 2.0;
            cv.path(
                vec![PathOp::Move(*sx, *sy), PathOp::Line(*ex, *ey), PathOp::Line(edge, *ey)],
                None,
                Some((Color::BLACK, 0.5)),
                None,
            );
            cv.rect((bx + 2.0) as f32, (by + 2.0) as f32, bw as f32, box_h, Some(Color { r: 0x97, g: 0x97, b: 0x97 }), None);
            cv.rect(bx as f32, by as f32, bw as f32, box_h, Some(Color::WHITE), Some(pen(Color::BLACK, 0.5)));
            cv.text(&font, Color::BLACK, key, (bx + 1.5) as f32, (by + 2.0) as f32, Anchor::Left);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn xy_or_category(
    cv: &mut Canvas,
    ch: &ChartElement,
    data: &DataState,
    texts: &ChartTexts,
    locale: &Locale,
    px: f32,
    py: f32,
    pw: f32,
    ph: f32,
    color_of: &dyn Fn(usize) -> Color,
) {
    let font = ch.font.clone();
    let lh = cv.line_height(&font);
    let horizontal = ch.horizontal && !ch.kind.is_xy();
    let fmt = ch.value_format.as_deref();

    // Value range (y for category and XY charts).
    let (mut vmin, mut vmax) = (f64::INFINITY, f64::NEG_INFINITY);
    let ncat = data.categories.len();
    let nser = data.series.len();
    if ch.kind.is_xy() {
        for (_, pts) in &data.xy {
            for &(_, yv) in pts {
                vmin = vmin.min(yv);
                vmax = vmax.max(yv);
            }
        }
    } else if ch.kind.is_stacked() {
        for c in 0..ncat {
            let (mut pos, mut neg) = (0.0, 0.0);
            for s in 0..nser {
                let v = data.values.get(&(s, c)).copied().unwrap_or(0.0);
                if v >= 0.0 {
                    pos += v
                } else {
                    neg += v
                }
            }
            vmin = vmin.min(neg);
            vmax = vmax.max(pos);
        }
    } else {
        for v in data.values.values() {
            vmin = vmin.min(*v);
            vmax = vmax.max(*v);
        }
    }
    // Scatter plots do not stretch their range to zero (JFreeChart's createScatterPlot).
    let (vlo, vhi) = auto_range(vmin, vmax, ch.kind != ChartKind::Scatter, (texts.value_min, texts.value_max));

    // Domain range for XY charts.
    let (mut xmin, mut xmax) = (f64::INFINITY, f64::NEG_INFINITY);
    for (_, pts) in &data.xy {
        for &(xv, _) in pts {
            xmin = xmin.min(xv);
            xmax = xmax.max(xv);
        }
    }
    let (xlo, xhi) = auto_range(xmin, xmax, false, (None, None));

    // Space taken by the axes around the data area.
    let value_label = texts.value_label.clone().filter(|s| !s.is_empty());
    let cat_label = texts.category_label.clone().filter(|s| !s.is_empty());
    let axis_label_h = |present: bool| if present { lh + 6.0 } else { 0.0 };

    let (dx, dy, dw, dh);
    let value_axis;
    if !horizontal {
        // Vertical value axis on the left, domain axis at the bottom.
        let bottom_h = 4.0 + (lh + 4.0) + axis_label_h(cat_label.is_some());
        dh = (ph - bottom_h).max(1.0);
        let unit = choose_unit(vlo, vhi, dh, |_| lh + 4.0);
        value_axis = Axis { lower: vlo, upper: vhi, unit };
        let widest = value_axis
            .ticks()
            .into_iter()
            .map(|t| {
                let l = tick_label(t, value_axis.unit, fmt, locale);
                cv.width(&font, &l)
            })
            .fold(0.0f32, f32::max);
        let left_w = axis_label_h(value_label.is_some()) + 4.0 + widest + 4.0 + 2.0;
        dx = px + left_w;
        dy = py;
        dw = (pw - left_w).max(1.0);
    } else {
        // Horizontal bars: categories on the left, value axis on top.
        let widest_cat = data.categories.iter().map(|c| cv.width(&font, c)).fold(0.0f32, f32::max);
        let left_w = axis_label_h(cat_label.is_some()) + 4.0 + widest_cat + 4.0;
        let top_h = axis_label_h(value_label.is_some()) + (lh + 4.0) + 2.0;
        dx = px + left_w;
        dy = py + top_h;
        dw = (pw - left_w).max(1.0);
        dh = (ph - top_h).max(1.0);
        let unit = choose_unit(vlo, vhi, dw, |u| widest_tick(cv, vlo, vhi, u, fmt, locale, &font) + 8.0);
        value_axis = Axis { lower: vlo, upper: vhi, unit };
    }
    let xaxis = if ch.kind.is_xy() {
        Some(Axis {
            lower: xlo,
            upper: xhi,
            unit: choose_unit(xlo, xhi, dw, |u| widest_tick(cv, xlo, xhi, u, fmt, locale, &font) + 8.0),
        })
    } else {
        None
    };

    if let Some(bg) = ch.plot_backcolor {
        cv.rect(dx, dy, dw, dh, Some(bg), None);
    }
    // Value position along its axis.
    let vpos = |v: f64| -> f32 {
        let t = ((v - value_axis.lower) / (value_axis.upper - value_axis.lower)) as f32;
        if horizontal {
            dx + t * dw
        } else {
            dy + dh - t * dh
        }
    };
    let xpos = |v: f64| -> f32 {
        let a = xaxis.as_ref().expect("xy axis");
        dx + ((v - a.lower) / (a.upper - a.lower)) as f32 * dw
    };

    // Gridlines and value axis.
    let dash = Some([2.0, 2.0]);
    for t in value_axis.ticks() {
        let p = vpos(t);
        let label = tick_label(t, value_axis.unit, fmt, locale);
        if horizontal {
            cv.line(p, dy, p, dy + dh, GRID, 0.5, dash);
            cv.line(p, dy - 2.0, p, dy, Color::BLACK, 0.5, None);
            cv.text(&font, Color::BLACK, &label, p, dy - 2.0 - 4.0 - lh + 2.0, Anchor::Center);
        } else {
            cv.line(dx, p, dx + dw, p, GRID, 0.5, dash);
            cv.line(dx - 2.0, p, dx, p, Color::BLACK, 0.5, None);
            cv.text(&font, Color::BLACK, &label, dx - 6.0, p - lh / 2.0, Anchor::Right);
        }
    }
    if let Some(a) = &xaxis {
        for t in a.ticks() {
            let p = xpos(t);
            cv.line(p, dy, p, dy + dh, GRID, 0.5, dash);
            cv.line(p, dy + dh, p, dy + dh + 2.0, Color::BLACK, 0.5, None);
            cv.text(&font, Color::BLACK, &tick_label(t, a.unit, fmt, locale), p, dy + dh + 4.0 + 2.0, Anchor::Center);
        }
    }
    // Axis lines.
    if horizontal {
        cv.line(dx, dy, dx + dw, dy, Color::BLACK, 0.5, None);
        cv.line(dx, dy, dx, dy + dh, Color::BLACK, 0.5, None);
    } else {
        cv.line(dx, dy, dx, dy + dh, Color::BLACK, 0.5, None);
        cv.line(dx, dy + dh, dx + dw, dy + dh, Color::BLACK, 0.5, None);
    }

    // Axis labels.
    if let Some(l) = &value_label {
        if horizontal {
            cv.text(&font, Color::BLACK, l, dx + dw / 2.0, py + 3.0, Anchor::Center);
        } else {
            rotated_label(cv, &font, l, px + 3.0, dy + dh / 2.0);
        }
    }
    if let Some(l) = &cat_label {
        if horizontal {
            rotated_label(cv, &font, l, px + 3.0, dy + dh / 2.0);
        } else {
            cv.text(&font, Color::BLACK, l, dx + dw / 2.0, dy + dh + 4.0 + lh + 4.0 + 3.0, Anchor::Center);
        }
    }

    if ch.kind.is_xy() {
        draw_xy(cv, ch, data, &xpos, &vpos, value_axis.lower.max(0.0).min(value_axis.upper), color_of);
        return;
    }

    // Category bands: 5% margins at both ends, 20% between categories.
    let span = if horizontal { dh } else { dw };
    let start = if horizontal { dy } else { dx };
    let cat_margin = if ncat > 1 { 0.2 } else { 0.0 };
    let cat_w = span * (1.0 - 0.1 - cat_margin) / ncat.max(1) as f32;
    let cat_gap = if ncat > 1 { span * cat_margin / (ncat - 1) as f32 } else { 0.0 };
    let cat_start = |c: usize| start + 0.05 * span + c as f32 * (cat_w + cat_gap);
    for (c, name) in data.categories.iter().enumerate() {
        let mid = cat_start(c) + cat_w / 2.0;
        if horizontal {
            cv.text(&font, Color::BLACK, name, dx - 4.0, mid - lh / 2.0, Anchor::Right);
        } else {
            cv.text(&font, Color::BLACK, name, mid, dy + dh + 4.0 + 2.0, Anchor::Center);
        }
    }
    let base = vpos(0.0f64.max(value_axis.lower).min(value_axis.upper));
    let value = |s: usize, c: usize| data.values.get(&(s, c)).copied();

    match ch.kind {
        ChartKind::Bar | ChartKind::Bar3D => {
            let item_margin = if nser > 1 { 0.2 } else { 0.0 };
            let used = span * (1.0 - 0.1 - cat_margin - item_margin);
            let bar_w = used / (nser * ncat).max(1) as f32;
            let item_gap = if nser > 1 { span * item_margin / (ncat * (nser - 1)).max(1) as f32 } else { 0.0 };
            for c in 0..ncat {
                for s in 0..nser {
                    let Some(v) = value(s, c) else { continue };
                    let a = cat_start(c) + s as f32 * (bar_w + item_gap);
                    let (p0, p1) = (base, vpos(v));
                    bar(cv, horizontal, a, bar_w, p0, p1, color_of(s), ch.kind.is_3d());
                }
            }
        }
        ChartKind::StackedBar | ChartKind::StackedBar3D => {
            for c in 0..ncat {
                let (mut pos, mut neg) = (0.0, 0.0);
                for s in 0..nser {
                    let Some(v) = value(s, c) else { continue };
                    let from = if v >= 0.0 { pos } else { neg };
                    let to = from + v;
                    if v >= 0.0 {
                        pos = to
                    } else {
                        neg = to
                    }
                    bar(cv, horizontal, cat_start(c), cat_w, vpos(from), vpos(to), color_of(s), ch.kind.is_3d());
                }
            }
        }
        ChartKind::Line => {
            for s in 0..nser {
                let pts: Vec<(f32, f32)> = (0..ncat)
                    .filter_map(|c| value(s, c).map(|v| (cat_start(c) + cat_w / 2.0, vpos(v))))
                    .map(|(a, b)| if horizontal { (b, a) } else { (a, b) })
                    .collect();
                if ch.show_lines && pts.len() > 1 {
                    let mut ops = vec![PathOp::Move(pts[0].0 as f64, pts[0].1 as f64)];
                    ops.extend(pts[1..].iter().map(|p| PathOp::Line(p.0 as f64, p.1 as f64)));
                    cv.path(ops, None, Some((color_of(s), 1.0)), None);
                }
                if ch.show_shapes {
                    for p in &pts {
                        shape(cv, s, p.0, p.1, color_of(s));
                    }
                }
            }
        }
        ChartKind::Area | ChartKind::StackedArea => {
            let mut acc = vec![0.0f64; ncat];
            for s in 0..nser {
                let mut top: Vec<(f32, f32)> = Vec::new();
                let mut bottom: Vec<(f32, f32)> = Vec::new();
                for (c, acc_c) in acc.iter_mut().enumerate() {
                    let v = value(s, c).unwrap_or(0.0);
                    let lo = if ch.kind == ChartKind::StackedArea { *acc_c } else { 0.0 };
                    let hi = lo + v;
                    if ch.kind == ChartKind::StackedArea {
                        *acc_c = hi;
                    }
                    top.push((cat_start(c) + cat_w / 2.0, vpos(hi)));
                    bottom.push((cat_start(c) + cat_w / 2.0, vpos(lo)));
                }
                if top.is_empty() {
                    continue;
                }
                // Taper to the base at both ends, like JFreeChart's AreaRenderer.
                let first_x = cat_start(0);
                let last_x = cat_start(ncat - 1) + cat_w;
                let mut pts = vec![(first_x, *bottom.first().map(|p| &p.1).unwrap_or(&base))];
                pts.extend(top.iter().copied());
                pts.push((last_x, bottom.last().map(|p| p.1).unwrap_or(base)));
                pts.extend(bottom.iter().rev().copied());
                let pts: Vec<(f32, f32)> = pts.into_iter().map(|(a, b)| if horizontal { (b, a) } else { (a, b) }).collect();
                let mut ops = vec![PathOp::Move(pts[0].0 as f64, pts[0].1 as f64)];
                ops.extend(pts[1..].iter().map(|p| PathOp::Line(p.0 as f64, p.1 as f64)));
                ops.push(PathOp::Close);
                cv.path(ops, Some(color_of(s)), None, None);
            }
        }
        _ => {}
    }
    let _ = (nser, ch.kind.is_bar());
}

#[allow(clippy::too_many_arguments)]
fn bar(cv: &mut Canvas, horizontal: bool, a: f32, width: f32, p0: f32, p1: f32, color: Color, three_d: bool) {
    let (lo, hi) = (p0.min(p1), p0.max(p1));
    let (x, y, w, h) = if horizontal { (lo, a, hi - lo, width) } else { (a, lo, width, hi - lo) };
    if three_d {
        let d = (width * 0.35).min(8.0);
        let (x0, y0, x1, y1) = (x as f64, y as f64, (x + w) as f64, (y + h) as f64);
        let d = d as f64;
        // Top and side faces, offset up and to the right.
        let top = vec![
            PathOp::Move(x0, y0),
            PathOp::Line(x0 + d, y0 - d),
            PathOp::Line(x1 + d, y0 - d),
            PathOp::Line(x1, y0),
            PathOp::Close,
        ];
        let side = vec![
            PathOp::Move(x1, y0),
            PathOp::Line(x1 + d, y0 - d),
            PathOp::Line(x1 + d, y1 - d),
            PathOp::Line(x1, y1),
            PathOp::Close,
        ];
        cv.path(top, Some(darker(color, 0.85)), None, None);
        cv.path(side, Some(darker(color, 0.7)), None, None);
    }
    cv.rect(x, y, w, h, Some(color), None);
}

fn rotated_label(cv: &mut Canvas, font: &FontSpec, text: &str, left: f32, center_y: f32) {
    let w = cv.width(font, text);
    let mut inner = Canvas { fonts: cv.fonts, items: Vec::new() };
    inner.text(font, Color::BLACK, text, 0.0, 0.0, Anchor::Left);
    let items = inner.items;
    // Turn 90° counter-clockwise: local (u, v) lands at (left + v, center_y + w/2 - u).
    cv.items.push(PrintItem::Transform([0.0, -1.0, 1.0, 0.0, left, center_y + w / 2.0], items));
}

#[allow(clippy::too_many_arguments)]
fn draw_xy(
    cv: &mut Canvas,
    ch: &ChartElement,
    data: &DataState,
    xpos: &dyn Fn(f64) -> f32,
    vpos: &dyn Fn(f64) -> f32,
    base_value: f64,
    color_of: &dyn Fn(usize) -> Color,
) {
    for (s, (_, pts)) in data.xy.iter().enumerate() {
        let mut pts = pts.clone();
        pts.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        let screen: Vec<(f32, f32)> = pts.iter().map(|&(x, y)| (xpos(x), vpos(y))).collect();
        if screen.is_empty() {
            continue;
        }
        match ch.kind {
            ChartKind::XyArea => {
                let base = vpos(base_value);
                let mut ops = vec![PathOp::Move(screen[0].0 as f64, base as f64)];
                ops.extend(screen.iter().map(|p| PathOp::Line(p.0 as f64, p.1 as f64)));
                ops.push(PathOp::Line(screen[screen.len() - 1].0 as f64, base as f64));
                ops.push(PathOp::Close);
                cv.path(ops, Some(color_of(s)), None, None);
            }
            _ => {
                if ch.show_lines && screen.len() > 1 {
                    let mut ops = vec![PathOp::Move(screen[0].0 as f64, screen[0].1 as f64)];
                    ops.extend(screen[1..].iter().map(|p| PathOp::Line(p.0 as f64, p.1 as f64)));
                    cv.path(ops, None, Some((color_of(s), 1.0)), None);
                }
                if ch.show_shapes {
                    for p in &screen {
                        shape(cv, s, p.0, p.1, color_of(s));
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_units_follow_jfreechart() {
        // 90,000 over 143 points with 14.5-point labels: steps of 10,000.
        assert_eq!(choose_unit(0.0, 90_334.0, 143.0, |_| 14.48), 10_000.0);
        assert_eq!(choose_unit(0.0, 14.65, 200.0, |_| 30.0), 2.5);
        let en = crate::format::locale::EN_US.clone();
        assert_eq!(tick_label(5.0, 2.5, None, &en), "5.0");
        assert_eq!(tick_label(30_000.0, 5_000.0, None, &en), "30,000");
    }

    #[test]
    fn datasets_accumulate() {
        let mut d = DataState::default();
        d.add_pie("a".into(), 1.0);
        d.add_pie("a".into(), 2.0);
        assert_eq!(d.pie, vec![("a".to_string(), 3.0)]);
        d.add_category("s".into(), "c".into(), 1.0);
        d.add_category("s".into(), "c".into(), 5.0);
        assert_eq!(d.values[&(0, 0)], 5.0);
        let (lo, hi) = auto_range(0.0, 100.0, true, (None, None));
        assert_eq!((lo, hi), (0.0, 105.0));
    }
}
