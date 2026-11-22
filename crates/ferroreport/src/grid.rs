//! Lays printed pages out on a grid of cells, like JasperReports' `JRGridLayout`: the edges of
//! the elements cut the page into columns and rows, and every element takes the cells between
//! its edges. Spreadsheet and CSV exporters write that grid.

use crate::model::{BoxStyle, Color, LineStyle, Pen};
use crate::print::{CellInfo, ElementInfo, PrintItem, Shape};

/// What an exporter lays out.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Nature {
    /// Elements take every cell between their edges (spreadsheets); otherwise only the cell at
    /// their top-left corner (CSV).
    pub span: bool,
    /// Only texts (and the frames holding them) are exported.
    pub texts_only: bool,
}

/// A printed element in whole points, as JasperReports keeps them.
#[derive(Debug)]
pub(crate) struct Node<'a> {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub what: What<'a>,
}

#[derive(Debug)]
pub(crate) enum What<'a> {
    Text(&'a CellInfo),
    /// A non-text element, its children (frames) and the items drawing it.
    Shape(&'a ElementInfo, Vec<Node<'a>>, &'a [PrintItem]),
}

impl Node<'_> {
    fn exported(&self, nature: Nature) -> bool {
        match &self.what {
            What::Text(_) => true,
            What::Shape(e, ..) => !nature.texts_only || matches!(e.shape, Shape::Frame),
        }
    }

    fn children(&self) -> Option<(&ElementInfo, &[Node<'_>])> {
        match &self.what {
            What::Shape(e, children, _) if matches!(e.shape, Shape::Frame) => Some((e, children)),
            _ => None,
        }
    }
}

fn r(v: f32) -> i32 {
    v.round() as i32
}

/// The elements of a page (or of a frame) from its print items.
pub(crate) fn nodes(items: &[PrintItem]) -> Vec<Node<'_>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < items.len() {
        match &items[i] {
            PrintItem::Cell(c, _) => {
                out.push(Node { x: r(c.x), y: r(c.y), w: r(c.w), h: r(c.h), what: What::Text(c) });
                i += 1;
            }
            PrintItem::Element(e) => {
                let end = (i + 1 + e.len).min(items.len());
                let children = if matches!(e.shape, Shape::Frame) { nodes(&items[i + 1..end]) } else { Vec::new() };
                let drawn = &items[i + 1..end];
                out.push(Node { x: r(e.x), y: r(e.y), w: r(e.w), h: r(e.h), what: What::Shape(e, children, drawn) });
                i = end;
            }
            _ => i += 1,
        }
    }
    out
}

/// Sorted cut offsets along one axis, with which of them start an element.
#[derive(Debug, Clone, Default)]
pub(crate) struct Cuts {
    pub offsets: Vec<i32>,
    /// `used[i]`: an element starts at cut `i` (`CutsInfo.isCutNotEmpty`).
    pub used: Vec<bool>,
}

impl Cuts {
    fn with(values: &[i32]) -> Cuts {
        let mut c = Cuts::default();
        for &v in values {
            c.add(v);
        }
        c
    }

    fn add(&mut self, v: i32) {
        if let Err(i) = self.offsets.binary_search(&v) {
            self.offsets.insert(i, v);
            self.used.insert(i, false);
        }
    }

    pub(crate) fn index(&self, v: i32) -> usize {
        self.offsets.binary_search(&v).unwrap_or_else(|i| i)
    }

    /// Number of columns or rows between the cuts.
    pub fn spans(&self) -> usize {
        self.offsets.len().saturating_sub(1)
    }

    pub fn size(&self, i: usize) -> i32 {
        self.offsets[i + 1] - self.offsets[i]
    }
}

/// Adds the edges of the elements of one page to the column cuts.
pub(crate) fn add_x_cuts(nodes: &[Node<'_>], nature: Nature, cuts: &mut Cuts) {
    for n in nodes.iter().filter(|n| n.exported(nature)) {
        cuts.add(n.x);
        cuts.add(n.x + n.w);
        if let Some((_, children)) = n.children() {
            add_x_cuts(children, nature, cuts);
        }
    }
}

impl Cuts {
    /// Column cuts before any page: the left edge.
    pub fn columns() -> Cuts {
        Cuts::with(&[0])
    }

    /// Once every page has added its edges: the right edge of the page, if nothing reaches it.
    pub fn close(&mut self, width: i32) {
        if self.offsets.last().is_some_and(|&l| l < width) {
            self.add(width);
        }
    }
}

/// The column cuts shared by all pages: page edges and element edges on every page.
pub(crate) fn x_cuts(pages: &[Vec<Node<'_>>], width: i32, nature: Nature) -> Cuts {
    let mut cuts = Cuts::columns();
    for p in pages {
        add_x_cuts(p, nature, &mut cuts);
    }
    cuts.close(width);
    cuts
}

/// Borders of a cell: top, left, bottom, right.
pub(crate) type Sides = [Pen; 4];

pub(crate) const NO_PEN: Pen = Pen { width: 0.0, color: Color::BLACK, style: LineStyle::Solid };

pub(crate) fn sides(b: &BoxStyle) -> Sides {
    [b.top, b.left, b.bottom, b.right]
}

/// The look of a grid cell before the exporter adds the element's own.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct CellStyle {
    pub back: Option<Color>,
    pub fore: Option<Color>,
    pub boxed: Option<Sides>,
}

#[derive(Debug, Clone)]
pub(crate) enum Cell<'a> {
    Empty(CellStyle),
    /// The cell at an element's top-left corner, spanning `rows` × `cols` cells.
    Element {
        node: &'a Node<'a>,
        rows: usize,
        cols: usize,
        style: CellStyle,
    },
    /// Covered by the element whose corner cell is at (row, column).
    Occupied(usize, usize),
}

impl Cell<'_> {
    fn is_empty(&self) -> bool {
        matches!(self, Cell::Empty(_))
    }
}

/// One page laid out on the grid.
pub(crate) struct PageGrid<'a> {
    pub rows: Cuts,
    pub cells: Vec<Vec<Cell<'a>>>,
}

/// Lays a page out on the shared column cuts (marking the columns elements start in).
pub(crate) fn layout<'a>(nodes: &'a [Node<'a>], xcuts: &mut Cuts, height: i32, nature: Nature) -> PageGrid<'a> {
    fn add_rows(nodes: &[Node<'_>], nature: Nature, cuts: &mut Cuts) {
        for n in nodes.iter().filter(|n| n.exported(nature)) {
            cuts.add(n.y);
            cuts.add(n.y + n.h);
            if let Some((_, children)) = n.children() {
                add_rows(children, nature, cuts);
            }
        }
    }
    let mut rows = Cuts::with(&[0, height]);
    add_rows(nodes, nature, &mut rows);
    let cells = vec![vec![Cell::Empty(CellStyle::default()); xcuts.spans()]; rows.spans()];
    let mut g = Layout { nature, xcuts, rows, cells };
    g.place_all(nodes);
    PageGrid { rows: g.rows, cells: g.cells }
}

struct Layout<'a, 'c> {
    nature: Nature,
    xcuts: &'c mut Cuts,
    rows: Cuts,
    cells: Vec<Vec<Cell<'a>>>,
}

impl<'a> Layout<'a, '_> {
    /// Places elements topmost first: an element overlapping one already placed is dropped.
    fn place_all(&mut self, nodes: &'a [Node<'a>]) {
        for n in nodes.iter().rev() {
            if !n.exported(self.nature) {
                continue;
            }
            let (c1, r1) = (self.xcuts.index(n.x), self.rows.index(n.y));
            let (c2, r2) = (self.xcuts.index(n.x + n.w), self.rows.index(n.y + n.h));
            if self.overlaps(r1, c1, r2, c2) {
                continue;
            }
            match n.children() {
                Some((frame, children)) => {
                    self.place_all(children);
                    self.frame_style(frame, r1, c1, r2, c2);
                }
                None => self.place(n, r1, c1, r2, c2),
            }
        }
    }

    fn overlaps(&self, r1: usize, c1: usize, r2: usize, c2: usize) -> bool {
        if self.nature.span {
            (r1..r2).any(|r| (c1..c2).any(|c| !self.cells[r][c].is_empty()))
        } else {
            self.cells.get(r1).and_then(|row| row.get(c1)).is_some_and(|c| !c.is_empty())
        }
    }

    fn place(&mut self, n: &'a Node<'a>, r1: usize, c1: usize, r2: usize, c2: usize) {
        if let Some(u) = self.rows.used.get_mut(r1) {
            *u = true;
        }
        if let Some(u) = self.xcuts.used.get_mut(c1) {
            *u = true;
        }
        // Nothing to place for an element without area (a negative size counts as none).
        if c2 <= c1 || r2 <= r1 {
            return;
        }
        let style = match &n.what {
            What::Text(t) => CellStyle { back: t.back, fore: Some(t.color), boxed: Some(sides(&t.boxed)) },
            What::Shape(e, ..) => CellStyle { back: e.back, fore: Some(e.fore), boxed: e.boxed.as_ref().map(sides) },
        };
        let (rows, cols) = if self.nature.span { (r2 - r1, c2 - c1) } else { (1, 1) };
        if self.nature.span {
            for row in &mut self.cells[r1..r2] {
                for cell in &mut row[c1..c2] {
                    *cell = Cell::Occupied(r1, c1);
                }
            }
        }
        self.cells[r1][c1] = Cell::Element { node: n, rows, cols, style };
    }

    /// A frame's background, color and borders go to the cells inside it that have none.
    fn frame_style(&mut self, frame: &ElementInfo, r1: usize, c1: usize, r2: usize, c2: usize) {
        let frame_box = match &frame.boxed {
            Some(b) => sides(b),
            None => [Pen { color: frame.fore, ..NO_PEN }; 4],
        };
        for (r, row) in self.cells.iter_mut().enumerate().take(r2).skip(r1) {
            for (c, cell) in row.iter_mut().enumerate().take(c2).skip(c1) {
                let (style, rows, cols) = match cell {
                    Cell::Empty(s) => (s, 1, 1),
                    Cell::Element { style, rows, cols, .. } => (style, *rows, *cols),
                    Cell::Occupied(..) => continue,
                };
                if style.back.is_none() {
                    style.back = frame.back;
                }
                if style.fore.is_none() {
                    style.fore = Some(frame.fore);
                }
                // Cells on the frame's edges take its borders there (`JRBoxUtil
                // .copyBordersNoPadding`); their other sides keep the cell's own borders, or
                // none in the frame's colors.
                let edges = [r == r1, c == c1, r + rows == r2, c + cols == c2];
                if edges.iter().any(|&e| e) {
                    let own = style.boxed;
                    let mut b = frame_box;
                    for k in 0..4 {
                        if !edges[k] || frame_box[k].width <= 0.0 {
                            b[k] = match own {
                                Some(o) => o[k],
                                None => Pen { width: 0.0, ..frame_box[k] },
                            };
                        }
                    }
                    style.boxed = Some(b);
                }
            }
        }
    }
}
