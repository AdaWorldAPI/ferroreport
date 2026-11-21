//! The filled document: absolutely positioned primitives, ready for any exporter.
//!
//! Coordinates are in points with the origin at the top-left corner of the page.

use crate::fonts::FaceId;
use crate::image::ImageData;
use crate::model::{Color, Pen};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct PrintLine {
    pub text: String,
    /// Left edge of the line.
    pub x: f32,
    /// Baseline position.
    pub baseline: f32,
    pub width: f32,
    /// Extra space added after each space character (justified text).
    pub word_spacing: f32,
}

#[derive(Debug, Clone)]
pub struct PrintText {
    pub face: FaceId,
    pub size: f32,
    pub color: Color,
    pub underline: bool,
    pub strike: bool,
    pub lines: Vec<PrintLine>,
}

#[derive(Debug, Clone)]
pub enum PrintItem {
    /// A path in page coordinates (charts), filled with the nonzero rule and/or stroked.
    Path {
        ops: Arc<Vec<PathOp>>,
        fill: Option<Color>,
        stroke: Option<(Color, f32)>,
        dash: Option<[f32; 2]>,
    },
    /// Several items drawn in order (the runs of a styled text).
    Group(Vec<PrintItem>),
    /// Items drawn in a local coordinate system: a local point `(u, v)` lands on the page at
    /// `(a·u + c·v + e, b·u + d·v + f)` (points, y down). Used for rotated text.
    Transform([f32; 6], Vec<PrintItem>),
    /// Filled rectangles drawn in a local drawing scaled by `scale` from `(x, y)` (barcodes).
    Shapes {
        x: f32,
        y: f32,
        scale: f64,
        color: Color,
        rects: Arc<Vec<[f64; 4]>>,
        /// Outlines (glyphs of human-readable text), filled with the nonzero rule.
        path: Arc<Vec<PathOp>>,
    },
    Rect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        fill: Option<Color>,
        stroke: Option<Pen>,
        radius: f32,
    },
    Ellipse {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        fill: Option<Color>,
        stroke: Option<Pen>,
    },
    Line {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        pen: Pen,
    },
    Text(PrintText),
    /// An image drawn into `x, y, w, h`, optionally clipped to `clip` (x, y, w, h).
    Image {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        clip: Option<[f32; 4]>,
        image: Arc<ImageData>,
    },
}

#[derive(Debug, Clone, Default)]
pub struct PrintPage {
    pub items: Vec<PrintItem>,
}

#[derive(Debug, Clone)]
pub struct PrintDocument {
    pub name: String,
    pub page_width: f32,
    pub page_height: f32,
    pub pages: Vec<PrintPage>,
}

/// One step of an outline, in the coordinates of a [`PrintItem::Shapes`] drawing.
#[derive(Debug, Clone, Copy)]
pub enum PathOp {
    Move(f64, f64),
    Line(f64, f64),
    /// A quadratic segment, which JasperReports' PDF exporter (iText's `PdfGraphics2D`)
    /// writes as a cubic whose first control point is the current point (`v`).
    Quad(f64, f64, f64, f64),
    Cubic(f64, f64, f64, f64, f64, f64),
    Close,
}

/// A horizontal slice of filled content (one band of a subreport), positioned relative to
/// its own top edge.
#[derive(Debug, Clone)]
pub struct Segment {
    pub height: f32,
    pub items: Vec<PrintItem>,
    /// Page or column header: printed again at the top of each page the content continues on.
    pub header: bool,
    /// The page footer for pages the content continues past: not part of the flow, printed
    /// after the last band that fits on each such page.
    pub footer: bool,
}

impl Segment {
    /// Height of content made of `segments`, leaving out page-footer templates.
    pub fn flow_height(segments: &[Segment]) -> f32 {
        segments.iter().filter(|s| !s.footer).map(|s| s.height).sum()
    }
}
