//! Custom-painted elements: bar charts, line charts, fans and cooler sprites.
use crate::theme::*;
use gpui::{canvas, fill, point, px, quad, size, BorderStyle, Bounds, IntoElement, PathBuilder, Pixels, Point, Styled, Window};
use std::f32::consts::PI;

fn stroke(window: &mut Window, pts: &[Point<Pixels>], width: f32, color: u32) {
    if pts.len() < 2 {
        return;
    }
    let mut b = PathBuilder::stroke(px(width));
    b.move_to(pts[0]);
    for p in &pts[1..] {
        b.line_to(*p);
    }
    if let Ok(path) = b.build() {
        window.paint_path(path, c(color));
    }
}

fn circle(window: &mut Window, cx: f32, cy: f32, r: f32, color: gpui::Rgba) {
    window.paint_quad(quad(
        Bounds { origin: point(px(cx - r), px(cy - r)), size: size(px(r * 2.0), px(r * 2.0)) },
        px(r),
        color,
        px(0.),
        h(0),
        BorderStyle::default(),
    ));
}

fn ring(window: &mut Window, cx: f32, cy: f32, r: f32, w: f32, color: u32) {
    window.paint_quad(quad(
        Bounds { origin: point(px(cx - r), px(cy - r)), size: size(px(r * 2.0), px(r * 2.0)) },
        px(r),
        ca(0, 0.0),
        px(w),
        h(color),
        BorderStyle::default(),
    ));
}

fn rect(window: &mut Window, x: f32, y: f32, w: f32, hh: f32, color: gpui::Rgba) {
    window.paint_quad(fill(Bounds { origin: point(px(x), px(y)), size: size(px(w), px(hh)) }, color));
}

/// Column chart of the last `n` samples (None = gap). `max` fixes the scale.
pub fn bars(data: Vec<Option<f64>>, color: u32, max: Option<f64>) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |b, _, window, _| {
            let n = 40usize;
            let d: Vec<Option<f64>> = data.iter().rev().take(n).rev().cloned().collect();
            let top = max.unwrap_or_else(|| d.iter().flatten().cloned().fold(1.0, f64::max));
            let (x0, y0) = (f32::from(b.origin.x), f32::from(b.origin.y));
            let (w, hh) = (f32::from(b.size.width), f32::from(b.size.height));
            let bw = w / n as f32;
            let off = n - d.len();
            for (i, v) in d.iter().enumerate() {
                let Some(v) = v else { continue };
                let bh = ((*v / top) as f32 * (hh - 2.0)).clamp(1.5, hh);
                let a = if i + 1 == d.len() { 1.0 } else { 0.55 };
                rect(window, x0 + (off + i) as f32 * bw + 0.5, y0 + hh - bh, (bw - 1.5).max(1.0), bh, ca(color, a));
            }
        },
    )
    .w(px(110.))
    .h(px(52.))
}

/// Multi-series temperature line chart with a 10 °C grid.
pub fn lines(series: Vec<(Vec<Option<f64>>, u32)>, cap: usize) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |b, _, window, _| {
            let (x0, y0) = (f32::from(b.origin.x), f32::from(b.origin.y));
            let (w, hh) = (f32::from(b.size.width), f32::from(b.size.height));
            let all: Vec<f64> = series.iter().flat_map(|s| s.0.iter().flatten().cloned()).collect();
            if all.is_empty() {
                return;
            }
            let mut lo = ((all.iter().cloned().fold(f64::MAX, f64::min) - 3.0) / 10.0).floor() * 10.0;
            let mut hi = ((all.iter().cloned().fold(f64::MIN, f64::max) + 3.0) / 10.0).ceil() * 10.0;
            if hi - lo < 20.0 {
                hi = lo + 20.0;
            }
            if lo < 0.0 {
                lo = 0.0;
            }
            let y = |v: f64| y0 + hh - 4.0 - ((v - lo) / (hi - lo)) as f32 * (hh - 8.0);
            let mut v = lo;
            while v <= hi {
                rect(window, x0, y(v), w, 1.0, c(LINE));
                v += 10.0;
            }
            for (data, color) in &series {
                let len = data.len();
                let mut seg: Vec<Point<Pixels>> = Vec::new();
                for (i, v) in data.iter().enumerate() {
                    let x = x0 + ((i + cap - len) as f32 / (cap - 1) as f32) * w;
                    match v {
                        Some(v) => seg.push(point(px(x), px(y(*v)))),
                        None => {
                            stroke(window, &seg, 2.0, *color);
                            seg.clear();
                        }
                    }
                }
                stroke(window, &seg, 2.0, *color);
            }
        },
    )
    .w_full()
    .h(px(110.))
}

/// Paint a 5-blade fan centred at (cx, cy), rotated by `angle` radians.
pub fn paint_fan(window: &mut Window, cx: f32, cy: f32, r: f32, angle: f32, blade: u32, hub: u32) {
    for k in 0..5 {
        let a = angle + k as f32 * 2.0 * PI / 5.0;
        let rot = |x: f32, y: f32| {
            let (s, co) = a.sin_cos();
            point(px(cx + x * co - y * s), px(cy + x * s + y * co))
        };
        let mut b = PathBuilder::fill();
        b.move_to(rot(0.0, 0.0));
        b.cubic_bezier_to(rot(r * 0.6, -r * 0.55), rot(0.0, -r * 0.9), rot(r * 0.55, -r * 0.95));
        b.cubic_bezier_to(rot(r * 0.2, -r * 0.1), rot(r * 0.5, -r * 0.3), rot(0.0, 0.0));
        b.close();
        if let Ok(p) = b.build() {
            window.paint_path(p, c(blade));
        }
    }
    circle(window, cx, cy, r * 0.22, c(hub));
}

/// Standalone fan element (used on the GPU board).
pub fn fan(angle: f32, d: f32) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |b, _, window, _| {
            let cx = f32::from(b.origin.x) + d / 2.0;
            let cy = f32::from(b.origin.y) + d / 2.0;
            ring(window, cx, cy, d / 2.0 - 1.0, 2.0, LINE2);
            paint_fan(window, cx, cy, d / 2.0 - 4.0, angle, 0x3a4452, AMBER);
        },
    )
    .size(px(d))
}

/// Cooler sprite: "water" | "air" | other (unknown).
pub fn cooler(kind: String, fan_angle: f32, pump_angle: f32, flow: f32) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |b, _, window, _| {
            // design space 210 x 150, scaled to fit
            let s = (f32::from(b.size.width) / 210.0).min(f32::from(b.size.height) / 150.0);
            let ox = f32::from(b.origin.x);
            let oy = f32::from(b.origin.y);
            let p = |x: f32, y: f32| (ox + x * s, oy + y * s);
            let r = |window: &mut Window, x: f32, y: f32, w: f32, hh: f32, col: gpui::Rgba| {
                let (a, bb) = p(x, y);
                rect(window, a, bb, w * s, hh * s, col)
            };
            match kind.as_str() {
                "water" => {
                    r(window, 6.0, 8.0, 198.0, 52.0, c(0x161c24));
                    for i in 0..30 {
                        r(window, 14.0 + i as f32 * 6.2, 12.0, 1.0, 44.0, c(0x262e39));
                    }
                    for fx in [56.0, 154.0] {
                        let (cx, cy) = p(fx, 34.0);
                        circle(window, cx, cy, 22.0 * s, c(0x10151b));
                        ring(window, cx, cy, 22.0 * s, 1.0, LINE2);
                        paint_fan(window, cx, cy, 20.0 * s, fan_angle, 0x3c4756, AMBER);
                    }
                    // tubes with moving dashes
                    for (x0, x1, col) in [(80.0, 100.0, CYAN), (130.0, 112.0, ORANGE)] {
                        let pts: Vec<Point<Pixels>> = (0..=12)
                            .map(|i| {
                                let t = i as f32 / 12.0;
                                let (a, bb) = p(x0 + (x1 - x0) * t * t, 60.0 + 52.0 * t);
                                point(px(a), px(bb))
                            })
                            .collect();
                        let mut b = PathBuilder::stroke(px(5.0 * s)).dash_array(&[px(6.0 * s), px(6.0 * s)]);
                        let _ = flow; // dash offset isn't exposed; flow shown by the pump spin
                        b.move_to(pts[0]);
                        for q in &pts[1..] {
                            b.line_to(*q);
                        }
                        if let Ok(path) = b.build() {
                            window.paint_path(path, c(col));
                        }
                    }
                    r(window, 80.0, 106.0, 52.0, 38.0, c(0x1b2129));
                    let (cx, cy) = p(106.0, 125.0);
                    ring(window, cx, cy, 11.0 * s, 2.0, CYAN);
                    // pump impeller marks
                    for k in 0..3 {
                        let a = pump_angle + k as f32 * 2.0 * PI / 3.0;
                        let (sn, co) = a.sin_cos();
                        circle(window, cx + co * 7.0 * s, cy + sn * 7.0 * s, 2.0 * s, c(CYAN));
                    }
                    circle(window, cx, cy, 4.0 * s, c(CYAN));
                }
                "air" => {
                    r(window, 40.0, 128.0, 130.0, 14.0, c(0x1a2e22));
                    r(window, 88.0, 118.0, 34.0, 12.0, c(0x5a6472));
                    for i in 0..4 {
                        r(window, 92.5 + i as f32 * 7.0, 14.0, 3.0, 106.0, c(0xb07a3a));
                    }
                    for i in 0..22 {
                        r(window, 70.0, 12.0 + i as f32 * 4.8, 96.0, 2.4, c(0x4a5462));
                    }
                    r(window, 16.0, 20.0, 56.0, 96.0, c(0x10151b));
                    let (cx, cy) = p(44.0, 68.0);
                    paint_fan(window, cx, cy, 25.0 * s, fan_angle, 0x3c4756, AMBER);
                }
                _ => {
                    let (a, bb) = p(55.0, 25.0);
                    window.paint_quad(quad(
                        Bounds { origin: point(px(a), px(bb)), size: size(px(100.0 * s), px(100.0 * s)) },
                        px(8.0),
                        ca(0, 0.0),
                        px(1.0),
                        h(LINE2),
                        BorderStyle::Dashed,
                    ));
                }
            }
        },
    )
    .w(px(210.))
    .h(px(150.))
}
