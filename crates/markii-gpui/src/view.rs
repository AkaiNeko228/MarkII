//! The Mark II dashboard view.
use crate::draw;
use crate::theme::*;
use gpui::{
    div, prelude::*, px, AnyElement, Context, Div, FontWeight, MouseButton, SharedString, Stateful, Window,
};
use markii_core::{pct, Config, PcInfo, Telemetry};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub const HIST: usize = 120;

#[derive(Default)]
pub struct Hist {
    pub cpu: VecDeque<Option<f64>>,
    pub gpu: VecDeque<Option<f64>>,
    pub mem: VecDeque<Option<f64>>,
    pub net: VecDeque<Option<f64>>,
    pub cput: VecDeque<Option<f64>>,
    pub gput: VecDeque<Option<f64>>,
}

fn push(q: &mut VecDeque<Option<f64>>, v: Option<f64>) {
    q.push_back(v);
    while q.len() > HIST {
        q.pop_front();
    }
}

pub struct Dashboard {
    pub t: Option<Telemetry>,
    pub pc: Option<PcInfo>,
    pub cfg: Arc<Mutex<Config>>,
    pub cfg_path: PathBuf,
    pub hist: Hist,
    pub fan_angle: f32,
    pub gpu_fan_angle: f32,
    pub pump_angle: f32,
    pub settings: Option<Config>,
    pub save_msg: SharedString,
    pub rgb: Option<crate::rgb_view::RgbUi>,
    /// Windows firewall state for the LAN server: (rule present?, last message).
    pub fw: Arc<Mutex<(Option<bool>, String)>>,
}

// ---------- formatting ----------
fn f0(v: Option<f64>) -> String {
    v.filter(|v| v.is_finite()).map_or("—".into(), |v| format!("{:.0}", v))
}
pub(crate) fn gb(b: u64) -> String {
    let g = b as f64 / 1073741824.0;
    if g >= 10.0 { format!("{:.0}", g) } else { format!("{:.1}", g) }
}
fn rate(mut bps: f64) -> (String, &'static str) {
    let u = ["B/s", "KB/s", "MB/s", "GB/s"];
    let mut i = 0;
    while bps >= 1000.0 && i < u.len() - 1 {
        bps /= 1024.0;
        i += 1;
    }
    (if bps >= 100.0 || i == 0 { format!("{:.0}", bps) } else { format!("{:.1}", bps) }, u[i])
}
fn uptime(s: u64) -> String {
    let (d, h, m) = (s / 86400, s % 86400 / 3600, s % 3600 / 60);
    if d > 0 { format!("{d}d {h}h {m}m") } else { format!("{h}h {m}m") }
}
pub(crate) fn clean_cpu(n: &str) -> String {
    let n = n.replace("(R)", "").replace("(TM)", "").replace("CPU", "");
    let n = n.split('@').next().unwrap_or(&n).to_string();
    n.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ---------- building blocks ----------
pub(crate) fn num(text: impl Into<SharedString>, size: f32) -> Div {
    div().font_family(NUM_FONT).font_weight(FontWeight::SEMIBOLD).text_size(px(size)).child(text.into())
}
pub(crate) fn small(text: impl Into<SharedString>, color: u32) -> Div {
    div().text_size(px(11.5)).text_color(c(color)).child(text.into())
}
pub(crate) fn label(text: &str) -> Div {
    div().font_family(NUM_FONT).font_weight(FontWeight::SEMIBOLD).text_size(px(10.5)).text_color(c(MUTED)).child(text.to_uppercase())
}

/// Panel with an accent tick and an uppercase header.
pub(crate) fn panel(accent: u32, title: impl Into<SharedString>, tag: impl Into<SharedString>, alert: bool) -> Div {
    let tag: SharedString = tag.into();
    panel_el(accent, title, div().text_size(px(12.)).text_color(c(MUTED)).child(tag).into_any_element(), alert)
}

/// Same as [`panel`] with any element (e.g. a toggle) on the right of the header.
pub(crate) fn panel_el(accent: u32, title: impl Into<SharedString>, right: AnyElement, alert: bool) -> Div {
    panel_logo(accent, None, title, right, alert)
}

/// Panel whose header starts with a brand mark.
pub(crate) fn panel_logo(accent: u32, logo: Option<AnyElement>, title: impl Into<SharedString>, right: AnyElement, alert: bool) -> Div {
    div()
        .relative()
        .flex()
        .flex_col()
        .min_w_0()
        .bg(c(PANEL))
        .border_1()
        .border_color(if alert { ca(RED, 0.6) } else { c(LINE) })
        .rounded(px(6.))
        .px(px(14.))
        .pt(px(12.))
        .pb(px(14.))
        .child(div().absolute().left(px(-1.)).top(px(12.)).w(px(3.)).h(px(16.)).bg(c(accent)))
        .child(
            div()
                .flex()
                .items_center()
                .mb(px(12.))
                .gap(px(8.))
                .children(logo)
                .child(div().flex_1().min_w_0().font_family(NUM_FONT).font_weight(FontWeight::SEMIBOLD).text_size(px(11.5)).child(title.into()))
                .child(div().ml(px(8.)).child(right)),
        )
}

pub(crate) fn segbar(p: f64, n: usize, color: u32, h: f32) -> Div {
    let on = ((p.clamp(0.0, 100.0) / 100.0) * n as f64).round() as usize;
    div().flex().gap(px(1.5)).mt(px(4.)).children((0..n).map(move |i| {
        div().flex_1().h(px(h)).rounded(px(1.)).bg(if i < on { c(color) } else { c(LINE) })
    }))
}

pub(crate) fn row(k: impl Into<SharedString>, v: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .py(px(7.))
        .border_b_1()
        .border_color(c(LINE))
        .text_size(px(12.5))
        .child(div().flex_1().min_w_0().overflow_hidden().child(k.into()))
        .child(div().font_family(NUM_FONT).text_color(c(MUTED)).child(v.into()))
}

impl Dashboard {
    /// First part of "Criar log": what this PC is and how Mark II is set up.
    pub fn log_header(&self) -> String {
        let cfg = self.cfg.lock().unwrap().clone();
        let t = self.t.as_ref();
        let r = markii_core::remote::global().info();
        let yn = |b: bool| if b { "sim" } else { "não" };
        let mut h = format!(
            "Mark II v{} · log criado em {}
",
            env!("CARGO_PKG_VERSION"),
            markii_core::log::now()
        );
        h += &format!("Sistema: {} {}
", std::env::consts::OS, sysinfo::System::long_os_version().unwrap_or_default());
        if let Some(t) = t {
            h += &format!("CPU: {}
GPU: {}
", t.cpu.name, if t.gpu.name.is_empty() { "?" } else { &t.gpu.name });
            h += &format!("RAM: {} GB
", crate::view::gb(t.mem.total));
        }
        h += &format!("Sensores: {:?}
", markii_core::lhm::global().status());
        h += &format!("RGB (OpenRGB): {:?}
", markii_core::rgb::global().status());
        h += &format!("Modo VR: {} (processo ativo: {})
", yn(cfg.vr_overlay), yn(crate::vr::global().running()));
        h += &format!(
            "Rede local: {} · ativa: {} · porta {} · {} aparelho(s) pareado(s){}{}
",
            yn(cfg.remote_enabled),
            yn(r.running),
            cfg.remote_port,
            r.paired,
            r.connected.map(|n| format!(" · conectado: {n}")).unwrap_or_default(),
            r.error.map(|e| format!(" · erro: {e}")).unwrap_or_default()
        );
        if cfg.remote_enabled {
            h += &format!("Firewall liberado: {}
", yn(markii_core::remote::firewall_ok()));
        }
        h += &format!(
            "Config: tema {}, intervalo {} ms, pedir administrador {}, alertas {}
",
            cfg.theme,
            cfg.interval_ms,
            yn(cfg.sensors_admin),
            yn(cfg.alerts_enabled)
        );
        h
    }

    /// Re-reads whether Mark II's firewall rule exists (in the background; netsh is slow-ish).
    pub fn check_fw(&self) {
        if !markii_core::remote::global().running() && !self.cfg.lock().unwrap().remote_enabled {
            return;
        }
        let fw = self.fw.clone();
        std::thread::spawn(move || {
            let ok = markii_core::remote::firewall_ok();
            fw.lock().unwrap().0 = Some(ok);
        });
    }

    pub fn ingest(&mut self, t: Telemetry) {
        let h = &mut self.hist;
        push(&mut h.cpu, Some(t.cpu.load as f64));
        push(&mut h.gpu, t.gpu.load.map(|v| v as f64));
        push(&mut h.mem, t.mem_pct());
        push(&mut h.net, Some(t.net.rx_bps));
        push(&mut h.cput, t.cpu.package_temp);
        push(&mut h.gput, t.gpu.temp.map(|v| v as f64));
        self.t = Some(t);
    }

    /// Advance sprite animations by `dt` seconds.
    pub fn animate(&mut self, dt: f32) {
        let Some(t) = &self.t else { return };
        let rpm_cpu = t.cooling.cpu_fan_rpm.unwrap_or(if t.cooling.kind == "unknown" { 0.0 } else { 900.0 });
        let rpm_pump = t.cooling.pump_rpm.unwrap_or(1500.0);
        let gpu = t.gpu.fan.unwrap_or(0) as f32 / 100.0 * 1800.0;
        // Rotation is slowed down 8x so it reads as motion rather than strobing.
        let step = |rpm: f64| (rpm as f32 / 60.0 / 8.0) * 2.0 * std::f32::consts::PI * dt;
        self.fan_angle = (self.fan_angle + step(rpm_cpu)) % (2.0 * std::f32::consts::PI);
        self.pump_angle = (self.pump_angle + step(rpm_pump)) % (2.0 * std::f32::consts::PI);
        self.gpu_fan_angle = (self.gpu_fan_angle + step(gpu as f64)) % (2.0 * std::f32::consts::PI);
    }

    fn cfg(&self) -> Config {
        self.cfg.lock().unwrap().clone()
    }

    fn set_rate(&mut self, ms: u64) {
        let mut c = self.cfg.lock().unwrap();
        c.interval_ms = ms;
        let _ = c.save(&self.cfg_path);
    }

    // ---------- header ----------
    fn header(&self, t: &Telemetry, cx: &mut Context<Self>) -> Div {
        let rate_ms = self.cfg().interval_ms;
        let seg = div().flex().border_1().border_color(c(LINE2)).rounded(px(6.)).overflow_hidden().children(
            [(500u64, "0.5s"), (1000, "1s"), (2000, "2s"), (5000, "5s")].into_iter().map(|(ms, l)| {
                let on = rate_ms == ms;
                div()
                    .id(SharedString::from(format!("rate{ms}")))
                    .px(px(13.))
                    .py(px(6.))
                    .font_family(NUM_FONT)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(12.))
                    .cursor_pointer()
                    .when(on, |d| d.bg(c(AMBER)).text_color(c(0x1a1205)))
                    .when(!on, |d| d.text_color(c(MUTED)).hover(|s| s.bg(c(PANEL2)).text_color(c(TEXT))))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_rate(ms);
                        cx.notify();
                    }))
                    .child(l)
            }),
        );
        div()
            .flex()
            .items_center()
            .justify_between()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(
                        div().size(px(40.)).rounded(px(8.)).bg(c(0x161b22)).flex().items_center().justify_center().child(
                            gpui::canvas(|_, _, _| (), |b, _, window, _| {
                                let (x, y) = (f32::from(b.origin.x), f32::from(b.origin.y));
                                let pts = [(7.5, 25.8), (13.8, 19.5), (18.5, 23.4), (25.0, 14.8), (32.5, 20.3)];
                                let mut pb = gpui::PathBuilder::stroke(px(3.2));
                                pb.move_to(gpui::point(px(x + pts[0].0), px(y + pts[0].1)));
                                for p in &pts[1..] {
                                    pb.line_to(gpui::point(px(x + p.0), px(y + p.1)));
                                }
                                if let Ok(p) = pb.build() {
                                    window.paint_path(p, c(AMBER));
                                }
                            })
                            .size(px(40.)),
                        ),
                    )
                    .child(
                        div()
                            .child(
                                div()
                                    .flex()
                                    .gap(px(8.))
                                    .font_family(NUM_FONT)
                                    .font_weight(FontWeight::BOLD)
                                    .text_size(px(26.))
                                    .child("M A R K")
                                    .child(div().text_color(c(AMBER)).child("I I")),
                            )
                            .child(
                                div()
                                    .flex()
                                    .gap(px(4.))
                                    .text_size(px(12.))
                                    .text_color(c(MUTED))
                                    .child(div().text_color(c(TEXT)).font_weight(FontWeight::SEMIBOLD).child(t.host.clone()))
                                    .child(format!("· ligado há {} · v{}", uptime(t.uptime), env!("CARGO_PKG_VERSION"))),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .items_center()
                    .child(seg)
                    .child(
                        div()
                            .id("open-settings")
                            .px(px(12.))
                            .py(px(6.))
                            .border_1()
                            .border_color(c(LINE2))
                            .rounded(px(6.))
                            .text_color(c(MUTED))
                            .text_size(px(12.))
                            .cursor_pointer()
                            .hover(|s| s.text_color(c(TEXT)).border_color(c(MUTED)))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.settings = Some(this.cfg());
                                this.save_msg = "".into();
                                this.check_fw();
                                cx.notify();
                            }))
                            .child("⚙  Configurações"),
                    )
                    .child(
                        div()
                            .id("open-rgb")
                            .px(px(12.))
                            .py(px(6.))
                            .border_1()
                            .border_color(c(LINE2))
                            .rounded(px(6.))
                            .text_color(c(MUTED))
                            .text_size(px(12.))
                            .cursor_pointer()
                            .hover(|s| s.text_color(c(TEXT)).border_color(c(MUTED)))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.open_rgb();
                                cx.notify();
                            }))
                            .child("◆  RGB"),
                    )
                    .child({
                        // VR mode on/off in one click (off = no overlay process at all)
                        let on = self.cfg().vr_overlay;
                        div()
                            .id("toggle-vr")
                            .px(px(12.))
                            .py(px(6.))
                            .border_1()
                            .rounded(px(6.))
                            .text_size(px(12.))
                            .cursor_pointer()
                            .when(on, |d| d.border_color(c(GREEN)).text_color(c(GREEN)))
                            .when(!on, |d| d.border_color(c(LINE2)).text_color(c(MUTED)).hover(|s| s.text_color(c(TEXT)).border_color(c(MUTED))))
                            .on_click(cx.listener(|this, _, _, cx| {
                                let on = {
                                    let mut c0 = this.cfg.lock().unwrap();
                                    c0.vr_overlay = !c0.vr_overlay;
                                    let _ = c0.save(&this.cfg_path);
                                    c0.vr_overlay
                                };
                                if let Some(s) = this.settings.as_mut() {
                                    s.vr_overlay = on;
                                }
                                if on {
                                    crate::vr::global().start();
                                } else {
                                    crate::vr::global().stop();
                                }
                                cx.notify();
                            }))
                            .child(if on { "◉  VR ligado" } else { "○  VR" })
                    }),
            )
    }

    fn sources(&self, t: &Telemetry) -> Div {
        let src = |n: &str, ok: bool, msg: &str| {
            div()
                .flex()
                .gap(px(4.))
                .child(div().text_color(c(MUTED)).font_weight(FontWeight::SEMIBOLD).child(n.to_string()))
                .child(div().text_color(c(if ok { GREEN } else { ORANGE })).child(if ok { "●" } else { "○" }))
                .child(msg.to_string())
        };
        div()
            .flex()
            .gap(px(14.))
            .text_size(px(11.))
            .text_color(c(DIM))
            .child(src(if cfg!(windows) { "Windows" } else { "Linux" }, true, "carga · clocks · RAM · discos · rede"))
            .child(src("NVML", t.gpu.error.is_none(), if t.gpu.error.is_some() { "sem GPU NVIDIA" } else { "GPU" }))
            .child(src(
                "Sensores",
                t.lhm_state == "running",
                match t.lhm_state.as_str() {
                    "running" => "CPU · placa-mãe · ventoinhas · RAM · GPU",
                    "limited" => "limitado (sem administrador)",
                    "starting" | "installing" => "iniciando…",
                    _ => "desligado",
                },
            ))
            .when(t.hwinfo.is_none(), |d| d.child(src("HWiNFO", true, if t.source == "hwinfo" { "em uso" } else { "disponível" })))
    }

    fn alert_bar(&self, t: &Telemetry) -> Option<Div> {
        if t.alerts.is_empty() {
            return None;
        }
        Some(
            div()
                .flex()
                .flex_wrap()
                .gap(px(14.))
                .px(px(12.))
                .py(px(9.))
                .rounded(px(6.))
                .border_1()
                .border_color(ca(RED, 0.4))
                .bg(ca(RED, 0.08))
                .text_size(px(12.5))
                .children(t.alerts.iter().map(|a| {
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(div().size(px(7.)).rounded(px(4.)).bg(c(RED)))
                        .child(div().font_weight(FontWeight::SEMIBOLD).child(a.title.clone()))
                        .child(format!("{:.0} {}", a.value, a.unit))
                        .child(div().text_color(c(MUTED)).child(format!("(limite {:.0})", a.limit)))
                })),
        )
    }

    // ---------- vitals ----------
    fn vital(&self, key: &str, value: String, unit: &str, sub: String, accent: u32, data: Vec<Option<f64>>, max: Option<f64>, alert: bool) -> Div {
        div()
            .relative()
            .flex_1()
            .min_w_0()
            .flex()
            .bg(c(PANEL))
            .border_1()
            .border_color(if alert { ca(RED, 0.6) } else { c(LINE) })
            .rounded(px(6.))
            .px(px(14.))
            .py(px(10.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(label(key))
                    .child(
                        div()
                            .flex()
                            .items_end()
                            .gap(px(3.))
                            .child(num(value, 34.))
                            .child(div().pb(px(5.)).text_size(px(15.)).text_color(c(MUTED)).child(unit.to_string())),
                    )
                    .child(small(sub, MUTED)),
            )
            .child(div().pt(px(8.)).child(draw::bars(data, accent, max)))
            .child(div().absolute().top(px(-1.)).left(px(-1.)).right(px(-1.)).h(px(2.)).rounded_t(px(6.)).bg(c(if alert { RED } else { accent })))
    }

    fn vitals(&self, t: &Telemetry) -> Div {
        let keys: Vec<&str> = t.alerts.iter().map(|a| a.key.as_str()).collect();
        let v = |q: &VecDeque<Option<f64>>| q.iter().cloned().collect::<Vec<_>>();
        let (rx, rxu) = rate(t.net.rx_bps);
        let (tx, txu) = rate(t.net.tx_bps);
        let cpu_sub = format!(
            "{:.2} GHz · {} núcleos{}",
            t.cpu.mhz_avg as f64 / 1000.0,
            if t.cpu.phys.is_empty() { t.cpu.cores.len() } else { t.cpu.phys.len() },
            t.cpu.package_temp.map_or(String::new(), |v| format!(" · {v:.0} °C"))
        );
        let gpu_sub = format!(
            "{} °C · {} W · {} MHz",
            t.gpu.temp.map_or("—".into(), |v| v.to_string()),
            f0(t.gpu.power_w),
            t.gpu.clock.map_or("—".into(), |v| v.to_string())
        );
        div()
            .flex()
            .gap(px(12.))
            .child(self.vital("CPU", f0(Some(t.cpu.load as f64)), "%", cpu_sub, INDIGO, v(&self.hist.cpu), Some(100.0), keys.contains(&"cpu_temp")))
            .child(self.vital("GPU", f0(t.gpu.load.map(|v| v as f64)), "%", gpu_sub, GREEN, v(&self.hist.gpu), Some(100.0), keys.contains(&"gpu_temp")))
            .child(self.vital("RAM", f0(t.mem_pct()), "%", format!("{} de {} GB", gb(t.mem.used), gb(t.mem.total)), AMBER, v(&self.hist.mem), Some(100.0), keys.contains(&"ram")))
            .child(self.vital("REDE ↓", rx, rxu, format!("↑ {tx} {txu}"), CYAN, v(&self.hist.net), None, false))
    }

    // ---------- this PC ----------
    fn pc_panel(&self) -> Div {
        let Some(pc) = &self.pc else {
            return panel(AMBER, "ESTE PC", "identificando…", false).child(small("lendo informações do sistema…", DIM));
        };
        if let Some(e) = &pc.error {
            return panel(AMBER, "ESTE PC", "—", false).child(small(e.clone(), DIM));
        }
        let generic = matches!(pc.maker.trim().to_lowercase().as_str(), "" | "intel" | "to be filled by o.e.m." | "system manufacturer" | "default string")
            || pc.model == pc.board;
        let name = if generic { "PC montado".to_string() } else { format!("{} {}", pc.maker, pc.model) };
        let jedec = |m: &markii_core::pcinfo::RamModule| -> String {
            let p = m.part.trim_end_matches('#');
            if p.len() == 7 && p.ends_with("AA") {
                return "HP".into();
            }
            match m.maker.to_uppercase().as_str() {
                "80CE" | "CE00" => "Samsung",
                "802C" | "2C00" => "Micron",
                "80AD" | "AD00" => "SK hynix",
                "859B" => "Crucial",
                "04CD" => "G.Skill",
                "029E" => "Corsair",
                "0198" | "9801" => "Kingston",
                _ => "",
            }
            .into()
        };
        let chip = |s: String| div().px(px(6.)).py(px(1.)).rounded(px(4.)).border_1().border_color(c(LINE2)).bg(c(PANEL2)).text_size(px(11.5)).child(s);
        let line = |k: &str, v: AnyElement| div().flex().gap(px(12.)).child(div().w(px(84.)).pt(px(2.)).child(label(k))).child(div().flex_1().min_w_0().text_size(px(12.5)).child(v));
        let ram_tot: f64 = pc.ram.iter().map(|m| m.size_gb).sum();
        let mut tips = Vec::new();
        if pc.ram.len() == 1 {
            tips.push("RAM em single channel: um segundo pente igual ativa o dual channel e melhora o desempenho, principalmente em jogos.".to_string());
        }
        if pc.disks.iter().any(|d| d.bus == "USB") {
            tips.push("Há um disco conectado por USB; ele pode ser mais lento que os internos.".into());
        }
        if pc.cpu_max_mhz > 0 && pc.cpu_max_mhz < 3500 {
            tips.push(format!("{:.1} GHz é o clock base; o processador acelera sozinho (turbo) quando precisa.", pc.cpu_max_mhz as f64 / 1000.0));
        }
        let sub = |a: String, b: String| div().child(a).child(div().text_size(px(11.)).text_color(c(MUTED)).child(b)).into_any_element();
        panel(AMBER, "ESTE PC", name.clone(), false).child(
            div()
                .flex()
                .flex_col()
                .gap(px(7.))
                .child(line("Sistema", sub(name, format!("placa {} {}", pc.board_maker, pc.board))))
                .child(line(if cfg!(windows) { "Windows" } else { "Sistema" }, sub(pc.os.replace("Microsoft ", ""), pc.os_build.clone())))
                .child(line("BIOS", div().child(pc.bios.clone()).into_any_element()))
                .child(line(
                    "CPU",
                    sub(
                        clean_cpu(&pc.cpu),
                        format!("{} núcleos · {} threads · {} MB L3 · soquete {}", pc.cpu_cores, pc.cpu_threads, pc.cpu_l3_kb / 1024, pc.cpu_socket),
                    ),
                ))
                .child(line("GPU", sub(pc.gpus.join(", "), format!("driver {}", pc.gpu_driver))))
                .child(line(
                    "RAM",
                    div()
                        .child(format!("{ram_tot:.0} GB · {} pente{}", pc.ram.len(), if pc.ram.len() > 1 { "s" } else { "" }))
                        .child(div().flex().flex_wrap().gap(px(4.)).mt(px(3.)).children(pc.ram.iter().map(|m| chip(format!("{} · {:.0} GB {} MT/s {}", m.slot, m.size_gb, m.speed, jedec(m))))))
                        .into_any_element(),
                ))
                .child(line(
                    "Discos",
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(px(4.))
                        .children(pc.disks.iter().map(|d| {
                            let size = if d.size_gb >= 1000.0 { format!("{:.1} TB", d.size_gb / 1024.0) } else { format!("{:.0} GB", d.size_gb) };
                            chip(format!("{} · {} · {}", d.model, size, if d.media == "Unspecified" { &d.bus } else { &d.media }))
                        }))
                        .into_any_element(),
                ))
                .when(!tips.is_empty(), |d| {
                    d.child(line("Dicas", div().flex().flex_col().gap(px(3.)).children(tips.into_iter().map(|t| div().child(format!("• {t}")))).into_any_element()))
                }),
        )
    }

    // ---------- cooling ----------
    fn cool_panel(&self, t: &Telemetry) -> Div {
        let k = &t.cooling;
        let (title, col) = match k.kind.as_str() {
            "water" => ("Water cooler", CYAN),
            "air" => ("Air cooler", TEXT),
            _ => ("Não identificado", MUTED),
        };
        let conf = match k.confidence.as_str() {
            "manual" => "definido manualmente",
            "high" => "certeza alta",
            "medium" => "provável",
            _ => "palpite",
        };
        let stat = |v: String, l: &str, vc: u32| {
            div()
                .flex_1()
                .min_w(px(110.))
                .bg(c(PANEL2))
                .border_1()
                .border_color(c(LINE))
                .rounded(px(4.))
                .px(px(8.))
                .py(px(6.))
                .child(num(v, 18.).text_color(c(vc)))
                .child(div().text_size(px(11.)).text_color(c(MUTED)).child(l.to_string()))
        };
        let mut stats = vec![stat(t.cpu.package_temp.map_or("—".into(), |v| format!("{v:.0} °C")), "CPU agora", temp_col(t.cpu.package_temp))];
        if k.kind == "water" {
            stats.push(stat(f0(k.pump_rpm), "bomba (rpm)", TEXT));
        }
        stats.push(stat(f0(k.cpu_fan_rpm), if k.kind == "water" { "fans do radiador (rpm)" } else { "ventoinha da CPU (rpm)" }, TEXT));
        if k.kind != "water" {
            stats.push(stat(t.cpu.power.map_or("—".into(), |v| format!("{v:.0} W")), "calor da CPU", TEXT));
        }
        let tip = match k.kind.as_str() {
            "water" => "Water cooler: a bomba precisa estar sempre girando. Se a rotação cair a zero a CPU esquenta em segundos. Limpe o radiador a cada 6 meses.",
            "air" => "Air cooler: limpe a poeira das aletas a cada 3–6 meses. CPU acima de 85 °C em carga costuma indicar pasta térmica velha ou ventoinha fraca.",
            _ => "Ative os sensores completos (aviso no topo) para o app ler as ventoinhas e a bomba, ou escolha o tipo em Configurações.",
        };
        panel(CYAN, "REFRIGERAÇÃO DA CPU", conf, false).child(
            div()
                .flex()
                .gap(px(16.))
                .items_center()
                .child(draw::cooler(k.kind.clone(), self.fan_angle, self.pump_angle, 0.0))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(num(title, 22.).text_color(c(col)))
                        .child(div().mt(px(3.)).mb(px(10.)).text_size(px(12.)).text_color(c(MUTED)).child(k.reason.clone()))
                        .child(div().flex().flex_wrap().gap(px(8.)).children(stats))
                        .child(div().mt(px(10.)).pl(px(8.)).border_l_2().border_color(c(CYAN)).text_size(px(12.)).text_color(c(MUTED)).child(tip)),
                ),
        )
    }

    // ---------- thermals / storage / procs / fans ----------
    fn therm_panel(&self, t: &Telemetry) -> Div {
        let mx = |q: &VecDeque<Option<f64>>| q.iter().flatten().cloned().fold(None, |a: Option<f64>, b| Some(a.map_or(b, |a| a.max(b))));
        let mins = (HIST as u64 * self.cfg().interval_ms) / 60000;
        let n = |k: &str, v: String, col: u32, sw: Option<u32>| {
            div()
                .flex_1()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .text_size(px(11.5))
                        .text_color(c(MUTED))
                        .when_some(sw, |d, s| d.child(div().w(px(10.)).h(px(3.)).rounded(px(2.)).bg(c(s))))
                        .child(k.to_string()),
                )
                .child(num(v, if sw.is_some() { 28. } else { 20. }).text_color(c(col)))
        };
        let alert = t.alerts.iter().any(|a| a.key == "cpu_temp" || a.key == "gpu_temp");
        panel(RED, "TEMPERATURAS", format!("máx. CPU {}° · GPU {}° · últimos {mins} min", f0(mx(&self.hist.cput)), f0(mx(&self.hist.gput))), alert)
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .mb(px(8.))
                    .child(n("CPU pacote", t.cpu.package_temp.map_or("—".into(), |v| format!("{v:.1}°")), temp_col(t.cpu.package_temp), Some(INDIGO)))
                    .child(n("GPU core", t.gpu.temp.map_or("—".into(), |v| format!("{v}°")), temp_col(t.gpu.temp.map(|v| v as f64)), Some(GREEN)))
                    .child(n("CPU potência", t.cpu.power.map_or("—".into(), |v| format!("{v:.0} W")), MUTED, None))
                    .child(n("GPU potência", t.gpu.power_w.map_or("—".into(), |v| format!("{v:.0} W")), MUTED, None)),
            )
            .child(draw::lines(
                vec![(self.hist.cput.iter().cloned().collect(), INDIGO), (self.hist.gput.iter().cloned().collect(), GREEN)],
                HIST,
            ))
    }

    fn disk_panel(&self, t: &Telemetry) -> Div {
        let (u, tt) = t.disks.iter().fold((0, 0), |(a, b), d| (a + d.used, b + d.total));
        panel(AMBER, "ARMAZENAMENTO", format!("{} / {} GB", gb(u), gb(tt)), false).children(t.disks.iter().map(|d| {
            let p = pct(d.used, d.total).unwrap_or(0.0);
            let alert = t.alerts.iter().any(|a| a.key == format!("disk:{}", d.mount));
            let col = if alert || p > 90.0 { RED } else if p > 75.0 { ORANGE } else { AMBER };
            div()
                .py(px(7.))
                .border_b_1()
                .border_color(c(LINE))
                .child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .text_size(px(12.5))
                        .child(div().font_weight(FontWeight::BOLD).child(d.mount.clone()))
                        .child(div().flex_1().min_w_0().child(format!("{}  ", if d.name.is_empty() { &d.kind } else { &d.name })))
                        .child(div().font_family(NUM_FONT).text_color(c(MUTED)).child(format!("{} / {} GB · {:.0}%", gb(d.used), gb(d.total), p))),
                )
                .child(segbar(p, 20, col, 6.))
        }))
        .when(!t.hw.drives.is_empty(), |d| d.child(label("Saúde dos discos").mt(px(10.)).mb(px(2.))).children(self.drives_rows(t)))
    }

    fn procs_panel(&self, t: &Telemetry) -> Div {
        panel(PINK, "PROCESSOS", "top CPU", false).children(t.procs.iter().map(|p| row(p.name.clone(), format!("{:.1} % · {} MB", p.cpu, p.mem / 1048576))))
    }

    // ---------- settings ----------
    fn settings_sheet(&self, s: &Config, cx: &mut Context<Self>) -> Stateful<Div> {
        let toggle = |id: &'static str, label_: &str, on: bool, f: fn(&mut Config)| {
            div()
                .id(id)
                .flex()
                .items_center()
                .gap(px(8.))
                .py(px(4.))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(s) = this.settings.as_mut() {
                        f(s);
                    }
                    cx.notify();
                }))
                .child(
                    div()
                        .size(px(15.))
                        .rounded(px(3.))
                        .border_1()
                        .border_color(c(if on { AMBER } else { LINE2 }))
                        .bg(c(if on { AMBER } else { BG }))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(11.))
                        .text_color(c(0x1a1205))
                        .child(if on { "✓" } else { "" }),
                )
                .child(label_.to_string())
        };
        let stepper = |id: &'static str, label_: &str, v: String, unit: &str, f: fn(&mut Config, i32)| {
            let btn = |sfx: &'static str, txt: &'static str, d: i32| {
                div()
                    .id(SharedString::from(format!("{id}{sfx}")))
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
                        if let Some(s) = this.settings.as_mut() {
                            f(s, d);
                        }
                        cx.notify();
                    }))
                    .child(txt)
            };
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .text_size(px(12.5))
                .text_color(c(MUTED))
                .child(div().flex_1().child(label_.to_string()))
                .child(btn("-", "−", -1))
                .child(num(v, 13.).w(px(40.)).flex().justify_center().text_color(c(TEXT)))
                .child(btn("+", "+", 1))
                .child(div().w(px(26.)).text_color(c(DIM)).child(unit.to_string()))
        };
        let cool_opt = |id: &'static str, v: &'static str, l: &str| {
            let on = s.cooling == v;
            div()
                .id(id)
                .flex_1()
                .py(px(6.))
                .flex()
                .justify_center()
                .text_size(px(12.))
                .cursor_pointer()
                .when(on, |d| d.bg(c(AMBER)).text_color(c(0x1a1205)))
                .when(!on, |d| d.text_color(c(MUTED)).hover(|s| s.bg(c(PANEL2))))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(s) = this.settings.as_mut() {
                        s.cooling = v.into();
                    }
                    cx.notify();
                }))
                .child(l.to_string())
        };
        let h3 = |t: &str| label(t).mt(px(16.)).mb(px(8.));
        let cur_theme = theme().key;
        let theme_opt = |i: usize| {
            let th = &THEMES[i];
            let on = th.key == cur_theme;
            let key = th.key;
            // swatches use the theme's own colors, not the active theme
            div()
                .id(SharedString::from(format!("theme-{key}")))
                .flex_1()
                .min_w_0()
                .rounded(px(5.))
                .border_1()
                .border_color(if on { c(AMBER) } else { c(LINE2) })
                .p(px(3.))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| {
                    set_theme(key);
                    if let Some(s) = this.settings.as_mut() {
                        s.theme = key.into();
                    }
                    let mut c0 = this.cfg.lock().unwrap();
                    c0.theme = key.into();
                    let _ = c0.save(&this.cfg_path);
                    cx.notify();
                }))
                .child(
                    div()
                        .rounded(px(3.))
                        .bg(gpui::rgb(th.grey[0]))
                        .p(px(6.))
                        .child(
                            div()
                                .rounded(px(2.))
                                .bg(gpui::rgb(th.grey[1]))
                                .border_1()
                                .border_color(gpui::rgb(th.grey[3]))
                                .p(px(5.))
                                .child(div().h(px(4.)).w(px(34.)).rounded(px(1.)).bg(gpui::rgb(th.grey[7])))
                                .child(div().flex().gap(px(3.)).mt(px(5.)).children([0usize, 1, 2, 3].map(|k| div().size(px(9.)).rounded(px(5.)).bg(gpui::rgb(th.accent[k]))))),
                        ),
                )
                .child(
                    div()
                        .pt(px(4.))
                        .flex()
                        .justify_center()
                        .text_size(px(11.))
                        .text_color(c(if on { TEXT } else { MUTED }))
                        .flex_col()
                        .items_center()
                        .child(th.name)
                        .child(div().text_size(px(9.5)).text_color(c(DIM)).child(if i == 0 { "padrão" } else if th.light { "claro" } else { "escuro" })),
                )
        };
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
        div()
            .id("modal")
            .absolute()
            .inset_0()
            .bg(ca(0, 0.65))
            .flex()
            .items_center()
            .justify_center()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .id("sheet")
                    .w(px(520.))
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
                    .child(div().absolute().top(px(-1.)).left(px(-1.)).right(px(-1.)).h(px(2.)).bg(c(AMBER)))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .child(num("Configurações", 15.))
                            .child(
                                div()
                                    .id("close")
                                    .ml_auto()
                                    .px(px(8.))
                                    .cursor_pointer()
                                    .text_color(c(MUTED))
                                    .hover(|s| s.text_color(c(TEXT)))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.settings = None;
                                        cx.notify();
                                    }))
                                    .child("✕"),
                            ),
                    )
                    .child(h3("Alertas"))
                    .child(toggle("t-alerts", "Alertas ativos", s.alerts_enabled, |s| s.alerts_enabled = !s.alerts_enabled))
                    .child(toggle("t-notify", "Notificação do Windows", s.notify, |s| s.notify = !s.notify))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(8.))
                            .mt(px(6.))
                            .child(stepper("cpu", "CPU acima de", format!("{:.0}", s.cpu_temp), "°C", |s, d| s.cpu_temp = (s.cpu_temp + d as f64).clamp(40.0, 110.0)))
                            .child(stepper("gpu", "GPU acima de", format!("{:.0}", s.gpu_temp), "°C", |s, d| s.gpu_temp = (s.gpu_temp + d as f64).clamp(40.0, 110.0)))
                            .child(stepper("ram", "RAM acima de", format!("{:.0}", s.ram_pct), "%", |s, d| s.ram_pct = (s.ram_pct + d as f64).clamp(10.0, 100.0)))
                            .child(stepper("disk", "Disco acima de", format!("{:.0}", s.disk_pct), "%", |s, d| s.disk_pct = (s.disk_pct + d as f64).clamp(10.0, 100.0)))
                            .child(stepper("hold", "Por pelo menos", s.hold_s.to_string(), "s", |s, d| s.hold_s = (s.hold_s as i64 + d as i64).clamp(0, 600) as u64))
                            .child(stepper("cool", "Repetir a cada", s.cooldown_min.to_string(), "min", |s, d| s.cooldown_min = (s.cooldown_min as i64 + d as i64).clamp(1, 240) as u64)),
                    )
                    .child(h3("Tema"))
                    .child(div().flex().flex_col().gap(px(6.)).children((0..THEMES.len().div_ceil(4)).map(|r| {
                        div().flex().gap(px(6.)).children((r * 4..(r * 4 + 4).min(THEMES.len())).map(&theme_opt))
                    })))
                    .child(h3("Refrigeração"))
                    .child(
                        div()
                            .flex()
                            .border_1()
                            .border_color(c(LINE2))
                            .rounded(px(6.))
                            .overflow_hidden()
                            .child(cool_opt("c-auto", "auto", "Detectar"))
                            .child(cool_opt("c-air", "air", "Air cooler"))
                            .child(cool_opt("c-water", "water", "Water cooler")),
                    )
                    .child(h3("Sensores"))
                    .when(cfg!(windows), |d| {
                        d.child(toggle("t-admin", "Pedir administrador (UAC) ao abrir: CPU, ventoinhas e placa-mãe", s.sensors_admin, |s| s.sensors_admin = !s.sensors_admin))
                    })
                    .child(small(
                        if cfg!(windows) {
                            format!(
                                "Estado: {} · driver PawnIO {}",
                                self.t.as_ref().map_or("—".to_string(), |t| t.lhm_msg.clone()),
                                if markii_core::lhm::pawnio_installed() { "instalado" } else { "não instalado" }
                            )
                        } else {
                            format!("Estado: {} · sensores do kernel (hwmon/lm-sensors)", self.t.as_ref().map_or("—".to_string(), |t| t.lhm_msg.clone()))
                        },
                        MUTED,
                    ))
                    .child(
                        div().flex().gap(px(8.)).mt(px(8.)).child(btn("restart-sensors", "Reiniciar sensores", false).on_click(cx.listener(|this, _, _, cx| {
                            let (iv, admin) = {
                                let c0 = this.cfg.lock().unwrap();
                                (c0.interval_ms, this.settings.as_ref().map_or(c0.sensors_admin, |s| s.sensors_admin))
                            };
                            markii_core::lhm::global().start(iv, admin && markii_core::lhm::pawnio_installed());
                            this.save_msg = "reiniciando sensores…".into();
                            cx.notify();
                        }))),
                    )
                    .child(h3("Realidade virtual"))
                    .child(toggle("t-vr", "Painel do Mark II no SteamVR (pulso esquerdo)", s.vr_overlay, |s| s.vr_overlay = !s.vr_overlay))
                    .child(small(
                        "Aparece quando o SteamVR está aberto (Quest via Link, Air Link ou Virtual Desktop, Index, Vive...). Não abre o SteamVR sozinho e fecha junto com o Mark II.",
                        MUTED,
                    ))
                    .child(h3("Celular e óculos (app Android)"))
                    .child(toggle("t-remote", "Permitir o app Mark II na rede local (só leitura, com PIN)", s.remote_enabled, |s| s.remote_enabled = !s.remote_enabled))
                    .child({
                        let info = markii_core::remote::global().info();
                        // read once: a guard kept alive across the builder chain would deadlock
                        let fw_block = {
                            let g = self.fw.lock().unwrap();
                            (g.0 == Some(false)).then(|| g.1.clone())
                        };
                        if info.running {
                            div()
                                .mt(px(6.))
                                .p(px(10.))
                                .rounded(px(4.))
                                .bg(c(PANEL2))
                                .flex()
                                .items_center()
                                .gap(px(18.))
                                .child(div().child(label("Endereço")).child(num(info.address.clone(), 15.)))
                                .child(div().child(label("PIN")).child(num(info.pin.clone(), 22.).text_color(c(AMBER))))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .child(small(format!("{} aparelho(s) pareado(s)", info.paired), MUTED))
                                        .child(small(info.connected.clone().map_or("nenhum conectado agora".to_string(), |n| format!("conectado: {n}")), if info.connected.is_some() { GREEN } else { DIM }))
                                        .when_some(info.error.clone(), |d, e| d.child(small(e, RED))),
                                )
                                .when_some(fw_block, |d, msg| {
                                    d.child(
                                        div()
                                            .child(small(if msg.is_empty() { "O firewall do Windows está bloqueando o app.".to_string() } else { msg }, RED))
                                            .child(btn("fw-allow", "Liberar no firewall", true).on_click(cx.listener(|this, _, _, cx| {
                                                let fw = this.fw.clone();
                                                let port = this.cfg.lock().unwrap().remote_port;
                                                fw.lock().unwrap().1 = "confirme no aviso do Windows (UAC)…".into();
                                                std::thread::spawn(move || {
                                                    let r = markii_core::remote::allow_firewall(port);
                                                    *fw.lock().unwrap() = match r {
                                                        Ok(()) => (Some(true), String::new()),
                                                        Err(e) if e == "cancelado" => (Some(false), "Cancelado no UAC: o app não vai achar o PC até liberar.".into()),
                                                        Err(e) => (Some(false), format!("Não consegui liberar: {e}")),
                                                    };
                                                });
                                                cx.notify();
                                            }))),
                                    )
                                })
                                .when(info.paired > 0, |d| {
                                    d.child(btn("unpair", "Desconectar todos", false).on_click(cx.listener(|_, _, _, cx| {
                                        markii_core::remote::global().unpair_all();
                                        cx.notify();
                                    })))
                                })
                        } else {
                            div().child(small(
                                if s.remote_enabled { "Clique em Salvar para ligar. O Windows pode pedir para liberar o Mark II no firewall (rede privada)." } else { "Desligado: nenhuma porta aberta." },
                                MUTED,
                            ))
                        }
                    })
                    .child(small(
                        "No óculos ou celular, abra o app Mark II, digite o endereço e o PIN. Só funciona com esta janela aberta e na mesma rede; o app só lê os números.",
                        DIM,
                    ))
                    .child(h3("Janela"))
                    .child(small("Fechar a janela encerra o Mark II e tudo o que ele abriu (sensores e OpenRGB); nada fica rodando em segundo plano.", MUTED))
                    .child(h3("Log"))
                    .child(small(
                        "O Mark II não grava nada sozinho. \"Criar log\" salva um arquivo com o que aconteceu nesta sessão (sensores, RGB, VR, rede, firewall) para enviar a quem for ajudar. Não inclui PIN nem chaves de pareamento.",
                        MUTED,
                    ))
                    .child(div().mt(px(6.)).flex().child(btn("make-log", "Criar log", false).on_click(cx.listener(|this, _, _, cx| {
                        let header = this.log_header();
                        this.save_msg = match markii_core::log::save_report(&header) {
                            Ok(p) => {
                                markii_core::log::reveal(&p);
                                format!("log criado: {}", p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()).into()
                            }
                            Err(e) => format!("não consegui criar o log: {e}").into(),
                        };
                        cx.notify();
                    }))))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .mt(px(18.))
                            .child(btn("test", "Testar notificação", false).on_click(cx.listener(|this, _, _, cx| {
                                crate::notify("Mark II · teste", "As notificações estão funcionando.");
                                this.save_msg = "notificação enviada".into();
                                cx.notify();
                            })))
                            .child(div().ml_auto().text_size(px(12.)).text_color(c(GREEN)).child(self.save_msg.clone()))
                            .child(btn("save", "Salvar", true).on_click(cx.listener(|this, _, _, cx| {
                                if let Some(s) = this.settings.take() {
                                    let mut c0 = this.cfg.lock().unwrap();
                                    let (interval, view, tokens) = (c0.interval_ms, c0.cpu_view.clone(), c0.remote_tokens.clone());
                                    *c0 = Config { interval_ms: interval, cpu_view: view, remote_tokens: tokens, ..s };
                                    let _ = c0.save(&this.cfg_path);
                                    if c0.vr_overlay {
                                        crate::vr::global().start();
                                    } else {
                                        crate::vr::global().stop();
                                    }
                                    let remote = c0.remote_enabled;
                                    drop(c0);
                                    if remote {
                                        let was_running = markii_core::remote::global().running();
                                        crate::start_remote(&this.cfg, &this.cfg_path);
                                        if !was_running {
                                            // keep the sheet open so the address and PIN can be read
                                            this.settings = Some(this.cfg.lock().unwrap().clone());
                                            this.save_msg = "salvo · use o endereço e o PIN acima".into();
                                        }
                                        this.check_fw();
                                    } else {
                                        markii_core::remote::global().stop();
                                    }
                                }
                                cx.notify();
                            }))),
                    ),
            )
    }
}

impl Render for Dashboard {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let root = div()
            .relative()
            .size_full()
            .bg(c(BG))
            .text_color(c(TEXT))
            .font_family(UI_FONT)
            .text_size(px(13.));
        let Some(t) = self.t.clone() else {
            return root
                .flex()
                .items_center()
                .justify_center()
                .child(num("MARK II", 22.).text_color(c(AMBER)))
                .child(small("  lendo sensores…", MUTED));
        };
        let duo = |a: Div, b: Div| div().flex().gap(px(12.)).child(a.flex_1()).child(b.flex_1());
        let settings = self.settings.clone();
        root.child(
            div()
                .id("scroll")
                .size_full()
                .overflow_y_scroll()
                .px(px(24.))
                .pt(px(18.))
                .pb(px(24.))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(12.))
                        .child(self.header(&t, cx))
                        .when_some(self.alert_bar(&t), |d, a| d.child(a))
                        .when_some(self.sensors_bar(&t, cx), |d, a| d.child(a))
                        .child(self.sources(&t))
                        .child(self.vitals(&t))
                        .when_some(self.devices_panel(), |d, p| d.child(p))
                        .child(duo(self.cpu_panel(&t, cx), self.gpu_panel(&t)))
                        .child(duo(self.ram_panel(&t), self.board_panel(&t)))
                        .child(duo(self.fans_panel(&t), self.cool_panel(&t)))
                        .child(duo(self.therm_panel(&t), self.disk_panel(&t)))
                        .child(duo(self.pc_panel(), self.procs_panel(&t))),
                ),
        )
        .when_some(settings, |d, s| d.child(self.settings_sheet(&s, cx)))
        .when(self.rgb.is_some(), |d| d.child(self.rgb_sheet(cx)))
    }
}
