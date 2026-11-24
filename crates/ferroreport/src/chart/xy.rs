//! XY plots (`XYPlot`): XY line, scatter, time series, XY area, XY bar, bubble, candlestick and
//! high-low charts as JFreeChart 1.5 draws them for JasperReports 7.

use super::category::{grid_line, OUTLINE};
use super::jfree::{translate, DateAxis, LegendGraphic, NumberAxis, Rect, Side, ValueAxis};
use super::{arc, series_shape, Canvas, ChartElement, ChartKind, ChartTexts, DataKind, DataState};
use crate::format::Locale;
use crate::model::Color;
use crate::print::PathOp;
use std::f64::consts::PI;

/// The dataset of an XY chart, in the shape its renderer reads it.
struct Series {
    /// (x, y) of each item, x ascending.
    points: Vec<(f64, f64)>,
    /// The x interval of each item (bars).
    intervals: Vec<(f64, f64)>,
    /// Bubble sizes.
    z: Vec<f64>,
}

/// The volume bars of candlestick charts.
const VOLUME: Color = Color { r: 0x80, g: 0x80, b: 0x80 };

/// Whether the domain axis shows dates.
fn dates(ch: &ChartElement) -> bool {
    matches!(ch.dataset.kind, DataKind::TimeSeries | DataKind::TimePeriod | DataKind::HighLow)
}

fn series_of(ch: &ChartElement, data: &DataState) -> Vec<Series> {
    let sorted = |mut v: Vec<(f64, f64)>| {
        v.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        v
    };
    match ch.dataset.kind {
        DataKind::Xy => {
            // XYSeriesCollection: bars as wide as the smallest step between x values.
            let all: Vec<Vec<(f64, f64)>> = data.xy.iter().map(|(_, p)| sorted(p.clone())).collect();
            let mut width = f64::INFINITY;
            for p in &all {
                for w in p.windows(2) {
                    width = width.min(w[1].0 - w[0].0);
                }
            }
            let width = if width.is_finite() { width } else { 1.0 };
            all.into_iter()
                .map(|points| Series {
                    intervals: points.iter().map(|p| (p.0 - width / 2.0, p.0 + width / 2.0)).collect(),
                    points,
                    z: Vec::new(),
                })
                .collect()
        }
        DataKind::TimeSeries | DataKind::TimePeriod => data
            .periods
            .iter()
            .map(|(_, p)| {
                let mut p = p.clone();
                p.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
                // Time series are plotted at the start of their periods, time periods in the middle.
                let at = |a: f64, b: f64| if ch.dataset.kind == DataKind::TimeSeries { a } else { (a + b) / 2.0 };
                Series {
                    points: p.iter().map(|&(a, b, v)| (at(a, b), v)).collect(),
                    intervals: p.iter().map(|&(a, b, _)| (a, b)).collect(),
                    z: Vec::new(),
                }
            })
            .collect(),
        DataKind::Xyz => data
            .xyz
            .iter()
            .map(|(_, p)| Series {
                points: p.iter().map(|&(x, y, _)| (x, y)).collect(),
                intervals: Vec::new(),
                z: p.iter().map(|&(_, _, z)| z).collect(),
            })
            .collect(),
        DataKind::HighLow => data
            .ohlc
            .iter()
            .map(|(_, items)| Series {
                points: items.iter().map(|it| (it[0], f64::NAN)).collect(),
                intervals: Vec::new(),
                z: Vec::new(),
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// The legend graphic of an XY chart's series.
pub(crate) fn legend_graphic(ch: &ChartElement, series: usize, color: Color) -> LegendGraphic {
    match ch.kind {
        ChartKind::XyArea => {
            let ops = vec![
                PathOp::Move(0.0, -4.0),
                PathOp::Line(3.0, -2.0),
                PathOp::Line(4.0, 4.0),
                PathOp::Line(-4.0, 4.0),
                PathOp::Line(-3.0, -2.0),
                PathOp::Close,
            ];
            LegendGraphic { shape: Some((ops, (-4.0, -4.0, 8.0, 8.0), None)), line: None, color }
        }
        ChartKind::XyBar => LegendGraphic {
            shape: Some((super::jfree::rect_ops(-3.0, -5.0, 6.0, 10.0), (-3.0, -5.0, 6.0, 10.0), None)),
            line: None,
            color,
        },
        ChartKind::Candlestick | ChartKind::HighLow => LegendGraphic::square(color, None),
        ChartKind::Bubble => LegendGraphic {
            shape: Some((circle(0.0, 0.0, 4.0, 4.0), (-4.0, -4.0, 8.0, 8.0), Some((OUTLINE, 1.0)))),
            line: None,
            color,
        },
        _ => {
            let shape = ch.show_shapes.then(|| {
                let (ops, bounds) = series_shape(series);
                (ops, bounds, Some((color, 1.0)))
            });
            LegendGraphic { shape, line: ch.show_lines.then_some(1.0), color }
        }
    }
}

/// An ellipse around `(cx, cy)` as Java draws one (from three o'clock).
fn circle(cx: f64, cy: f64, rx: f64, ry: f64) -> Vec<PathOp> {
    let mut ops = Vec::new();
    arc(&mut ops, cx, cy, rx, ry, 0.0, 2.0 * PI, true);
    ops.push(PathOp::Close);
    ops
}

/// Draws an XY plot in `area` (what the titles and legend leave).
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
    let series = series_of(ch, data);
    let bars = ch.kind == ChartKind::XyBar;

    // Data ranges (each renderer's findDomainBounds and findRangeBounds).
    let (mut x0, mut x1, mut y0, mut y1) = (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
    for s in &series {
        if bars {
            for &(a, b) in &s.intervals {
                x0 = x0.min(a.min(b));
                x1 = x1.max(a.max(b));
            }
        } else {
            for &(x, _) in &s.points {
                x0 = x0.min(x);
                x1 = x1.max(x);
            }
        }
        for &(_, y) in &s.points {
            if y.is_finite() {
                y0 = y0.min(y);
                y1 = y1.max(y);
            }
        }
    }
    // OHLC data: from the lowest low to the highest high.
    for (_, items) in &data.ohlc {
        for it in items {
            for v in [it[1], it[2]] {
                if v.is_finite() {
                    y0 = y0.min(v);
                    y1 = y1.max(v);
                }
            }
        }
    }
    let mut daxis = if dates(ch) {
        let margin = if ch.kind == ChartKind::TimeSeries { 0.02 } else { 0.05 };
        let (lo, hi) = DateAxis::auto_range(x0, x1, margin);
        let mut a = DateAxis::new(lo, hi, font.clone());
        a.format = ch.time_format.clone();
        a.label = texts.category_label.clone();
        ValueAxis::Date(a)
    } else {
        let (lo, hi) = NumberAxis::auto_range(x0, x1, false);
        let mut a = NumberAxis::new(lo, hi, font.clone());
        a.format = ch.x_format.clone();
        a.label = texts.category_label.clone();
        ValueAxis::Number(a)
    };
    let include_zero =
        matches!(ch.kind, ChartKind::XyLine | ChartKind::XyArea | ChartKind::XyBar | ChartKind::Candlestick | ChartKind::HighLow);
    let (lo, hi) = NumberAxis::auto_range(y0, y1, include_zero);
    let mut raxis = NumberAxis::new(lo, hi, font.clone());
    raxis.fix(texts.value_min, texts.value_max);
    raxis.format = ch.value_format.clone();
    raxis.label = texts.value_label.clone();
    if let Some(c) = ch.axis_color {
        (raxis.line_color, raxis.tick_label_color, raxis.label_color) = (c, c, c);
    }
    let mut raxis = ValueAxis::Number(raxis);

    // XYPlot.draw: insets, room for the range axis, then for the domain axis in what is left,
    // the data area in whole points.
    let (dom_side, rng_side) = if ch.horizontal { (Side::Left, Side::Bottom) } else { (Side::Bottom, Side::Left) };
    let plot = area.trim(4.0, 8.0, 4.0, 8.0);
    let shrink = |r: Rect, side: Side, d: f64| match side {
        Side::Top => r.trim(d, 0.0, 0.0, 0.0),
        Side::Bottom => r.trim(0.0, 0.0, d, 0.0),
        Side::Left => r.trim(0.0, d, 0.0, 0.0),
        Side::Right => r.trim(0.0, 0.0, 0.0, d),
    };
    let rng_space = raxis.reserve(cv, plot, rng_side, locale);
    let dom_space = daxis.reserve(cv, shrink(plot, rng_side, rng_space), dom_side, locale);
    let data_area = shrink(shrink(plot, dom_side, dom_space), rng_side, rng_space).integerise();
    if data_area.w <= 0.0 || data_area.h <= 0.0 {
        return;
    }
    if let Some(bg) = ch.plot_backcolor {
        cv.fill_rect(data_area, bg);
    }
    let cursor = |side: Side| match side {
        Side::Top => data_area.y,
        Side::Bottom => data_area.max_y(),
        Side::Left => data_area.x,
        Side::Right => data_area.max_x(),
    };
    // Axes: those at the bottom before those on the left.
    let (dticks, rticks) = if ch.horizontal {
        let r = raxis.draw(cv, cursor(rng_side), plot, data_area, rng_side, locale);
        let d = daxis.draw(cv, cursor(dom_side), plot, data_area, dom_side, locale);
        (d, r)
    } else {
        let d = daxis.draw(cv, cursor(dom_side), plot, data_area, dom_side, locale);
        let r = raxis.draw(cv, cursor(rng_side), plot, data_area, rng_side, locale);
        (d, r)
    };
    let xpos = |v: f64| daxis.java2d(v, data_area, dom_side);
    let ypos = |v: f64| raxis.java2d(v, data_area, rng_side);
    let point = |x: f64, y: f64| if ch.horizontal { (ypos(y), xpos(x)) } else { (xpos(x), ypos(y)) };
    cv.clipped(data_area, |cv| {
        for (v, _) in &dticks {
            grid_line(cv, data_area, xpos(*v), ch.horizontal);
        }
        for (v, _) in &rticks {
            grid_line(cv, data_area, ypos(*v), !ch.horizontal);
        }
        // Series last to first (SeriesRenderingOrder.REVERSE), pass by pass.
        let order: Vec<usize> = (0..series.len()).rev().collect();
        match ch.kind {
            ChartKind::XyArea => {
                let zero = ypos(0.0);
                for &s in &order {
                    let p = &series[s].points;
                    if p.is_empty() {
                        continue;
                    }
                    let f = |v: f64| v as f32 as f64;
                    let mut pts = Vec::new();
                    let first = point(p[0].0, 0.0);
                    pts.push(if ch.horizontal { (f(zero), f(first.1)) } else { (f(first.0), f(zero)) });
                    for &(x, y) in p {
                        let q = point(x, y);
                        pts.push((f(q.0), f(q.1)));
                    }
                    let last = point(p[p.len() - 1].0, 0.0);
                    pts.push(if ch.horizontal { (f(zero), f(last.1)) } else { (f(last.0), f(zero)) });
                    cv.fill_poly(&pts, color_of(s));
                }
            }
            ChartKind::XyBar => {
                let (lo, hi) = raxis.bounds();
                for &s in &order {
                    for (&(_, y), &(a, b)) in series[s].points.iter().zip(&series[s].intervals) {
                        if !y.is_finite() || 0.0f64.max(y) < lo || 0.0f64.min(y) > hi {
                            continue;
                        }
                        let (t0, t1) = (ypos(0.0), ypos(y));
                        let (bottom, top) = (t0.min(t1), t0.max(t1));
                        let (xa, xb) = (xpos(a), xpos(b));
                        let width = (xb - xa).abs().max(1.0);
                        let left = xa.min(xb);
                        let r = if ch.horizontal {
                            Rect::new(bottom, left, top - bottom, width)
                        } else {
                            Rect::new(left, bottom, width, top - bottom)
                        };
                        cv.fill_rect(r, color_of(s));
                    }
                }
            }
            ChartKind::Candlestick => {
                let volume_max =
                    data.ohlc.iter().flat_map(|(_, i)| i.iter().map(|it| it[5])).fold(0.0f64, |a, v| if v > a { v } else { a });
                for &s in &order {
                    let items = &data.ohlc[s].1;
                    // WIDTHMETHOD_AVERAGE: the plot shared among the items, 4.5/7 of it, at most
                    // 20 hours wide.
                    let max_width = (xpos(20.0 * 3_600_000.0) - xpos(0.0)).abs();
                    let avg = if ch.horizontal { data_area.h } else { data_area.w } / items.len().max(1) as f64;
                    let width = (avg * (4.5 / 7.0)).min(max_width);
                    let volume_w = width.max(1.0f64.min(max_width));
                    let stick_w = width.max(3.0f64.min(max_width));
                    let color = color_of(s);
                    for it in items {
                        let [t, high, low, open, close, volume] = *it;
                        let xx = xpos(t);
                        if ch.show_volume && volume_max > 0.0 && volume.is_finite() {
                            // The volume: gray at 30% opacity; drawing SVG, Batik paints it as an
                            // opaque bitmap of a pixel a point.
                            let h = (volume.trunc() / volume_max) * data_area.h;
                            let r = Rect::new(xx - volume_w / 2.0, data_area.max_y() - h, volume_w, h);
                            if ch.svg {
                                // Java2D clips the bitmap's pixels to the data area.
                                let (x0, y0) = (r.x.max(data_area.x), r.y.max(data_area.y));
                                let (x1, y1) = (r.max_x().min(data_area.max_x()), r.max_y().min(data_area.max_y()));
                                if x1 > x0 && y1 > y0 {
                                    super::bitmap::fill_rect_bitmap(cv, Rect::new(x0, y0, x1 - x0, y1 - y0), VOLUME);
                                }
                            } else {
                                cv.fill_rect(r, super::blend(VOLUME, 0.3, ch.plot_backcolor.unwrap_or(Color::WHITE)));
                            }
                        }
                        let (yh, yl, yo, yc) = (ypos(high), ypos(low), ypos(open), ypos(close));
                        let (ymax, ymin) = (yo.max(yc), yo.min(yc));
                        if high > open.max(close) {
                            cv.line_square(xx, yh, xx, ymax, color, 1.0);
                        }
                        if low < open.min(close) {
                            cv.line_square(xx, yl, xx, ymin, color, 1.0);
                        }
                        let body = Rect::new(xx - stick_w / 2.0, ymin, stick_w, ymax - ymin);
                        let fill = if close > open { Color { r: 0, g: 255, b: 0 } } else { Color { r: 255, g: 0, b: 0 } };
                        cv.fill_rect(body, fill);
                        cv.stroke_closed(super::jfree::rect_ops(body.x, body.y, body.w, body.h), color, 1.0);
                    }
                }
            }
            ChartKind::HighLow => {
                for &s in &order {
                    for it in &data.ohlc[s].1 {
                        let [t, high, low, open, close, _] = *it;
                        let xx = xpos(t);
                        let color = color_of(s);
                        if high.is_finite() && low.is_finite() {
                            cv.line_square(xx, ypos(low), xx, ypos(high), color, 1.0);
                        }
                        if ch.show_open_ticks && open.is_finite() {
                            cv.line_square(xx - 2.0, ypos(open), xx, ypos(open), color, 1.0);
                        }
                        if ch.show_close_ticks && close.is_finite() {
                            cv.line_square(xx, ypos(close), xx + 2.0, ypos(close), color, 1.0);
                        }
                    }
                }
            }
            ChartKind::Bubble => {
                // XYBubbleRenderer scaled on the range axis: a bubble is z of the range axis across.
                let zero = ypos(0.0);
                for &s in &order {
                    for (&(x, y), &z) in series[s].points.iter().zip(&series[s].z) {
                        let d = (zero - ypos(z)).abs();
                        let (cx, cy) = point(x, y);
                        let ops = circle(cx, cy, d / 2.0, d / 2.0);
                        cv.path(ops.clone(), Some(color_of(s)), None, None);
                        cv.stroke_closed(ops, OUTLINE, 1.0);
                    }
                }
            }
            _ => {
                // XYLineAndShapeRenderer: the lines, then the shapes.
                if ch.show_lines {
                    for &s in &order {
                        for w in series[s].points.windows(2) {
                            if !(w[0].1.is_finite() && w[1].1.is_finite()) {
                                continue;
                            }
                            let (a, b) = (point(w[0].0, w[0].1), point(w[1].0, w[1].1));
                            cv.line_square(a.0, a.1, b.0, b.1, color_of(s), 1.0);
                        }
                    }
                }
                if ch.show_shapes {
                    for &s in &order {
                        for &(x, y) in &series[s].points {
                            if !y.is_finite() {
                                continue;
                            }
                            let (px, py) = point(x, y);
                            let (ops, _) = series_shape(s);
                            let ops = translate(&ops, px, py);
                            cv.path(ops.clone(), Some(color_of(s)), None, None);
                            cv.stroke_closed(ops, color_of(s), 1.0);
                        }
                    }
                }
            }
        }
    });
}
