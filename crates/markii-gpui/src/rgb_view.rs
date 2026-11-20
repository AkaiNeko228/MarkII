//! RGB controller sheet (OpenRGB through markii_core::rgb).
use crate::theme::*;
use crate::view::{label, num, small, Dashboard};
use gpui::{div, prelude::*, px, Context, Div, FontWeight, MouseButton, SharedString, Stateful};
use markii_core::rgb::{self, Status};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex};

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
}

impl RgbUi {
    pub fn new() -> Self {
        let autostart = Arc::new(Mutex::new(None));
        let a2 = autostart.clone();
        std::thread::spawn(move || *a2.lock().unwrap() = Some(rgb::autostart_enabled()));
        Self {
            color: 0xFF2D55,
            brightness: 100,
            modes: HashMap::new(),
            off: HashSet::new(),
            save_device: true,
            msg: Arc::new(Mutex::new(String::new())),
            autostart,
            busy: Arc::new(Mutex::new(false)),
        }
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
        let (color, bright, save) = (ui.color, ui.brightness, ui.save_device);
        let keep = ui.autostart.lock().unwrap().unwrap_or(false);
        let (msg, busy) = (ui.msg.clone(), ui.busy.clone());
        *busy.lock().unwrap() = true;
        *msg.lock().unwrap() = "aplicando…".into();
        std::thread::spawn(move || {
            let r = (|| -> Result<String, String> {
                let mut c = rgb::Client::connect(rgb::PORT)?;
                let mut fails = Vec::new();
                for (d, m) in &devs {
                    if let Err(e) = c.apply(d, *m, color, bright, save) {
                        fails.push(format!("{}: {e}", d.name));
                    }
                }
                // the profile is what the login entry reapplies
                c.save_profile(rgb::PROFILE)?;
                if keep {
                    rgb::enable_autostart(&mut c)?;
                }
                for (d, _) in &devs {
                    rgb::global().refresh(d.idx);
                }
                Ok(if fails.is_empty() { format!("aplicado em {} dispositivo(s)", devs.len()) } else { fails.join(" · ") })
            })();
            *msg.lock().unwrap() = r.unwrap_or_else(|e| format!("erro: {e}"));
            *busy.lock().unwrap() = false;
        });
    }

    fn rgb_set_autostart(&mut self, on: bool) {
        let Some(ui) = self.rgb.as_ref() else { return };
        let (msg, auto) = (ui.msg.clone(), ui.autostart.clone());
        *msg.lock().unwrap() = if on { "salvando o perfil e criando a entrada de login…".into() } else { "removendo…".into() };
        std::thread::spawn(move || {
            let r = if on { rgb::Client::connect(rgb::PORT).and_then(|mut c| rgb::enable_autostart(&mut c)) } else { rgb::disable_autostart() };
            *auto.lock().unwrap() = Some(rgb::autostart_enabled());
            *msg.lock().unwrap() = match r {
                Ok(()) if on => "as cores serão reaplicadas ao ligar o PC, sem abrir o Mark II".into(),
                Ok(()) => "não reaplica mais ao ligar o PC".into(),
                Err(e) => format!("erro: {e}"),
            };
        });
    }

    pub(crate) fn rgb_sheet(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let ui = self.rgb.as_ref().expect("rgb sheet open");
        let status = rgb::global().status();
        let devices = rgb::global().devices();
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
            swatch(format!("hue{i}").into(), col, col == cur, 18.).on_click(cx.listener(move |this, _, _, cx| {
                if let Some(u) = this.rgb.as_mut() {
                    u.color = col;
                }
                cx.notify();
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
        let color_block = div()
            .flex()
            .gap(px(14.))
            .items_center()
            .child(div().size(px(56.)).flex_none().rounded(px(6.)).border_1().border_color(c(LINE2)).bg(gpui::rgb(cur)))
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
                Some(m) if !m.takes_color() => " · efeito sem cor",
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

        div()
            .id("rgb-modal")
            .absolute()
            .inset_0()
            .bg(ca(0, 0.65))
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
                            .child(small(
                                if cfg!(windows) {
                                    "Cria a tarefa \"Mark II RGB\" no Agendador do Windows, que roda o OpenRGB uma vez no login com o perfil salvo. Pede administrador uma vez."
                                } else {
                                    "Cria ~/.config/autostart/markii-rgb.desktop, que roda o OpenRGB uma vez no login com o perfil salvo."
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
