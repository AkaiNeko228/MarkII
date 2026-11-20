//! Mark II palettes. Colors are 0xRRGGBB.
//!
//! The constants below are the default "Grafite" palette and double as color
//! *roles*: every color goes through [`c`]/[`ca`]/[`mix`]/[`h`], which swap a
//! role for the active theme's color. Hard-coded greys (sprite borders, PCB
//! tones) are mapped by brightness onto the theme's grey ramp, so the whole UI
//! follows the theme without each panel knowing about it.
use gpui::{rgb, rgba, Hsla, Rgba};
use std::sync::atomic::{AtomicUsize, Ordering};

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

#[cfg(windows)]
pub const NUM_FONT: &str = "Bahnschrift";
#[cfg(windows)]
pub const UI_FONT: &str = "Segoe UI";
// Linux: common families; GPUI falls back to whatever sans font is installed.
#[cfg(not(windows))]
pub const NUM_FONT: &str = "DejaVu Sans";
#[cfg(not(windows))]
pub const UI_FONT: &str = "Noto Sans";

/// A theme: 8 greys (BG → TEXT) and 8 accents, in the order of the constants.
pub struct Theme {
    pub key: &'static str,
    pub name: &'static str,
    pub light: bool,
    pub grey: [u32; 8],
    pub accent: [u32; 8],
}

const ROLES_GREY: [u32; 8] = [BG, PANEL, PANEL2, LINE, LINE2, DIM, MUTED, TEXT];
const ROLES_ACCENT: [u32; 8] = [AMBER, INDIGO, GREEN, CYAN, RED, PINK, YELLOW, ORANGE];

pub const THEMES: [Theme; 8] = [
    Theme { key: "grafite", name: "Grafite", light: false, grey: ROLES_GREY, accent: ROLES_ACCENT },
    Theme {
        key: "roxo",
        name: "Roxo",
        light: false,
        grey: [0x07060b, 0x0e0c15, 0x15121f, 0x221d33, 0x2d2643, 0x5e5577, 0x9a90b5, 0xece8f6],
        accent: [0xb57cff, 0x8f6bff, 0x5fd49a, 0x7aa8ff, 0xff5f7a, 0xe06bff, 0xf0c75e, 0xff8f6b],
    },
    Theme {
        key: "carmesim",
        name: "Carmesim",
        light: false,
        grey: [0x0b0708, 0x130c0e, 0x1b1114, 0x2a181c, 0x372026, 0x6e5157, 0xa88d92, 0xf2e6e8],
        accent: [0xe0284f, 0xff6b81, 0x5fd49a, 0x6fb8e8, 0xff3b3b, 0xff7aa8, 0xf2c14e, 0xff7a45],
    },
    Theme {
        key: "azul",
        name: "Azul profundo",
        light: false,
        grey: [0x050a14, 0x0a1220, 0x0f1a2c, 0x17263f, 0x1f3150, 0x4d6385, 0x8ea3c4, 0xe6eefb],
        accent: [0x3d8bff, 0x6c8cff, 0x4fd6a0, 0x4cc9f0, 0xff5a6a, 0xb48cff, 0xf2d06b, 0xff9a5a],
    },
    Theme {
        key: "lavanda",
        name: "Lavanda",
        light: false,
        grey: [0x14121d, 0x1b1927, 0x232031, 0x2f2b40, 0x3a354e, 0x6b6585, 0xa39cbc, 0xeeeaf8],
        accent: [0xf6c98e, 0xb6a9f6, 0x9fdfb9, 0xa0d6ee, 0xf4a0a0, 0xf3abcb, 0xf1df9c, 0xf5b58f],
    },
    Theme {
        key: "menta",
        name: "Menta",
        light: false,
        grey: [0x0e1818, 0x142121, 0x1a2a2a, 0x243736, 0x2e4443, 0x5c7672, 0x93afaa, 0xe6f3f0],
        accent: [0xf1d19c, 0xabb9f1, 0x9fe5c2, 0x91dbe0, 0xf1a3a3, 0xefb2d1, 0xede2a1, 0xf2be96],
    },
    Theme {
        key: "algodao",
        name: "Algodão-doce",
        light: true,
        grey: [0xf5eff7, 0xfffbff, 0xf2e9f5, 0xe4d7ea, 0xd5c5dc, 0xa797b1, 0x776a85, 0x382e44],
        accent: [0xd08a2c, 0x7d6fd9, 0x2f9f6a, 0x2c92b5, 0xd9534f, 0xcf5a92, 0xa98c12, 0xd97b3d],
    },
    Theme {
        key: "pistache",
        name: "Pistache",
        light: true,
        grey: [0xeff5ef, 0xfbfefb, 0xe8f2ea, 0xd4e4d8, 0xc2d6c8, 0x98ae9f, 0x667d70, 0x2a3831],
        accent: [0xc8862a, 0x6a80d2, 0x2a9663, 0x2a8faa, 0xd0524d, 0xc35b8a, 0x9c8410, 0xcf773d],
    },
];

static ACTIVE: AtomicUsize = AtomicUsize::new(0);

pub fn set_theme(key: &str) {
    ACTIVE.store(THEMES.iter().position(|t| t.key == key).unwrap_or(0), Ordering::Relaxed);
}

pub fn theme() -> &'static Theme {
    &THEMES[ACTIVE.load(Ordering::Relaxed).min(THEMES.len() - 1)]
}

fn chan(v: u32, sh: u32) -> f32 {
    ((v >> sh) & 0xff) as f32
}

fn lerp(a: u32, b: u32, t: f32) -> u32 {
    let f = |sh: u32| (chan(a, sh) + (chan(b, sh) - chan(a, sh)) * t).round().clamp(0.0, 255.0) as u32;
    (f(16) << 16) | (f(8) << 8) | f(0)
}

fn luma(v: u32) -> f32 {
    (0.2126 * chan(v, 16) + 0.7152 * chan(v, 8) + 0.0722 * chan(v, 0)) / 255.0
}

/// Maps a Grafite color to the active theme.
pub fn remap(hex: u32) -> u32 {
    let th = theme();
    if ACTIVE.load(Ordering::Relaxed) == 0 {
        return hex;
    }
    if let Some(i) = ROLES_GREY.iter().position(|r| *r == hex) {
        return th.grey[i];
    }
    if let Some(i) = ROLES_ACCENT.iter().position(|r| *r == hex) {
        return th.accent[i];
    }
    // Any other color: place it on the theme's grey ramp by brightness, then
    // keep a little of its own hue (PCB green, gold pins...).
    let l = luma(hex);
    let ramp: Vec<f32> = ROLES_GREY.iter().map(|g| luma(*g)).collect();
    let mut grey = th.grey[7];
    for i in 0..7 {
        if l <= ramp[i + 1] {
            let t = ((l - ramp[i]) / (ramp[i + 1] - ramp[i]).max(1e-4)).clamp(0.0, 1.0);
            grey = lerp(th.grey[i], th.grey[i + 1], t);
            break;
        }
    }
    let (r, g, b) = (chan(hex, 16), chan(hex, 8), chan(hex, 0));
    let sat = (r.max(g).max(b) - r.min(g).min(b)) / 255.0;
    if sat < 0.12 {
        grey
    } else {
        lerp(grey, hex, if th.light { 0.25 } else { 0.5 })
    }
}

pub fn c(hex: u32) -> Rgba {
    rgb(remap(hex))
}

/// Color with alpha 0..=1.
pub fn ca(hex: u32, a: f32) -> Rgba {
    rgba((remap(hex) << 8) | ((a.clamp(0.0, 1.0) * 255.0) as u32))
}

pub fn h(hex: u32) -> Hsla {
    rgb(remap(hex)).into()
}

/// Mix `hex` over `base` (for tinted tile backgrounds).
pub fn mix(hex: u32, base: u32, t: f32) -> Rgba {
    rgb(lerp(remap(base), remap(hex), t))
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
