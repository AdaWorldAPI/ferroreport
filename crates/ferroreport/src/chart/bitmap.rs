//! Translucent shapes in charts drawn as SVG. Batik paints a group with an opacity below 1 into
//! a bitmap of a pixel a point, with no anti-aliasing, and the PDF graphics then draw that
//! bitmap opaque (JasperReports' 3D pies, candlestick volumes): the same bitmaps, cut into the
//! same 128-pixel tiles, come out the same on the page.

use super::Canvas;
use crate::model::Color;
use crate::print::{PathOp, PrintItem};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// Pixels of a bitmap laid on the chart's grid of whole points.
struct Bitmap {
    x0: i64,
    y0: i64,
    w: usize,
    h: usize,
    on: Vec<bool>,
}

impl Bitmap {
    fn new(x0: i64, y0: i64, x1: i64, y1: i64) -> Bitmap {
        let (w, h) = ((x1 - x0).max(0) as usize, (y1 - y0).max(0) as usize);
        Bitmap { x0, y0, w, h, on: vec![false; w * h] }
    }
    fn set(&mut self, x: i64, y: i64) {
        let (i, j) = (x - self.x0, y - self.y0);
        if i >= 0 && j >= 0 && (i as usize) < self.w && (j as usize) < self.h {
            self.on[j as usize * self.w + i as usize] = true;
        }
    }
}

/// The path relative to `origin`, normalized as Java2D's span iterator normalizes paths it
/// fills without anti-aliasing (STROKE_NORMALIZE): end points moved to a quarter past the
/// pixel, `floor(v + 0.25) + 0.25`, control points moved along with their end points.
fn normalize(ops: &[PathOp], origin: (f64, f64)) -> Vec<PathOp> {
    let n = |v: f64| (v + 0.25).floor() + 0.25;
    let mut adj = (0.0, 0.0);
    let mut out = Vec::with_capacity(ops.len());
    for op in ops {
        let (ox, oy) = origin;
        out.push(match *op {
            PathOp::Move(x, y) => {
                let (x, y) = (x - ox, y - oy);
                adj = (n(x) - x, n(y) - y);
                PathOp::Move(n(x), n(y))
            }
            PathOp::Line(x, y) => {
                let (x, y) = (x - ox, y - oy);
                adj = (n(x) - x, n(y) - y);
                PathOp::Line(n(x), n(y))
            }
            PathOp::Quad(a, b, x, y) => {
                let (a, b, x, y) = (a - ox, b - oy, x - ox, y - oy);
                let new = (n(x) - x, n(y) - y);
                let q = PathOp::Quad(a + (adj.0 + new.0) / 2.0, b + (adj.1 + new.1) / 2.0, n(x), n(y));
                adj = new;
                q
            }
            PathOp::Cubic(a, b, c, d, x, y) => {
                let (a, b, c, d, x, y) = (a - ox, b - oy, c - ox, d - oy, x - ox, y - oy);
                let new = (n(x) - x, n(y) - y);
                let q = PathOp::Cubic(a + adj.0, b + adj.1, c + new.0, d + new.1, n(x), n(y));
                adj = new;
                q
            }
            PathOp::Close => PathOp::Close,
        });
    }
    out
}

/// The path as Java2D flattens it to rasterize it (approximately: curves cut where they turn
/// back in x or y, halved while over 256 pixels, then into 8 even steps), relative to
/// `origin`.
fn java_segments(ops: &[PathOp], origin: (f64, f64)) -> Vec<Vec<(f64, f64)>> {
    type P = (f64, f64);
    fn split(c: [P; 4], t: f64) -> ([P; 4], [P; 4]) {
        let l = |a: P, b: P| (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
        let (ab, bc, cd) = (l(c[0], c[1]), l(c[1], c[2]), l(c[2], c[3]));
        let (abc, bcd) = (l(ab, bc), l(bc, cd));
        let m = l(abc, bcd);
        ([c[0], ab, abc, m], [m, bcd, cd, c[3]])
    }
    fn turns(c: [P; 4]) -> Vec<f64> {
        let mut ts = Vec::new();
        for v in [[c[0].0, c[1].0, c[2].0, c[3].0], [c[0].1, c[1].1, c[2].1, c[3].1]] {
            // The derivative's roots: 3(a t^2 + 2 b t + c).
            let (a, b, cc) = (-v[0] + 3.0 * v[1] - 3.0 * v[2] + v[3], v[0] - 2.0 * v[1] + v[2], v[1] - v[0]);
            if a.abs() < 1e-12 {
                if b.abs() > 1e-12 {
                    ts.push(-cc / (2.0 * b));
                }
            } else {
                let disc = b * b - a * cc;
                if disc >= 0.0 {
                    ts.push((-b + disc.sqrt()) / a);
                    ts.push((-b - disc.sqrt()) / a);
                }
            }
        }
        ts.retain(|t| *t > 1e-9 && *t < 1.0 - 1e-9);
        ts.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        ts.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
        ts
    }
    fn steps(c: [P; 4], out: &mut Vec<P>) {
        let (xs, ys) = (c.map(|p| p.0), c.map(|p| p.1));
        let size =
            |v: [f64; 4]| v.iter().cloned().fold(f64::NEG_INFINITY, f64::max) - v.iter().cloned().fold(f64::INFINITY, f64::min);
        if size(xs) > 256.0 || size(ys) > 256.0 {
            let (a, b) = split(c, 0.5);
            steps(a, out);
            steps(b, out);
            return;
        }
        for k in 1..=8 {
            let t = k as f64 / 8.0;
            let u = 1.0 - t;
            let f = |a: f64, b: f64, c2: f64, d: f64| u * u * u * a + 3.0 * u * u * t * b + 3.0 * u * t * t * c2 + t * t * t * d;
            out.push((f(c[0].0, c[1].0, c[2].0, c[3].0), f(c[0].1, c[1].1, c[2].1, c[3].1)));
        }
    }
    let mut out: Vec<Vec<P>> = Vec::new();
    let mut cur = (0.0, 0.0);
    let mut start = (0.0, 0.0);
    let p = |x: f64, y: f64| (x - origin.0, y - origin.1);
    for op in ops {
        let cubic = match *op {
            PathOp::Move(x, y) => {
                cur = p(x, y);
                start = cur;
                out.push(vec![cur]);
                None
            }
            PathOp::Line(x, y) => {
                cur = p(x, y);
                if let Some(s) = out.last_mut() {
                    s.push(cur);
                }
                None
            }
            PathOp::Quad(a, b, x, y) => {
                let (q, e) = (p(a, b), p(x, y));
                let c1 = (cur.0 + 2.0 / 3.0 * (q.0 - cur.0), cur.1 + 2.0 / 3.0 * (q.1 - cur.1));
                let c2 = (e.0 + 2.0 / 3.0 * (q.0 - e.0), e.1 + 2.0 / 3.0 * (q.1 - e.1));
                Some([cur, c1, c2, e])
            }
            PathOp::Cubic(a, b, c, d, x, y) => Some([cur, p(a, b), p(c, d), p(x, y)]),
            PathOp::Close => {
                if let Some(s) = out.last_mut() {
                    s.push(start);
                }
                cur = start;
                None
            }
        };
        if let Some(c) = cubic {
            let mut rest = c;
            let mut last = 0.0;
            let mut pts = Vec::new();
            for t in turns(c) {
                let (a, b) = split(rest, (t - last) / (1.0 - last));
                steps(a, &mut pts);
                rest = b;
                last = t;
            }
            steps(rest, &mut pts);
            if let Some(s) = out.last_mut() {
                s.extend(pts);
            }
            cur = c[3];
        }
    }
    out
}

/// The path as line segments (curves cut into 16 pieces), in coordinates relative to `origin`.
fn segments(ops: &[PathOp], origin: (f64, f64)) -> Vec<Vec<(f64, f64)>> {
    let mut out: Vec<Vec<(f64, f64)>> = Vec::new();
    let mut cur = (0.0, 0.0);
    let mut start = (0.0, 0.0);
    let p = |x: f64, y: f64| (x - origin.0, y - origin.1);
    for op in ops {
        match *op {
            PathOp::Move(x, y) => {
                cur = p(x, y);
                start = cur;
                out.push(vec![cur]);
            }
            PathOp::Line(x, y) => {
                cur = p(x, y);
                if let Some(s) = out.last_mut() {
                    s.push(cur);
                }
            }
            PathOp::Quad(a, b, x, y) => {
                let (q, e) = (p(a, b), p(x, y));
                for k in 1..=16 {
                    let t = k as f64 / 16.0;
                    let u = 1.0 - t;
                    let pt = (u * u * cur.0 + 2.0 * u * t * q.0 + t * t * e.0, u * u * cur.1 + 2.0 * u * t * q.1 + t * t * e.1);
                    if let Some(s) = out.last_mut() {
                        s.push(pt);
                    }
                }
                cur = e;
            }
            PathOp::Cubic(a, b, c, d, x, y) => {
                let (c1, c2, e) = (p(a, b), p(c, d), p(x, y));
                for k in 1..=16 {
                    let t = k as f64 / 16.0;
                    let u = 1.0 - t;
                    let f = |p0: f64, p1: f64, p2: f64, p3: f64| {
                        u * u * u * p0 + 3.0 * u * u * t * p1 + 3.0 * u * t * t * p2 + t * t * t * p3
                    };
                    let pt = (f(cur.0, c1.0, c2.0, e.0), f(cur.1, c1.1, c2.1, e.1));
                    if let Some(s) = out.last_mut() {
                        s.push(pt);
                    }
                }
                cur = e;
            }
            PathOp::Close => {
                if let Some(s) = out.last_mut() {
                    s.push(start);
                }
                cur = start;
            }
        }
    }
    out
}

/// The bounds of the points, unrounded.
fn extent<'a>(points: impl Iterator<Item = &'a (f64, f64)>) -> Option<(f64, f64, f64, f64)> {
    let (mut x0, mut y0, mut x1, mut y1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for p in points {
        x0 = x0.min(p.0);
        y0 = y0.min(p.1);
        x1 = x1.max(p.0);
        y1 = y1.max(p.1);
    }
    x0.is_finite().then_some((x0, y0, x1, y1))
}

/// `Rectangle2D.getBounds`: the whole pixels the bounds touch.
fn pixels(e: (f64, f64, f64, f64)) -> (i64, i64, i64, i64) {
    (e.0.floor() as i64, e.1.floor() as i64, e.2.ceil() as i64, e.3.ceil() as i64)
}

/// A shape filled without anti-aliasing: the pixels whose centers are inside (nonzero rule).
fn fill(polys: &[Vec<(f64, f64)>], e: (f64, f64, f64, f64)) -> Bitmap {
    let (x0, y0, x1, y1) = pixels(e);
    let mut b = Bitmap::new(x0, y0, x1, y1);
    for j in y0..y1 {
        let yc = j as f64 + 0.5;
        // Crossings of the row's center line, with their winding direction.
        let mut xs: Vec<(f64, i32)> = Vec::new();
        for poly in polys {
            for w in poly.windows(2) {
                let ((ax, ay), (bx, by)) = (w[0], w[1]);
                if (ay <= yc && by > yc) || (by <= yc && ay > yc) {
                    let x = ax + (yc - ay) * (bx - ax) / (by - ay);
                    xs.push((x, if by > ay { 1 } else { -1 }));
                }
            }
        }
        xs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        let mut winding = 0;
        for k in 0..xs.len() {
            winding += xs[k].1;
            if winding != 0 && k + 1 < xs.len() {
                let (a, c) = (xs[k].0, xs[k + 1].0);
                let mut i = (a - 0.5).ceil() as i64;
                while (i as f64) + 0.5 < c {
                    b.set(i, j);
                    i += 1;
                }
            }
        }
    }
    b
}

/// A thin line (a point wide or less) drawn without anti-aliasing, as Java2D draws a path's
/// segments (measured against its own rendering): along the longer direction, the pixels at
/// the whole coordinates the segment passes, from the first to the last joined by Bresenham's
/// steps (halfway rounding towards the end), the other coordinate rounded at both ends.
fn thin_stroke(polys: &[Vec<(f64, f64)>], e: (f64, f64, f64, f64)) -> Bitmap {
    let (x0, y0, x1, y1) = pixels(e);
    let mut b = Bitmap::new(x0, y0, x1, y1);
    for poly in polys {
        for w in poly.windows(2) {
            let ((ax, ay), (bx, by)) = (w[0], w[1]);
            let (dx, dy) = (bx - ax, by - ay);
            let along_x = dx.abs() >= dy.abs();
            let (a, e, d, m, dm) = if along_x { (ax, bx, dx, ay, dy) } else { (ay, by, dy, ax, dx) };
            if d == 0.0 {
                continue;
            }
            let (k0, k1) = if d > 0.0 { (a.ceil(), e.floor()) } else { (a.floor(), e.ceil()) };
            if (k1 - k0) * d.signum() < 0.0 {
                continue;
            }
            let other = |k: f64| (m + (k - a) * dm / d + 0.5).floor();
            let (m0, m1) = (other(k0), other(k1));
            let n = (k1 - k0).abs().max((m1 - m0).abs()) as i64;
            for i in 0..=n {
                let t = |from: f64, to: f64| {
                    if n == 0 {
                        from
                    } else {
                        from + (to - from).signum() * ((to - from).abs() * i as f64 / n as f64 + 0.5).floor()
                    }
                };
                let (k, o) = (t(k0, k1) as i64, t(m0, m1) as i64);
                if along_x {
                    b.set(k, o);
                } else {
                    b.set(o, k);
                }
            }
        }
    }
    b
}

/// Image ids for chart bitmaps, apart from those of the report's images.
static NEXT_ID: AtomicUsize = AtomicUsize::new(crate::image::KEYED_IDS);

/// Draws `b` in `color`, cut into tiles as Batik cuts it.
fn emit(cv: &mut Canvas, b: &Bitmap, color: Color, origin: (f64, f64)) {
    if b.w == 0 || b.h == 0 || !b.on.iter().any(|&v| v) {
        return;
    }
    // GraphicsNodeRed8Bit's tiles: a grid from the multiple of 128 at or before the image,
    // tiles at most 128 wide; GraphicsUtil.drawImage draws the image whole when one tile
    // holds as many pixels.
    let size = |from: i64, len: usize| (from + len as i64 - from.div_euclid(128) * 128).clamp(1, 128);
    let (tw, th) = (size(b.x0, b.w), size(b.y0, b.h));
    let whole = tw * th >= (b.w * b.h) as i64;
    let cut = |from: i64, len: usize, step: i64| {
        let end = from + len as i64;
        if whole {
            return vec![(from, end)];
        }
        let mut out = Vec::new();
        let mut a = from.div_euclid(128) * 128;
        while a < end {
            out.push((a.max(from), (a + step).min(end)));
            a += step;
        }
        out
    };
    for (ty0, ty1) in cut(b.y0, b.h, th) {
        for (tx0, tx1) in cut(b.x0, b.w, tw) {
            let (w, h) = ((tx1 - tx0) as usize, (ty1 - ty0) as usize);
            let mut samples = Vec::with_capacity(w * h * 3);
            let mut alpha = Vec::with_capacity(w * h);
            let mut any = false;
            for y in ty0..ty1 {
                for x in tx0..tx1 {
                    let on = b.on[(y - b.y0) as usize * b.w + (x - b.x0) as usize];
                    any |= on;
                    let c = if on { color } else { Color::BLACK };
                    samples.extend_from_slice(&[c.r, c.g, c.b]);
                    alpha.push(if on { 255 } else { 0 });
                }
            }
            if !any {
                continue;
            }
            let image = crate::image::ImageData {
                id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
                width: w as u32,
                height: h as u32,
                // Whole tiles have no mask.
                pixels: crate::image::Pixels::Raw {
                    samples,
                    gray: false,
                    alpha: if alpha.iter().all(|&a| a == 255) { None } else { Some(alpha) },
                },
            };
            cv.items.push(PrintItem::Image {
                x: (origin.0 + tx0 as f64) as f32,
                y: (origin.1 + ty0 as f64) as f32,
                w: w as f32,
                h: h as f32,
                clip: None,
                image: Arc::new(image),
            });
        }
    }
}

/// The bitmap's tiles, clipped to the shape's bounds (chart-local `e`, which the SVG writes
/// to 0.01) as the SVG clips the group's image.
fn emit_clipped(cv: &mut Canvas, b: &Bitmap, color: Color, e: (f64, f64, f64, f64)) {
    let origin = cv.origin;
    let r = |v: f64| (v * 100.0).round() / 100.0;
    let (x0, y0, x1, y1) = (r(e.0), r(e.1), r(e.2), r(e.3));
    let clip = super::jfree::Rect::new(origin.0 + x0, origin.1 + y0, x1 - x0, y1 - y0);
    cv.clipped(clip, |cv| emit(cv, b, color, origin));
}

/// `ops` filled in `color` as a translucent group of an SVG chart reaches the page. Java2D
/// fills a path (as Batik draws an SVG path) without anti-aliasing by its pixels' top left
/// corners: the pixels whose corner is inside.
pub(crate) fn fill_bitmap(cv: &mut Canvas, ops: &[PathOp], color: Color) {
    let Some(e) = extent(segments(ops, cv.origin).iter().flatten()) else { return };
    let b = fill(&java_segments(ops, (cv.origin.0 - 0.5, cv.origin.1 - 0.5)), e);
    emit_clipped(cv, &b, color, e);
}

/// `r` filled in `color` as a translucent group of an SVG chart reaches the page. Java2D fills
/// a rectangle (an SVG rect) normalized: its edges moved to a quarter past the pixel, the
/// pixels whose centers are inside.
pub(crate) fn fill_rect_bitmap(cv: &mut Canvas, r: super::jfree::Rect, color: Color) {
    let ops = super::jfree::rect_ops(r.x, r.y, r.w, r.h);
    let Some(e) = extent(segments(&ops, cv.origin).iter().flatten()) else { return };
    let b = fill(&segments(&normalize(&ops, cv.origin), (0.0, 0.0)), e);
    emit_clipped(cv, &b, color, e);
}

/// The outline of `ops`, a thin line `width` wide, in `color`, as a translucent group of an
/// SVG chart reaches the page.
pub(crate) fn stroke_bitmap(cv: &mut Canvas, ops: &[PathOp], color: Color, width: f64) {
    let polys = java_segments(ops, cv.origin);
    // Batik bounds a stroke by the shape's bounds grown half the width (no miters).
    let hw = width / 2.0;
    let Some(e) = extent(segments(ops, cv.origin).iter().flatten()).map(|e| (e.0 - hw, e.1 - hw, e.2 + hw, e.3 + hw)) else {
        return;
    };
    emit_clipped(cv, &thin_stroke(&polys, e), color, e);
}
