//! Mark II palette: graphite + amber. Colors are 0xRRGGBB.
use gpui::{rgb, rgba, Hsla, Rgba};

pub const BG: u32 = 0x0a0d11;
pub const PANEL: u32 = 0x11151b;
pub const PANEL2: u32 = 0x171c24;
pub const LINE: u32 = 0x222a35;
pub const LINE2: u32 = 0x2c3542;
pub const TEXT: u32 = 0xe8edf3;
pub const MUTED: u32 = 0x8592a3;
pub const DIM: u32 = 0x556070;

pub const AMBER: u32 = 0xffb020;
pub const INDIGO: u32 = 0x7b7ff5;
pub const GREEN: u32 = 0x3fd17a;
pub const CYAN: u32 = 0x44c7e0;
pub const RED: u32 = 0xff5a52;
pub const PINK: u32 = 0xff5c8a;
pub const YELLOW: u32 = 0xf2cc3d;
pub const ORANGE: u32 = 0xff8a3d;

pub const NUM_FONT: &str = "Bahnschrift";
pub const UI_FONT: &str = "Segoe UI";

pub fn c(hex: u32) -> Rgba {
    rgb(hex)
}

/// Color with alpha 0..=1.
pub fn ca(hex: u32, a: f32) -> Rgba {
    rgba((hex << 8) | ((a.clamp(0.0, 1.0) * 255.0) as u32))
}

pub fn h(hex: u32) -> Hsla {
    rgb(hex).into()
}

/// Mix `hex` over `base` (for tinted tile backgrounds).
pub fn mix(hex: u32, base: u32, t: f32) -> Rgba {
    let f = |sh: u32| {
        let a = ((hex >> sh) & 0xff) as f32;
        let b = ((base >> sh) & 0xff) as f32;
        (b + (a - b) * t).round() as u32
    };
    rgb((f(16) << 16) | (f(8) << 8) | f(0))
}

/// Temperature → color (cool cyan → green → yellow → orange → red).
pub fn temp_col(t: Option<f64>) -> u32 {
    match t {
        None => DIM,
        Some(t) if t < 45.0 => CYAN,
        Some(t) if t < 65.0 => GREEN,
        Some(t) if t < 78.0 => YELLOW,
        Some(t) if t < 88.0 => ORANGE,
        Some(_) => RED,
    }
}

/// Load % → color.
pub fn load_col(p: Option<f64>) -> u32 {
    match p {
        None => DIM,
        Some(p) if p < 40.0 => CYAN,
        Some(p) if p < 70.0 => GREEN,
        Some(p) if p < 90.0 => YELLOW,
        Some(_) => ORANGE,
    }
}
