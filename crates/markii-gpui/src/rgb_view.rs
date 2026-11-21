//! RGB controller sheet (OpenRGB through markii_core::rgb).
use crate::theme::*;
use crate::view::{label, num, small, Dashboard};
use gpui::{div, prelude::*, px, Context, Div, FontWeight, MouseButton, MouseMoveEvent, SharedString, Stateful};
use markii_core::rgb::{self, Status};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{mpsc, Arc, Mutex};

/// UI state of the RGB sheet (lives while the sheet is open).
pub struct RgbUi {
    pub color: u32,
    pub brightness: u8,
    pub modes: HashMap<u32, usize>,
    pub off: HashSet<u32>,
    pub save_device: bool,
    /// Written by background work, read on the next frame.
    pub msg: Arc<Mutex<String>>,
    pub autostart: Arc<Mutex<Option<bool>>>,
    pub busy: Arc<Mutex<bool>>,
    /// Colors the user saved (rgb-favoritas.json).
    pub favorites: Vec<u32>,
    /// Small xorshift state for "Aleatória".
    seed: u32,
    /// Show each change on the LEDs right away ("Aplicar" saves it).
    pub live: bool,
    /// Gradient: the color not being edited now (None = one color). `color` is
    /// always the one being edited; `slot` says whether that is color 1 or 2.
    pub grad: Option<u32>,
    pub slot: u8,
    /// What was last sent to the preview worker, to send only changes.
    sent: Mutex<u64>,
    /// Preview worker; it ends when this is dropped (sheet closed).
    preview: Mutex<Option<mpsc::Sender<Preview>>>,
}

/// One live preview: the latest color for the selected devices (nothing saved).
struct Preview {
    color: u32,
    color2: Option<u32>,
    brightness: u8,
    devs: Vec<(rgb::Device, usize)>,
}

/// Applies previews in the background, newest only, over one OpenRGB connection.
/// The effect is sent once per device (then only LED colors), and mice / keyboards
/// get at most ~3 updates a second: their USB link also carries the movement and
/// key reports, and a flood of color writes makes the pointer stutter.
fn preview_worker() -> mpsc::Sender<Preview> {
    use std::time::{Duration, Instant};
    let (tx, rx) = mpsc::channel::<Preview>();
    std::thread::spawn(move || {
        let mut client: Option<rgb::Client> = None;
        // device -> (effect, brightness) already sent
        let mut sent: HashMap<u32, (usize, u8)> = HashMap::new();
        let mut latest: Option<Preview> = None;
        let (mut fast_dirty, mut slow_dirty) = (false, false);
        let mut last_slow = Instant::now() - Duration::from_secs(1);
        let mut open = true;
        while open || slow_dirty {
            match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(mut p) => {
                    while let Ok(newer) = rx.try_recv() {
                        p = newer;
                    }
                    latest = Some(p);
                    fast_dirty = true;
                    slow_dirty = true;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                // sheet closed: send the last color to the peripherals too, then end
                Err(mpsc::RecvTimeoutError::Disconnected) => open = false,
            }
            let Some(p) = latest.as_ref() else { continue };
            let slow_due = slow_dirty && (last_slow.elapsed() >= Duration::from_millis(330) || !open);
            if !fast_dirty && !slow_due {
                continue;
            }
            if client.is_none() {
                client = rgb::Client::connect(rgb::PORT).ok();
            }
            let Some(c) = client.as_mut() else {
                if !open {
                    break;
                }
                continue;
            };
            let mut failed = false;
            for (d, m) in &p.devs {
                let periph = d.group() == "Periféricos";
                if (periph && !slow_due) || (!periph && !fast_dirty) {
                    continue;
                }
                let set_mode = sent.get(&d.idx) != Some(&(*m, p.brightness));
                if c.apply_ex(d, *m, p.color, p.color2, p.brightness, false, set_mode).is_err() {
                    failed = true;
                    break;
                }
                sent.insert(d.idx, (*m, p.brightness));
            }
            if failed {
                client = None;
                sent.clear();
                if !open {
                    break;
                }
                continue;
            }
            fast_dirty = false;
            if slow_due {
                slow_dirty = false;
                last_slow = Instant::now();
            }
        }
    });
    tx
}

impl RgbUi {
    pub fn new() -> Self {
        let autostart = Arc::new(Mutex::new(None));
        let a2 = autostart.clone();
        std::thread::spawn(move || *a2.lock().unwrap() = Some(rgb::autostart_enabled()));
        // start from the last color applied, if any
        let saved = rgb::load_saved();
        let last = saved.last().map(|s| (s.color, s.brightness));
        Self {
            color: last.map_or(0xFF2D55, |l| l.0),
            brightness: last.map_or(100, |l| l.1.clamp(0, 100)),
            modes: HashMap::new(),
            off: HashSet::new(),
            save_device: true,
            msg: Arc::new(Mutex::new(String::new())),
            autostart,
            busy: Arc::new(Mutex::new(false)),
            favorites: rgb::load_favorites(),
            live: true,
            grad: saved.last().and_then(|s| s.color2),
            slot: 0,
            sent: Mutex::new(0),
            preview: Mutex::new(None),
            seed: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0x9E37_79B9, |d| d.subsec_nanos() | 1),
        }
    }
}

impl RgbUi {
    /// (color 1, color 2 of the gradient)
    pub fn colors(&self) -> (u32, Option<u32>) {
        match self.grad {
            None => (self.color, None),
            Some(o) if self.slot == 0 => (self.color, Some(o)),
            Some(o) => (o, Some(self.color)),
        }
    }

    /// A random vivid color.
    fn random(&mut self) -> u32 {
        let mut x = self.seed;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.seed = x;
        hsv((x % 360) as f32, 0.75 + (x >> 9 & 63) as f32 / 252.0, 1.0)
    }

    fn set_channel(&mut self, shift: u32, v: i32) {
        let v = v.clamp(0, 255) as u32;
        self.color = (self.color & !(0xFF << shift)) | (v << shift);
    }
}

const PRESETS: [u32; 12] = [0xFF0000, 0xFF2D55, 0xFF7A00, 0xFFD000, 0x7CFF00, 0x00FF6A, 0x00FFD5, 0x00A2FF, 0x1E3BFF, 0x8A2BFF, 0xFF00D4, 0xFFFFFF];

fn hsv(h: f32, s: f32, v: f32) -> u32 {
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    let q = |f: f32| (((f + m) * 255.0).round() as u32).min(255);
    (q(r) << 16) | (q(g) << 8) | q(b)
}

fn swatch(id: SharedString, col: u32, on: bool, w: f32) -> Stateful<Div> {
    div()
        .id(id)
        .w(px(w))
        .h(px(22.))
        .rounded(px(3.))
        .border_2()
        .border_color(if on { c(TEXT) } else { gpui::rgba(0) })
        .bg(gpui::rgb(col))
        .cursor_pointer()
}

impl Dashboard {
    pub(crate) fn open_rgb(&mut self) {
        let admin = self.cfg.lock().unwrap().sensors_admin;
        rgb::global().start(admin);
        self.rgb = Some(RgbUi::new());
    }

    fn rgb_apply(&mut self) {
        let Some(ui) = self.rgb.as_ref() else { return };
        if *ui.busy.lock().unwrap() {
            return;
        }
        let devs: Vec<(rgb::Device, usize)> = rgb::global()
            .devices()
            .into_iter()
            .filter(|d| !ui.off.contains(&d.idx))
            .map(|d| {
                let m = ui.modes.get(&d.idx).copied().unwrap_or_else(|| rgb::default_mode(&d));
                (d, m)
            })
            .collect();
        let ((color, color2), bright, save) = (ui.colors(), ui.brightness, ui.save_device);
        let keep = ui.autostart.lock().unwrap().unwrap_or(false);
        let (msg, busy) = (ui.msg.clone(), ui.busy.clone());
        *busy.lock().unwrap() = true;
        *msg.lock().unwrap() = "aplicando…".into();
        std::thread::spawn(move || {
            let r = (|| -> Result<String, String> {
                let mut c = rgb::Client::connect(rgb::PORT)?;
                let mut fails = Vec::new();
                for (d, m) in &devs {
                    if let Err(e) = c.apply2(d, *m, color, color2, bright, save) {
                        fails.push(format!("{}: {e}", d.name));
                    }
                }
                // Mark II's own record (rgb.json) is what `--apply-rgb` reapplies at login
                let saved: Vec<rgb::Saved> = devs
                    .iter()
                    .map(|(d, m)| rgb::Saved {
                        name: d.name.clone(),
                        location: d.location.clone(),
                        mode: d.modes.get(*m).map(|x| x.name.clone()).unwrap_or_default(),
                        color,
                        brightness: bright,
                        color2,
                    })
                    .collect();
                rgb::remember(&saved)?;
                if keep {
                    rgb::enable_autostart()?;
                }
                for (d, _) in &devs {
                    rgb::global().refresh(d.idx);
                }
                Ok(if fails.is_empty() { format!("aplicado em {} dispositivo(s)", devs.len()) } else { fails.join(" · ") })
            })();
            if r.as_ref().is_ok_and(|m| m.starts_with("aplicado")) {
                crate::sound::play(crate::sound::Sound::Ok);
            }
            *msg.lock().unwrap() = r.unwrap_or_else(|e| format!("erro: {e}"));
            *busy.lock().unwrap() = false;
        });
    }

    fn rgb_set_autostart(&mut self, on: bool) {
        let Some(ui) = self.rgb.as_ref() else { return };
        let (msg, auto) = (ui.msg.clone(), ui.autostart.clone());
        *msg.lock().unwrap() = if on { "criando a entrada de login…".into() } else { "removendo…".into() };
        std::thread::spawn(move || {
            let r = if on { rgb::enable_autostart() } else { rgb::disable_autostart() };
            *auto.lock().unwrap() = Some(rgb::autostart_enabled());
            *msg.lock().unwrap() = match r {
                Ok(()) if on => "as cores serão reaplicadas ao ligar o PC, sem abrir o Mark II".into(),
                Ok(()) => "não reaplica mais ao ligar o PC".into(),
                Err(e) => format!("erro: {e}"),
            };
        });
    }

    /// Sends the current color to the LEDs when it changed (live preview).
    fn rgb_preview(&self, devices: &[rgb::Device]) {
        let Some(ui) = self.rgb.as_ref() else { return };
        if !ui.live || devices.is_empty() || *ui.busy.lock().unwrap() {
            return;
        }
        let devs: Vec<(rgb::Device, usize)> = devices
            .iter()
            .filter(|d| !ui.off.contains(&d.idx))
            .map(|d| (d.clone(), ui.modes.get(&d.idx).copied().unwrap_or_else(|| rgb::default_mode(d))))
            .collect();
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (ui.colors(), ui.brightness).hash(&mut h);
        for (d, m) in &devs {
            (d.idx, *m).hash(&mut h);
        }
        let key = h.finish() | 1;
        let mut sent = ui.sent.lock().unwrap();
        if *sent == key {
            return;
        }
        if *sent == 0 {
            // first frame: don't touch the LEDs until the user changes something
            *sent = key;
            return;
        }
        *sent = key;
        let mut tx = ui.preview.lock().unwrap();
        let (color, color2) = ui.colors();
        let p = Preview { color, color2, brightness: ui.brightness, devs };
        if let Err(mpsc::SendError(p)) = tx.get_or_insert_with(preview_worker).send(p) {
            *tx = Some(preview_worker());
            let _ = tx.as_ref().unwrap().send(p);
        }
    }

    pub(crate) fn rgb_sheet(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let ui = self.rgb.as_ref().expect("rgb sheet open");
        let status = rgb::global().status();
        let devices = rgb::global().devices();
        if status == Status::Running {
            self.rgb_preview(&devices);
        }
        let running = status == Status::Running;
        let busy = *ui.busy.lock().unwrap();
        let msg = ui.msg.lock().unwrap().clone();
        let auto = *ui.autostart.lock().unwrap();
        let h3 = |t: &str| label(t).mt(px(16.)).mb(px(8.));
        let btn = |id: &'static str, txt: &str, primary: bool| {
            div()
                .id(id)
                .px(px(16.))
                .py(px(7.))
                .rounded(px(4.))
                .font_family(NUM_FONT)
                .font_weight(FontWeight::SEMIBOLD)
                .text_size(px(12.5))
                .cursor_pointer()
                .when(primary, |d| d.bg(c(AMBER)).text_color(c(0x1a1205)))
                .when(!primary, |d| d.border_1().border_color(c(LINE2)))
                .child(txt.to_string())
        };
        let check = |on: bool| {
            div()
                .size(px(15.))
                .flex_none()
                .rounded(px(3.))
                .border_1()
                .border_color(c(if on { AMBER } else { LINE2 }))
                .bg(c(if on { AMBER } else { BG }))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(11.))
                .text_color(c(0x1a1205))
                .child(if on { "✓" } else { "" })
        };

        // ---- color
        let cur = ui.color;
        let hue_row = div().flex().gap(px(2.)).children((0..24).map(|i| {
            let col = hsv(i as f32 * 15.0, 1.0, 1.0);
            swatch(format!("hue{i}").into(), col, col == cur, 18.)
                .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
                    if let Some(u) = this.rgb.as_mut() {
                        u.color = col;
                    }
                    cx.notify();
                }))
                .on_mouse_move(cx.listener(move |this, e: &MouseMoveEvent, _, cx| {
                    if e.pressed_button == Some(MouseButton::Left) {
                        if let Some(u) = this.rgb.as_mut() {
                            if u.color != col {
                                u.color = col;
                                cx.notify();
                            }
                        }
                    }
                }))
        }));
        let preset_row = div().flex().gap(px(6.)).mt(px(6.)).children(PRESETS.iter().enumerate().map(|(i, &col)| {
            swatch(format!("pre{i}").into(), col, col == cur, 30.).on_click(cx.listener(move |this, _, _, cx| {
                if let Some(u) = this.rgb.as_mut() {
                    u.color = col;
                }
                cx.notify();
            }))
        }));
        let step = |id: &'static str, txt: &'static str, d: i32| {
            div()
                .id(id)
                .w(px(24.))
                .h(px(24.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.))
                .border_1()
                .border_color(c(LINE2))
                .cursor_pointer()
                .hover(|s| s.bg(c(PANEL2)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(u) = this.rgb.as_mut() {
                        u.brightness = (u.brightness as i32 + d).clamp(0, 100) as u8;
                    }
                    cx.notify();
                }))
                .child(txt)
        };
        // ---- manual color: R, G and B, each a clickable 0-255 strip with -/+ for fine steps
        let chan = |name: &'static str, shift: u32| {
            let v = ((cur >> shift) & 0xFF) as i32;
            let cells = 32;
            let fine = |sfx: &'static str, txt: &'static str, d: i32| {
                div()
                    .id(SharedString::from(format!("ch{shift}{sfx}")))
                    .w(px(20.))
                    .h(px(18.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(3.))
                    .border_1()
                    .border_color(c(LINE2))
                    .cursor_pointer()
                    .hover(|s| s.bg(c(PANEL2)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(u) = this.rgb.as_mut() {
                            let now = ((u.color >> shift) & 0xFF) as i32;
                            u.set_channel(shift, now + d);
                        }
                        cx.notify();
                    }))
                    .child(txt)
            };
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(div().w(px(12.)).text_size(px(12.)).text_color(c(MUTED)).child(name))
                .child(div().flex().children((0..cells).map(|i| {
                    let val = (i * 255 / (cells - 1)) as i32;
                    let col = (cur & !(0xFF << shift)) | ((val as u32) << shift);
                    let here = v * (cells - 1) as i32 / 255 == i as i32;
                    div()
                        .id(SharedString::from(format!("ch{shift}-{i}")))
                        .w(px(9.))
                        .h(px(16.))
                        .bg(gpui::rgb(col))
                        .when(here, |d| d.border_1().border_color(c(TEXT)))
                        .cursor_pointer()
                        .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
                            if let Some(u) = this.rgb.as_mut() {
                                u.set_channel(shift, val);
                            }
                            cx.notify();
                        }))
                        // drag along the strip
                        .on_mouse_move(cx.listener(move |this, e: &MouseMoveEvent, _, cx| {
                            if e.pressed_button == Some(MouseButton::Left) {
                                if let Some(u) = this.rgb.as_mut() {
                                    if (u.color >> shift) & 0xFF != val as u32 {
                                        u.set_channel(shift, val);
                                        cx.notify();
                                    }
                                }
                            }
                        }))
                })))
                .child(fine("m", "−", -1))
                .child(num(format!("{v}"), 12.5).w(px(30.)).flex().justify_center().text_color(c(TEXT)))
                .child(fine("p", "+", 1))
        };
        let action = |id: &'static str, txt: &'static str| {
            div()
                .id(id)
                .px(px(10.))
                .py(px(4.))
                .rounded(px(4.))
                .border_1()
                .border_color(c(LINE2))
                .text_size(px(12.))
                .cursor_pointer()
                .hover(|s| s.bg(c(PANEL2)))
                .child(txt)
        };
        let theme_col = remap(AMBER);
        let actions = div()
            .flex()
            .flex_wrap()
            .gap(px(6.))
            .mt(px(8.))
            .child(action("rgb-theme", "Cor do tema").on_click(cx.listener(move |this, _, _, cx| {
                if let Some(u) = this.rgb.as_mut() {
                    u.color = theme_col;
                }
                cx.notify();
            })))
            .child(action("rgb-rand", "Aleatória").on_click(cx.listener(|this, _, _, cx| {
                if let Some(u) = this.rgb.as_mut() {
                    u.color = u.random();
                }
                cx.notify();
            })))
            .child(action("rgb-black", "Apagar LEDs").on_click(cx.listener(|this, _, _, cx| {
                if let Some(u) = this.rgb.as_mut() {
                    u.color = 0x000000;
                    u.grad = None;
                    u.slot = 0;
                }
                this.rgb_apply();
                cx.notify();
            })))
            .child(action("rgb-fav", "★ Salvar cor").on_click(cx.listener(|this, _, _, cx| {
                if let Some(u) = this.rgb.as_mut() {
                    let col = u.color;
                    u.favorites.retain(|f| *f != col);
                    u.favorites.insert(0, col);
                    u.favorites.truncate(12);
                    let r = rgb::save_favorites(&u.favorites);
                    *u.msg.lock().unwrap() = match r {
                        Ok(()) => format!("#{col:06X} salva nas favoritas"),
                        Err(e) => format!("erro: {e}"),
                    };
                }
                cx.notify();
            })));
        let favs = ui.favorites.clone();
        let fav_row = (!favs.is_empty()).then(|| {
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .mt(px(8.))
                .child(small("Favoritas".to_string(), MUTED))
                .children(favs.into_iter().enumerate().map(|(i, col)| {
                    swatch(format!("fav{i}").into(), col, col == cur, 26.)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(u) = this.rgb.as_mut() {
                                u.color = col;
                            }
                            cx.notify();
                        }))
                        // right click removes it
                        .on_mouse_down(MouseButton::Right, cx.listener(move |this, _, _, cx| {
                            if let Some(u) = this.rgb.as_mut() {
                                u.favorites.retain(|f| *f != col);
                                let _ = rgb::save_favorites(&u.favorites);
                            }
                            cx.notify();
                        }))
                }))
                .child(small("(botão direito remove)".to_string(), DIM))
        });
        // ---- gradient of two colors
        let (ga, gb) = ui.colors();
        let slot = ui.slot;
        let pick = |id: &'static str, n: u8, col: u32| {
            let on = slot == n;
            div()
                .id(id)
                .flex()
                .items_center()
                .gap(px(6.))
                .px(px(8.))
                .py(px(3.))
                .rounded(px(4.))
                .border_1()
                .border_color(c(if on { TEXT } else { LINE2 }))
                .cursor_pointer()
                .text_size(px(12.))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(u) = this.rgb.as_mut() {
                        if u.slot != n {
                            // edit the other color: swap it into `color`
                            if let Some(o) = u.grad {
                                u.grad = Some(u.color);
                                u.color = o;
                                u.slot = n;
                            }
                        }
                    }
                    cx.notify();
                }))
                .child(div().size(px(14.)).rounded(px(3.)).bg(gpui::rgb(col)))
                .child(if n == 0 { "Cor 1" } else { "Cor 2" })
        };
        let grad_on = gb.is_some();
        let grad_row = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .mt(px(6.))
            .child(
                div()
                    .id("rgb-grad")
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .cursor_pointer()
                    .text_size(px(12.))
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(u) = this.rgb.as_mut() {
                            if u.grad.is_some() {
                                // back to one color: keep color 1
                                u.color = u.colors().0;
                                u.grad = None;
                                u.slot = 0;
                            } else {
                                // start with a contrasting second color
                                let (r, g, b) = ((u.color >> 16) & 255, (u.color >> 8) & 255, u.color & 255);
                                u.grad = Some(if u.color == 0 { 0x00A2FF } else { (b << 16) | (r << 8) | g });
                                u.slot = 0;
                            }
                        }
                        cx.notify();
                    }))
                    .child(check(grad_on))
                    .child("Degradê de 2 cores"),
            )
            .when(grad_on, |d| {
                d.child(pick("rgb-c1", 0, ga))
                    .child(pick("rgb-c2", 1, gb.unwrap_or(ga)))
                    .child(
                        div()
                            .id("rgb-swap")
                            .px(px(8.))
                            .py(px(3.))
                            .rounded(px(4.))
                            .border_1()
                            .border_color(c(LINE2))
                            .cursor_pointer()
                            .text_size(px(12.))
                            .hover(|s| s.bg(c(PANEL2)))
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(u) = this.rgb.as_mut() {
                                    if let Some(o) = u.grad {
                                        u.grad = Some(u.color);
                                        u.color = o;
                                    }
                                }
                                cx.notify();
                            }))
                            .child("⇄ inverter"),
                    )
                    .child(
                        div()
                            .flex_1()
                            .h(px(14.))
                            .rounded(px(3.))
                            .bg(gpui::linear_gradient(90., gpui::linear_color_stop(gpui::rgb(ga), 0.), gpui::linear_color_stop(gpui::rgb(gb.unwrap_or(ga)), 1.))),
                    )
            });
        let manual = div()
            .mt(px(10.))
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(14.))
                    .child(small("Cor manual (clique ou arraste)".to_string(), MUTED))
                    .child(
                        div()
                            .id("rgb-live")
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .ml_auto()
                            .cursor_pointer()
                            .text_size(px(12.))
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(u) = this.rgb.as_mut() {
                                    u.live = !u.live;
                                }
                                cx.notify();
                            }))
                            .child(check(ui.live))
                            .child("Ver nos LEDs ao vivo"),
                    ),
            )
            .child(chan("R", 16))
            .child(chan("G", 8))
            .child(chan("B", 0))
            .child(grad_row)
            .child(actions)
            .children(fav_row);

        let color_block = div()
            .flex()
            .gap(px(14.))
            .items_center()
            .child({
                let (a, b) = ui.colors();
                let sw = div().size(px(56.)).flex_none().rounded(px(6.)).border_1().border_color(c(LINE2));
                match b {
                    Some(b) => sw.bg(gpui::linear_gradient(
                        135.,
                        gpui::linear_color_stop(gpui::rgb(a), 0.),
                        gpui::linear_color_stop(gpui::rgb(b), 1.),
                    )),
                    None => sw.bg(gpui::rgb(a)),
                }
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(hue_row)
                    .child(preset_row)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .text_size(px(12.5))
                            .text_color(c(MUTED))
                            .child(div().font_family(NUM_FONT).child(format!("#{cur:06X}")))
                            .child(div().ml(px(14.)).child("Brilho"))
                            .child(step("b-", "−", -10))
                            .child(num(format!("{}%", ui.brightness), 13.).w(px(46.)).flex().justify_center().text_color(c(TEXT)))
                            .child(step("b+", "+", 10)),
                    ),
            );

        // ---- devices
        let mut groups: BTreeMap<&str, Vec<&rgb::Device>> = BTreeMap::new();
        for d in &devices {
            groups.entry(d.group()).or_default().push(d);
        }
        let dev_row = |d: &rgb::Device| {
            let idx = d.idx;
            let on = !ui.off.contains(&idx);
            let m = ui.modes.get(&idx).copied().unwrap_or_else(|| rgb::default_mode(d));
            let n = d.modes.len().max(1);
            let mode = d.modes.get(m);
            let mname = mode.map_or("—".to_string(), |m| m.name.clone());
            let note = match mode {
                _ if ui.grad.is_some() && d.leds < 2 => " · 1 LED: sem degradê, usa a cor 1",
                Some(m) if !m.takes_color() => " · efeito sem cor",
                Some(m) if ui.grad.is_some() && m.color_mode != rgb::COLORS_PER_LED && m.colors_max < 2 => " · este efeito usa só a cor 1",
                Some(m) if m.can_save() => " · salva no dispositivo",
                _ => "",
            };
            let arrow = |sfx: &str, txt: &'static str, delta: usize| {
                div()
                    .id(SharedString::from(format!("m{idx}{sfx}")))
                    .px(px(6.))
                    .cursor_pointer()
                    .text_color(c(MUTED))
                    .hover(|s| s.text_color(c(TEXT)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(u) = this.rgb.as_mut() {
                            u.modes.insert(idx, (m + delta) % n);
                        }
                        cx.notify();
                    }))
                    .child(txt)
            };
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .py(px(6.))
                .border_b_1()
                .border_color(c(LINE))
                .child(
                    div()
                        .id(SharedString::from(format!("d{idx}")))
                        .flex()
                        .flex_1()
                        .min_w_0()
                        .items_center()
                        .gap(px(8.))
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(u) = this.rgb.as_mut() {
                                if !u.off.remove(&idx) {
                                    u.off.insert(idx);
                                }
                            }
                            cx.notify();
                        }))
                        .child(check(on))
                        .child(
                            div()
                                .min_w_0()
                                .child(div().text_size(px(12.5)).child(d.name.clone()))
                                .child(small(format!("{}{} · {} LEDs{note}", d.vendor, if d.location.is_empty() { String::new() } else { format!(" · {}", d.location) }, d.leds), DIM)),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .border_1()
                        .border_color(c(LINE2))
                        .rounded(px(4.))
                        .text_size(px(12.))
                        .child(arrow("p", "◀", n - 1))
                        .child(div().w(px(110.)).flex().justify_center().overflow_hidden().child(mname))
                        .child(arrow("n", "▶", 1)),
                )
        };

        let hint = if cfg!(windows) {
            "Memória RAM, placa de vídeo e placa-mãe precisam do driver PawnIO e de administrador (ative \"Pedir administrador\" em Configurações). Ventoinhas, coolers e fitas via USB funcionam sem."
        } else {
            "Linux: rode uma vez  sudo ./OpenRGB.AppImage --generate-udev-rules  (veja o LEIA-ME) e carregue i2c-dev para a RAM e a placa de vídeo."
        };

        let body = match &status {
            Status::Running | Status::Starting if !devices.is_empty() => div()
                .child(h3("Cor"))
                .child(color_block)
                .child(manual)
                .child(h3("Dispositivos"))
                .children(groups.into_iter().map(|(g, ds)| div().mb(px(8.)).child(small(g.to_string(), MUTED).font_weight(FontWeight::SEMIBOLD)).children(ds.into_iter().map(&dev_row)))),
            Status::Starting | Status::Off => div().py(px(24.)).flex().justify_center().child(small(status.text(), MUTED)),
            Status::Running => div().py(px(16.)).child(small("Nenhum dispositivo RGB encontrado.", TEXT)).child(small(hint, MUTED).mt(px(6.))),
            _ => div()
                .py(px(16.))
                .child(small(status.text(), RED))
                .child(btn("rgb-retry", "Tentar de novo", false).mt(px(10.)).on_click(cx.listener(|this, _, _, cx| {
                    this.open_rgb();
                    cx.notify();
                }))),
        };

        let k = crate::view::sheet_appear("rgb");
        div()
            .id("rgb-modal")
            .absolute()
            .inset_0()
            .bg(ca(0, 0.65 * k))
            .opacity(k)
            .pt(px((1.0 - k) * 24.))
            .flex()
            .items_center()
            .justify_center()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .id("rgb-sheet")
                    .w(px(640.))
                    .max_h(gpui::relative(0.92))
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll[2].handle)
                    .bg(c(PANEL))
                    .relative()
                    .border_1()
                    .border_color(c(LINE2))
                    .rounded(px(6.))
                    .px(px(18.))
                    .pt(px(14.))
                    .pb(px(16.))
                    .child(div().absolute().top(px(-1.)).left(px(-1.)).right(px(-1.)).h(px(2.)).bg(c(PINK)))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .child(num("Controle de RGB", 15.))
                            .child(small(format!("OpenRGB · {}", status.text()), if running { GREEN } else { MUTED }))
                            .child(
                                div()
                                    .id("rgb-close")
                                    .ml_auto()
                                    .px(px(8.))
                                    .cursor_pointer()
                                    .text_color(c(MUTED))
                                    .hover(|s| s.text_color(c(TEXT)))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        // colors stay on the hardware; OpenRGB does not stay running
                                        rgb::global().stop();
                                        this.rgb = None;
                                        cx.notify();
                                    }))
                                    .child("✕"),
                            ),
                    )
                    .child(body)
                    .when(!devices.is_empty(), |d| {
                        d.child(h3("Manter as cores"))
                            .child(
                                div()
                                    .id("rgb-save-dev")
                                    .flex()
                                    .items_center()
                                    .gap(px(8.))
                                    .py(px(4.))
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if let Some(u) = this.rgb.as_mut() {
                                            u.save_device = !u.save_device;
                                        }
                                        cx.notify();
                                    }))
                                    .child(check(ui.save_device))
                                    .child("Gravar na memória do dispositivo (quando ele suporta)"),
                            )
                            .child(
                                div()
                                    .id("rgb-auto")
                                    .flex()
                                    .items_center()
                                    .gap(px(8.))
                                    .py(px(4.))
                                    .cursor_pointer()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if let Some(on) = auto {
                                            this.rgb_set_autostart(!on);
                                        }
                                        cx.notify();
                                    }))
                                    .child(check(auto == Some(true)))
                                    .child(if auto.is_none() { "Reaplicar ao ligar o PC (verificando…)" } else { "Reaplicar ao ligar o PC, sem abrir o Mark II" }),
                            )
                            .when(auto == Some(true), |d| {
                                d.child(
                                    div().flex().items_center().gap(px(10.)).mt(px(6.)).mb(px(4.)).child(
                                        btn("rgb-del-task", if cfg!(windows) { "Excluir tarefa \"Mark II RGB\"" } else { "Excluir entrada de login" }, false)
                                            .text_color(c(RED))
                                            .hover(|s| s.bg(c(PANEL2)))
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.rgb_set_autostart(false);
                                                cx.notify();
                                            })),
                                    ),
                                )
                            })
                            .child(small(
                                if cfg!(windows) {
                                    "Cria a tarefa \"Mark II RGB\" no Agendador do Windows, que reaplica as últimas cores aplicadas (rgb.json) no login, sem abrir a janela, e fecha em seguida. Pede administrador uma vez."
                                } else {
                                    "Cria ~/.config/autostart/markii-rgb.desktop, que reaplica as últimas cores aplicadas (rgb.json) no login e fecha em seguida."
                                },
                                DIM,
                            ))
                            .child(small(hint, DIM).mt(px(4.)))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(10.))
                                    .mt(px(16.))
                                    .child(div().flex_1().min_w_0().text_size(px(12.)).text_color(c(if msg.starts_with("erro") { RED } else { GREEN })).child(msg))
                                    .child(btn("rgb-apply", if busy { "Aplicando…" } else { "Aplicar" }, true).on_click(cx.listener(|this, _, _, cx| {
                                        this.rgb_apply();
                                        cx.notify();
                                    }))),
                            )
                    }),
            )
    }
}
