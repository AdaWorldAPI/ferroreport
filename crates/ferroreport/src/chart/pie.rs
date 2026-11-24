//! Pie charts (`PiePlot`) as JFreeChart 1.5 draws them for JasperReports 7, which draws 3D pie
//! charts as flat ones too.

use super::category::OUTLINE;
use super::jfree::{BlockAnchor, HAnchor, LegendGraphic, Rect, TextBlock, VAnchor};
use super::{arc, blend, pie_label, Canvas, ChartElement, DataState};
use crate::format::Locale;
use crate::model::Color;
use crate::print::PathOp;
use std::f64::consts::PI;

/// The shadow JFreeChart draws under the pie, 4 points right and down.
const SHADOW: Color = Color { r: 0x80, g: 0x80, b: 0x80 };
/// Label boxes' shadow: (151, 151, 151) at alpha 128, over white.
const LABEL_SHADOW: Color = Color { r: 203, g: 203, b: 203 };
/// The link lines where a label shadow covers them (black under that shadow).
const LINK_UNDER_SHADOW: Color = Color { r: 76, g: 76, b: 76 };

/// A circle of radius `r` around `(cx, cy)`, starting at three o'clock as Java's ellipses do.
fn circle(cx: f64, cy: f64, r: f64) -> Vec<PathOp> {
    let mut ops = Vec::new();
    arc(&mut ops, cx, cy, r, r, 0.0, 2.0 * PI, true);
    ops.push(PathOp::Close);
    ops
}

/// The legend graphic of a pie section: a circle 8 across, outlined in gray.
pub(crate) fn legend_graphic(color: Color) -> LegendGraphic {
    LegendGraphic { shape: Some((circle(0.0, 0.0, 4.0), (-4.0, -4.0, 8.0, 8.0), Some((OUTLINE, 0.5)))), line: None, color }
}

/// Which sections get a color, in order: those the legend lists (values not negative) when
/// there is a legend, otherwise those drawn (positive values).
pub(crate) fn color_index(ch: &ChartElement, data: &DataState, section: usize) -> usize {
    data.pie[..section].iter().filter(|(_, v)| if ch.show_legend { *v >= 0.0 } else { *v > 0.0 }).count()
}

/// The sections listed in the legend.
pub(crate) fn legend_sections(data: &DataState) -> Vec<usize> {
    (0..data.pie.len()).filter(|&i| data.pie[i].1 >= 0.0).collect()
}

/// A pie section (`Arc2D.PIE`) from `a1` to `a2` degrees (counter-clockwise from three
/// o'clock, as Java measures them).
fn wedge(cx: f64, cy: f64, r: f64, a1: f64, a2: f64) -> Vec<PathOp> {
    let mut ops = Vec::new();
    arc(&mut ops, cx, cy, r, r, -a1.to_radians(), -a2.to_radians(), true);
    ops.push(PathOp::Line(cx, cy));
    ops.push(PathOp::Close);
    ops
}

struct Label {
    angle: f64,
    base_y: f64,
    block: TextBlock,
    height: f64,
    gap: f64,
    y: f64,
}

/// PieLabelDistributor: labels moved apart so that they do not overlap (4 points apart).
fn distribute(labels: &mut [Label], min_y: f64, height: f64) {
    labels.sort_by(|a, b| a.base_y.partial_cmp(&b.base_y).unwrap_or(std::cmp::Ordering::Equal));
    for l in labels.iter_mut() {
        l.y = l.base_y;
    }
    let overlap = |labels: &[Label]| {
        let mut y = 0.0;
        for l in labels {
            if y > l.y - l.height / 2.0 {
                return true;
            }
            y = l.y + l.height / 2.0;
        }
        false
    };
    const GAP: f64 = 4.0;
    if overlap(labels) {
        for i in 0..labels.len().saturating_sub(1) {
            let (upper0, lower1, h1) =
                (labels[i].y + labels[i].height / 2.0, labels[i + 1].y - labels[i + 1].height / 2.0, labels[i + 1].height);
            if lower1 < upper0 {
                labels[i + 1].y = (min_y + height - h1 / 2.0).min(upper0 + GAP + h1 / 2.0);
            }
        }
    }
    if overlap(labels) {
        for i in (1..labels.len()).rev() {
            let (lower0, upper1, h1) =
                (labels[i].y - labels[i].height / 2.0, labels[i - 1].y + labels[i - 1].height / 2.0, labels[i - 1].height);
            if upper1 > lower0 {
                labels[i - 1].y = (min_y + h1 / 2.0).max(lower0 - GAP - h1 / 2.0);
            }
        }
    }
    if overlap(labels) {
        let total: f64 = labels.iter().map(|l| l.height).sum();
        let mut gap = height - total;
        if labels.len() > 1 {
            gap /= (labels.len() - 1) as f64;
        }
        let mut y = min_y;
        for l in labels.iter_mut() {
            y += l.height / 2.0;
            l.y = y;
            y += l.height / 2.0 + gap;
        }
    }
}

/// Draws a pie plot in `area` (what the titles and legend leave).
pub(crate) fn draw(
    cv: &mut Canvas,
    ch: &ChartElement,
    data: &DataState,
    locale: &Locale,
    area: Rect,
    color_of: &dyn Fn(usize) -> Color,
) {
    // PiePlot's insets.
    let plot = area.trim(0.0, 5.0, 5.0, 5.0);
    if let Some(bg) = ch.plot_backcolor {
        cv.fill_rect(plot, bg);
    }
    let total: f64 = data.pie.iter().map(|(_, v)| if *v > 0.0 { *v } else { 0.0 }).sum();
    if data.pie.is_empty() || total <= 0.0 {
        return;
    }
    let labels = ch.show_labels;
    // The link area: room for labels on both sides, 8% of the height above and below, square.
    let reserve = if labels { 0.025 + 0.14 } else { 0.0 };
    let (gap_h, gap_v) = (plot.w * reserve * 2.0, plot.h * 0.08 * 2.0);
    let (mut lx, mut ly, mut lw, mut lh) = (plot.x + gap_h / 2.0, plot.y + gap_v / 2.0, plot.w - gap_h, plot.h - gap_v);
    let min = lw.min(lh) / 2.0;
    lx = (lx + lx + lw) / 2.0 - min;
    ly = (ly + ly + lh) / 2.0 - min;
    lw = 2.0 * min;
    lh = 2.0 * min;
    let link = Rect::new(lx, ly, lw, lh);
    let (hh, vv) = (link.w * 0.025 * 2.0, link.h * 0.025 * 2.0);
    let pie_area = Rect::new(lx + hh / 2.0, ly + vv / 2.0, lw - hh, lh - vv);
    let (cx, cy) = (pie_area.center_x(), pie_area.center_y());
    let r = pie_area.w / 2.0;
    let rv = pie_area.h / 2.0;

    // Sections clockwise from twelve o'clock: their shadows first, then the sections.
    // Drawing SVG, Batik paints translucent groups as opaque bitmaps of a pixel a point, where
    // a thin outline is a point wide.
    let bitmaps = ch.svg && ch.foreground_alpha < 1.0;
    let alpha = if ch.svg { 1.0 } else { ch.foreground_alpha.clamp(0.0, 1.0) };
    let opaque = alpha >= 1.0;
    let back = ch.plot_backcolor.or(ch.backcolor).unwrap_or(Color::WHITE);
    let shadow = blend(SHADOW, alpha, back);
    let mut sections = Vec::new();
    let mut angle = 90.0f64;
    for (i, (_, v)) in data.pie.iter().enumerate() {
        if *v > 0.0 {
            let a2 = angle - v / total * 360.0;
            if (a2 - angle).abs() > 1.0e-5 {
                sections.push((i, angle, a2));
            }
            angle = a2;
        }
    }
    if bitmaps {
        // Batik paints each translucent shape as an opaque bitmap.
        for &(_, a1, a2) in &sections {
            super::bitmap::fill_bitmap(cv, &wedge(cx + 4.0, cy + 4.0, r, a1, a2), SHADOW);
        }
        for &(i, a1, a2) in &sections {
            let ops = wedge(cx, cy, r, a1, a2);
            super::bitmap::fill_bitmap(cv, &ops, color_of(i));
            super::bitmap::stroke_bitmap(cv, &ops, OUTLINE, 0.5);
        }
    }
    for &(_, a1, a2) in sections.iter().filter(|_| !bitmaps) {
        cv.path(wedge(cx + 4.0, cy + 4.0, r, a1, a2), Some(shadow), None, None);
    }
    for &(i, a1, a2) in sections.iter().filter(|_| !bitmaps) {
        let color = color_of(i);
        let ops = wedge(cx, cy, r, a1, a2);
        let fill = blend(color, alpha, back);
        cv.path(ops.clone(), Some(fill), None, None);
        if !opaque {
            // Where the section is over the shadow, it shows the shadow through.
            let over = blend(color, alpha, shadow);
            cv.clipped_path(circle(cx + 4.0, cy + 4.0, r), |cv| cv.path(ops.clone(), Some(over), None, None));
        }
        cv.stroke_closed(ops, blend(OUTLINE, alpha, fill), 0.5);
    }

    if !labels {
        return;
    }
    // Labels, left of the pie for sections whose middle is left of it, right of it otherwise.
    let font = ch.font.clone();
    let label_back = Color::WHITE;
    let (mut left, mut right) = (Vec::new(), Vec::new());
    let mut running = 0.0;
    let lgap = plot.w * 0.025;
    let max_w = ((link.x - lgap - plot.x) - 4.0) as f32 as f64;
    let h_text = cv.metrics(&font).height;
    for (key, v) in &data.pie {
        if *v < 0.0 {
            continue;
        }
        running += v;
        let mid = 90.0 - (running - v / 2.0) * 360.0 / total;
        let theta = mid.to_radians();
        let text = match &ch.label_format {
            Some(f) => pie_label(f, key, *v, total, locale),
            None => key.clone(),
        };
        let block = TextBlock::wrap(cv, &font, &text, max_w, usize::MAX);
        let height = block.lines.len() as f64 * h_text + 4.0;
        let base_y = cy - theta.sin() * (link.h / 2.0);
        let on_left = theta.cos() < 0.0;
        let gap = lgap / 2.0 + lgap / 2.0 * if on_left { -theta.cos() } else { theta.cos() };
        let label = Label { angle: theta, base_y, block, height, gap, y: base_y };
        if on_left {
            left.push(label);
        } else {
            right.push(label);
        }
    }
    for (side, labels) in [(0, &mut left), (1, &mut right)] {
        distribute(labels, plot.y, plot.h);
        for l in labels.iter() {
            let theta = l.angle;
            let (anchor_x, target_x) = if side == 0 { (link.x, link.x - l.gap) } else { (link.max_x(), link.max_x() + l.gap) };
            let target_y = l.y;
            let (link_x, link_y) = (cx + theta.cos() * r * 0.9, cy - theta.sin() * rv * 0.9);
            let (elbow_x, elbow_y) = (cx + theta.cos() * link.w / 2.0, cy - theta.sin() * link.h / 2.0);
            let segments = [
                (link_x, link_y, elbow_x, elbow_y),
                (anchor_x, elbow_y, elbow_x, elbow_y),
                (anchor_x, elbow_y, target_x, target_y),
            ];
            for &(x1, y1, x2, y2) in &segments {
                cv.line_square(x1, y1, x2, y2, Color::BLACK, 0.5);
            }
            // TextBox: a shadow 2 points off, the box, its outline, the text 2 points in.
            let (bw, bh) = l.block.size(cv, &font);
            let (w, h) = (bw + 4.0, bh + 4.0);
            let (tx, ty) = (target_x as f32 as f64, target_y as f32 as f64);
            let x = if side == 0 { tx - w } else { tx };
            let y = ty - h / 2.0;
            let shadow_box = Rect::new(x + 2.0, y + 2.0, w, h);
            cv.fill_rect(shadow_box, LABEL_SHADOW);
            // The link lines under the (translucent) shadow.
            cv.clipped(shadow_box, |cv| {
                for &(x1, y1, x2, y2) in &segments {
                    cv.line_square(x1, y1, x2, y2, LINK_UNDER_SHADOW, 0.5);
                }
            });
            let bx = Rect::new(x, y, w, h);
            cv.fill_rect(bx, label_back);
            cv.stroke_closed(super::jfree::rect_ops(x, y, w, h), Color::BLACK, 0.5);
            l.block.draw(
                cv,
                &font,
                Color::BLACK,
                (x + 2.0) as f32 as f64,
                (y + 2.0) as f32 as f64,
                BlockAnchor(HAnchor::Left, VAnchor::Top),
            );
        }
    }
}
