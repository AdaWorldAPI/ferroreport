//! Category plots (`CategoryPlot`): bar, stacked bar, line, area and stacked area charts as
//! JFreeChart 1.5 draws them for JasperReports 7.

use super::jfree::{CategoryAxis, LegendGraphic, NumberAxis, Rect, Side};
use super::{series_shape, Canvas, ChartElement, ChartKind, ChartTexts, DataState};
use crate::format::Locale;
use crate::model::Color;
use crate::print::PathOp;

/// JFreeChart's default grid lines: light gray, half a point, dashed 2 on 2.
pub(crate) const GRID: Color = Color { r: 0xC0, g: 0xC0, b: 0xC0 };
/// JFreeChart's default outline color (legend shapes of area charts).
pub(crate) const OUTLINE: Color = Color { r: 0x80, g: 0x80, b: 0x80 };

/// The legend graphic of a category chart's series.
pub(crate) fn legend_graphic(ch: &ChartElement, series: usize, color: Color) -> LegendGraphic {
    match ch.kind {
        ChartKind::Area | ChartKind::StackedArea => LegendGraphic::square(color, Some((OUTLINE, 1.0))),
        ChartKind::Line => {
            let shape = ch.show_shapes.then(|| {
                let (ops, bounds) = series_shape(series);
                (ops, bounds, Some((color, 1.0)))
            });
            LegendGraphic { shape, line: ch.show_lines.then_some(1.0), color }
        }
        _ => LegendGraphic::square(color, None),
    }
}

/// A grid line across `data` at `p` (dashed, butt ends).
pub(crate) fn grid_line(cv: &mut Canvas, data: Rect, p: f64, vertical_axis: bool) {
    let ops = if vertical_axis {
        vec![PathOp::Move(data.x, p), PathOp::Line(data.max_x(), p)]
    } else {
        vec![PathOp::Move(p, data.y), PathOp::Line(p, data.max_y())]
    };
    cv.path(ops, None, Some((GRID, 0.5)), Some([2.0, 2.0]));
}

/// Draws a category plot in `area` (what the titles and legend leave).
pub(crate) fn draw(
    cv: &mut Canvas,
    ch: &ChartElement,
    data: &DataState,
    texts: &ChartTexts,
    locale: &Locale,
    area: Rect,
    color_of: &dyn Fn(usize) -> Color,
) {
    let font = ch.font.clone();
    let horizontal = ch.horizontal;
    // The range axis is on top when the bars lie (AxisLocation.TOP_OR_LEFT).
    let (dom_side, rng_side) = if horizontal { (Side::Left, Side::Top) } else { (Side::Bottom, Side::Left) };
    let ncat = data.categories.len();
    let nser = data.series.len();
    let value = |s: usize, c: usize| data.values.get(&(s, c)).copied();
    let stacked = matches!(ch.kind, ChartKind::StackedBar | ChartKind::StackedBar3D | ChartKind::StackedArea);

    // The range of the values (each renderer's findRangeBounds).
    let (mut vmin, mut vmax) = (f64::INFINITY, f64::NEG_INFINITY);
    if stacked {
        for c in 0..ncat {
            let (mut pos, mut neg) = (0.0f64, 0.0f64);
            for s in 0..nser {
                if let Some(v) = value(s, c) {
                    if v > 0.0 {
                        pos += v;
                    } else {
                        neg += v;
                    }
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
        if matches!(ch.kind, ChartKind::Bar | ChartKind::Bar3D) && vmin.is_finite() {
            vmin = vmin.min(0.0);
            vmax = vmax.max(0.0);
        }
    }
    let (lo, hi) = NumberAxis::auto_range(vmin, vmax, true);
    let mut vaxis = NumberAxis::new(lo, hi, font.clone());
    vaxis.fix(texts.value_min, texts.value_max);
    vaxis.format = ch.value_format.clone();
    vaxis.label = texts.value_label.clone();
    if let Some(c) = ch.axis_color {
        (vaxis.line_color, vaxis.tick_label_color, vaxis.label_color) = (c, c, c);
    }
    let mut caxis = CategoryAxis::new(data.categories.clone(), font.clone());
    caxis.label = texts.category_label.clone();
    if matches!(ch.kind, ChartKind::Area | ChartKind::StackedArea) {
        caxis.category_margin = 0.0;
    }

    // CategoryPlot.draw: insets, room for the axes, the data area in whole points.
    let plot = area.trim(4.0, 8.0, 4.0, 8.0);
    let dom_space = caxis.reserve(cv, plot, dom_side);
    let rng_space = vaxis.reserve(cv, plot, rng_side, locale);
    let shrink = |r: Rect, side: Side, d: f64| match side {
        Side::Top => r.trim(d, 0.0, 0.0, 0.0),
        Side::Bottom => r.trim(0.0, 0.0, d, 0.0),
        Side::Left => r.trim(0.0, d, 0.0, 0.0),
        Side::Right => r.trim(0.0, 0.0, 0.0, d),
    };
    let data_area = shrink(shrink(plot, dom_side, dom_space), rng_side, rng_space).integerise();
    if data_area.w <= 0.0 || data_area.h <= 0.0 {
        return;
    }
    if let Some(bg) = ch.plot_backcolor {
        cv.fill_rect(data_area, bg);
    }
    // Axes: those at the bottom before those on the left.
    let cursor = |side: Side| match side {
        Side::Top => data_area.y,
        Side::Bottom => data_area.max_y(),
        Side::Left => data_area.x,
        Side::Right => data_area.max_x(),
    };
    let ticks = if horizontal {
        // Top, then left.
        let t = vaxis.draw(cv, cursor(rng_side), plot, data_area, rng_side, locale);
        caxis.draw(cv, cursor(dom_side), plot, data_area, dom_side);
        t
    } else {
        caxis.draw(cv, cursor(dom_side), plot, data_area, dom_side);
        vaxis.draw(cv, cursor(rng_side), plot, data_area, rng_side, locale)
    };
    // What follows is clipped to the data area.
    cv.clipped(data_area, |cv| {
        for (v, _) in &ticks {
            let p = vaxis.java2d(*v, data_area, rng_side);
            grid_line(cv, data_area, p, !horizontal);
        }
        items(cv, ch, data, &caxis, &vaxis, data_area, dom_side, rng_side, color_of);
    });
}

/// The renderer's items, category by category, series by series.
#[allow(clippy::too_many_arguments)]
fn items(
    cv: &mut Canvas,
    ch: &ChartElement,
    data: &DataState,
    caxis: &CategoryAxis,
    vaxis: &NumberAxis,
    data_area: Rect,
    dom_side: Side,
    rng_side: Side,
    color_of: &dyn Fn(usize) -> Color,
) {
    let horizontal = ch.horizontal;
    let ncat = data.categories.len();
    let nser = data.series.len();
    let value = |s: usize, c: usize| data.values.get(&(s, c)).copied();
    let pos = |v: f64| vaxis.java2d(v, data_area, rng_side);
    let rect = |w0: f64, w: f64, l0: f64, l1: f64| {
        let (a, b) = (l0.min(l1), (l1 - l0).abs());
        if horizontal {
            Rect::new(a, w0, b, w)
        } else {
            Rect::new(w0, a, w, b)
        }
    };
    let (lower_clip, upper_clip) = (vaxis.lower, vaxis.upper);
    match ch.kind {
        ChartKind::Bar | ChartKind::Bar3D => {
            // BarRenderer: bars of equal width, 20% of the room between the series.
            let space = if horizontal { data_area.h } else { data_area.w };
            let cat_margin = if ncat > 1 { caxis.category_margin } else { 0.0 };
            let item_margin = if nser > 1 { 0.2 } else { 0.0 };
            let used = space * (1.0 - caxis.lower_margin - caxis.upper_margin - cat_margin - item_margin);
            let bar_w = if nser * ncat > 0 { (used / (nser * ncat) as f64).min(space) } else { 0.0 };
            for c in 0..ncat {
                for s in 0..nser {
                    let Some(v) = value(s, c) else { continue };
                    let w0 = if nser > 1 {
                        let gap = space * 0.2 / (ncat * (nser - 1)) as f64;
                        let series_w =
                            space * (1.0 - caxis.lower_margin - caxis.upper_margin - cat_margin - 0.2) / (ncat * nser) as f64;
                        caxis.start(c, ncat, data_area, dom_side) + s as f64 * (series_w + gap) + series_w / 2.0 - bar_w / 2.0
                    } else {
                        caxis.middle(c, ncat, data_area, dom_side) - bar_w / 2.0
                    };
                    let (low, high) = (v.min(0.0), v.max(0.0));
                    if high < lower_clip || low > upper_clip {
                        continue;
                    }
                    let (low, high) = (low.max(lower_clip), high.min(upper_clip));
                    cv.fill_rect(rect(w0, bar_w, pos(low), pos(high)), color_of(s));
                }
            }
        }
        ChartKind::StackedBar | ChartKind::StackedBar3D => {
            // StackedBarRenderer: one bar per category, the series on top of each other.
            let space = if horizontal { data_area.h } else { data_area.w };
            let cat_margin = if ncat > 1 { caxis.category_margin } else { 0.0 };
            let used = space * (1.0 - caxis.lower_margin - caxis.upper_margin - cat_margin);
            let bar_w = if ncat > 0 { used / ncat as f64 } else { 0.0 };
            for c in 0..ncat {
                let (mut pos_base, mut neg_base) = (0.0f64, 0.0f64);
                for s in 0..nser {
                    let Some(v) = value(s, c) else { continue };
                    let w0 = caxis.middle(c, ncat, data_area, dom_side) - bar_w / 2.0;
                    let (l0, l1) = if v > 0.0 {
                        let r = (pos_base, pos_base + v);
                        pos_base += v;
                        r
                    } else {
                        let r = (neg_base + v, neg_base);
                        neg_base += v;
                        r
                    };
                    let (l0, l1) = (l0.max(lower_clip), l1.min(upper_clip));
                    if l1 < l0 {
                        continue;
                    }
                    cv.fill_rect(rect(w0, bar_w, pos(l0), pos(l1)), color_of(s));
                }
            }
        }
        ChartKind::Line => {
            let point = |c: usize, v: f64| {
                let (a, b) = (caxis.middle(c, ncat, data_area, dom_side), pos(v));
                if horizontal {
                    (b, a)
                } else {
                    (a, b)
                }
            };
            // Pass 0: a line from each item to the one before it.
            if ch.show_lines {
                for c in 1..ncat {
                    for s in 0..nser {
                        let (Some(v0), Some(v1)) = (value(s, c - 1), value(s, c)) else { continue };
                        let (p0, p1) = (point(c - 1, v0), point(c, v1));
                        cv.line_square(p0.0, p0.1, p1.0, p1.1, color_of(s), 1.0);
                    }
                }
            }
            // Pass 1: the shapes, filled and outlined in the series color.
            if ch.show_shapes {
                for c in 0..ncat {
                    for s in 0..nser {
                        let Some(v) = value(s, c) else { continue };
                        let (x, y) = point(c, v);
                        let (ops, _) = series_shape(s);
                        let ops = super::jfree::translate(&ops, x, y);
                        cv.path(ops.clone(), Some(color_of(s)), None, None);
                        cv.stroke_closed(ops, color_of(s), 1.0);
                    }
                }
            }
        }
        ChartKind::Area => {
            // AreaRenderer: each item from the middle of its category to half way to its
            // neighbors, at whole points, tapering to zero at both ends.
            let zero = pos(0.0);
            for c in 0..ncat {
                for s in 0..nser {
                    let Some(y1) = value(s, c) else { continue };
                    let round = |v: f64| (v as f32 + 0.5).floor() as f64;
                    let x0 = round(caxis.start(c, ncat, data_area, dom_side));
                    let x1 = round(caxis.middle(c, ncat, data_area, dom_side));
                    let x2 = round(caxis.end(c, ncat, data_area, dom_side));
                    let y0 = if c > 0 { value(s, c - 1).map(|v| (v + y1) / 2.0).unwrap_or(0.0) } else { 0.0 };
                    let y2 = if c + 1 < ncat { value(s, c + 1).map(|v| (v + y1) / 2.0).unwrap_or(0.0) } else { 0.0 };
                    let (t0, t1, t2) = (pos(y0) as f32 as f64, pos(y1) as f32 as f64, pos(y2) as f32 as f64);
                    let pts = [(x0, zero), (x0, t0), (x1, t1), (x2, t2), (x2, zero)];
                    let pts: Vec<(f64, f64)> = pts.iter().map(|&(a, b)| if horizontal { (b, a) } else { (a, b) }).collect();
                    cv.fill_poly(&pts, color_of(s));
                }
            }
        }
        ChartKind::StackedArea => {
            // StackedAreaRenderer: the left and right halves of each item, on top of the
            // series before it.
            let stack = |s: usize, c: usize| -> (f64, f64) {
                let (mut neg, mut posv) = (0.0, 0.0);
                for i in 0..s {
                    if let Some(v) = value(i, c) {
                        if v >= 0.0 {
                            posv += v;
                        } else {
                            neg += v;
                        }
                    }
                }
                (neg, posv)
            };
            let adjusted = |a: f64, b: f64| if a == 0.0 || b == 0.0 { 0.0 } else { (a + b) / 2.0 };
            for c in 0..ncat {
                for s in 0..nser {
                    let y1 = value(s, c).unwrap_or(0.0);
                    let cp = c.saturating_sub(1);
                    let cn = (c + 1).min(ncat - 1);
                    let y0 = value(s, cp).unwrap_or(0.0);
                    let y2 = value(s, cn).unwrap_or(0.0);
                    let (st0, st1, st2) = (stack(s, cp), stack(s, c), stack(s, cn));
                    let xx1 = caxis.middle(c, ncat, data_area, dom_side) as f32 as f64;
                    let xl = caxis.start(c, ncat, data_area, dom_side) as f32 as f64;
                    let xr = caxis.end(c, ncat, data_area, dom_side) as f32 as f64;
                    let f = |v: f64| pos(v) as f32 as f64;
                    let flip = |pts: Vec<(f64, f64)>| -> Vec<(f64, f64)> {
                        if horizontal {
                            pts.into_iter().map(|(a, b)| (b, a)).collect()
                        } else {
                            pts
                        }
                    };
                    let (left, right);
                    if y1 >= 0.0 {
                        let t1 = f(y1 + st1.1);
                        let ts1 = f(st1.1);
                        let tsl = f(adjusted(st0.1, st1.1));
                        left = if y0 >= 0.0 {
                            let yl = (y0 + y1) / 2.0 + (st0.1 + st1.1) / 2.0;
                            vec![(xx1, t1), (xx1, ts1), (xl, tsl), (xl, f(yl))]
                        } else {
                            vec![(xx1, t1), (xx1, ts1), (xl, tsl)]
                        };
                        let tsr = f(adjusted(st1.1, st2.1));
                        right = if y2 >= 0.0 {
                            let yr = (y1 + y2) / 2.0 + (st1.1 + st2.1) / 2.0;
                            vec![(xx1, ts1), (xx1, t1), (xr, f(yr)), (xr, tsr)]
                        } else {
                            vec![(xx1, ts1), (xx1, t1), (xr, tsr)]
                        };
                    } else {
                        let t1 = f(y1 + st1.0);
                        let ts1 = f(st1.0);
                        let tsl = f(adjusted(st0.0, st1.0));
                        left = if y0 >= 0.0 {
                            vec![(xx1, t1), (xx1, ts1), (xl, tsl)]
                        } else {
                            let yl = (y0 + y1) / 2.0 + (st0.0 + st1.0) / 2.0;
                            vec![(xx1, t1), (xx1, ts1), (xl, tsl), (xl, f(yl))]
                        };
                        let tsr = f(adjusted(st1.0, st2.0));
                        right = if y2 >= 0.0 {
                            vec![(xx1, ts1), (xx1, t1), (xr, tsr)]
                        } else {
                            let yr = (y1 + y2) / 2.0 + (st1.0 + st2.0) / 2.0;
                            vec![(xx1, ts1), (xx1, t1), (xr, f(yr)), (xr, tsr)]
                        };
                    }
                    cv.fill_poly(&flip(left), color_of(s));
                    cv.fill_poly(&flip(right), color_of(s));
                }
            }
        }
        _ => {}
    }
}
