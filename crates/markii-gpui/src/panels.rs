//! Hardware panels: CPU (cores/threads), compact GPU, memory, motherboard, fans,
//! and the sensor-status banner. Everything is drawn from what this PC reports;
//! nothing is specific to one model of CPU, GPU or board.
use crate::draw;
use crate::theme::*;
use crate::view::{gb, label, num, panel, panel_logo, row, segbar, small, Dashboard};
use gpui::{div, img, prelude::*, px, AnyElement, Context, Div, FontWeight, Image, ImageFormat, SharedString};
use markii_core::brands::{self, Brand};
use markii_core::{lhm, ram, thread_map, CoreT, Telemetry};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

/// GPUI 0.2's SVG rasteriser hands the pixels over as BGRA, so red and blue
/// come out swapped. Pre-swap every #RGB / #RRGGBB in the SVG to compensate.
pub(crate) fn svg_for_gpui(svg: &str) -> Vec<u8> {
    let b = svg.as_bytes();
    let mut out = String::with_capacity(svg.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'#' {
            let n = b[i + 1..].iter().take_while(|c| c.is_ascii_hexdigit()).count();
            if n == 6 || n == 3 {
                let h = &svg[i + 1..i + 1 + n];
                let full: String = if n == 3 { h.chars().flat_map(|c| [c, c]).collect() } else { h.to_string() };
                out.push('#');
                out.push_str(&full[4..6]);
                out.push_str(&full[2..4]);
                out.push_str(&full[0..2]);
                i += 1 + n;
                continue;
            }
        }
        let ch = svg[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out.into_bytes()
}

/// SVG logos are parsed once and reused, so GPUI keeps its rasterised copy.
fn logo_image(b: &Brand) -> Option<Arc<Image>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Arc<Image>>>> = OnceLock::new();
    let svg = b.svg.as_ref()?;
    // logos drawn in the text color (black brands) follow the theme
    let fill = format!("#{:06X}", brand_col(b));
    let k = format!("{}{fill}", b.key);
    let mut m = CACHE.get_or_init(Default::default).lock().ok()?;
    Some(
        m.entry(k)
            .or_insert_with(|| Arc::new(Image::from_bytes(ImageFormat::Svg, svg_for_gpui(&svg.replacen(&format!("fill=\"{}\"", b.color), &format!("fill=\"{fill}\""), 1)))))
            .clone(),
    )
}

/// Brand colors are kept as they are; only the neutral (text) color follows the theme.
fn brand_col(b: &Brand) -> u32 {
    let v = hex(&b.color);
    if v == TEXT { remap(TEXT) } else { v }
}

fn hex(s: &str) -> u32 {
    u32::from_str_radix(s.trim_start_matches('#'), 16).unwrap_or(TEXT)
}

/// Brand logo (plus "RADEON"/"ARC" when the product line has no logo of its
/// own), or a text badge when no free logo exists. `None` for unknown brands.
pub(crate) fn brand_mark(b: &Brand, size: f32) -> Option<AnyElement> {
    let col = gpui::rgb(brand_col(b));
    if let Some(im) = logo_image(b) {
        return Some(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(4.))
                .child(img(im).w(px(size)).h(px(size)).flex_none())
                .when(!b.tag.is_empty(), |d| {
                    d.child(div().font_family(NUM_FONT).font_weight(FontWeight::BOLD).text_size(px(size * 0.5)).text_color(col).child(b.tag.clone()))
                })
                .into_any_element(),
        );
    }
    (!b.name.is_empty()).then(|| {
        div()
            .flex_none()
            .px(px(5.))
            .py(px(1.))
            .rounded(px(3.))
            .border_1()
            .border_color(gpui::Hsla::from(col).opacity(0.5))
            .font_family(NUM_FONT)
            .font_weight(FontWeight::BOLD)
            .text_size(px((size * 0.45).max(9.5)))
            .text_color(col)
            .child(b.name.to_uppercase())
            .into_any_element()
    })
}

fn opt(v: Option<f64>, fmt: impl Fn(f64) -> String) -> String {
    v.filter(|v| v.is_finite()).map_or("—".into(), fmt)
}

/// A small metric cell: label, value, optional note under it.
fn cell(k: &str, v: String, col: Option<u32>, note: Option<String>) -> Div {
    let missing = v == "—";
    div()
        .flex_1()
        .min_w(px(0.))
        .bg(c(PANEL2))
        .border_1()
        .border_color(c(LINE))
        .rounded(px(4.))
        .px(px(8.))
        .py(px(6.))
        .child(label(k))
        .child(num(v, 16.).text_color(c(if missing { DIM } else { col.unwrap_or(TEXT) })))
        .when_some(note, |d, n| d.child(div().text_size(px(10.5)).text_color(c(if missing { DIM } else { MUTED })).child(n)))
}

/// Lays cells out in rows of `n`.
fn grid(cells: Vec<Div>, n: usize) -> Div {
    let mut rows = Vec::new();
    let mut it = cells.into_iter().peekable();
    while it.peek().is_some() {
        let mut r: Vec<Div> = it.by_ref().take(n).collect();
        while r.len() < n {
            r.push(div().flex_1());
        }
        rows.push(div().flex().gap(px(6.)).children(r));
    }
    div().flex().flex_col().gap(px(6.)).children(rows)
}

fn seg<T: 'static>(cx: &mut Context<Dashboard>, id: &str, opts: &[(&'static str, &'static str)], cur: &str, f: fn(&mut Dashboard, &'static str)) -> Div
where
    Dashboard: 'static,
{
    let _ = std::marker::PhantomData::<T>;
    div().flex().border_1().border_color(c(LINE2)).rounded(px(5.)).overflow_hidden().children(opts.iter().map(|(v, l)| {
        let on = cur == *v;
        let v = *v;
        div()
            .id(SharedString::from(format!("{id}-{v}")))
            .px(px(9.))
            .py(px(3.))
            .text_size(px(11.))
            .font_family(NUM_FONT)
            .font_weight(FontWeight::SEMIBOLD)
            .cursor_pointer()
            .when(on, |d| d.bg(c(INDIGO)).text_color(c(if theme().light { PANEL } else { 0x0d0f1a })))
            .when(!on, |d| d.text_color(c(MUTED)).hover(|s| s.bg(c(PANEL2)).text_color(c(TEXT))))
            .on_click(cx.listener(move |this, _, _, cx| {
                f(this, v);
                cx.notify();
            }))
            .child(*l)
    }))
}

fn needs_sensors(t: &Telemetry) -> Option<String> {
    (t.source.is_empty() || t.lhm_state == "limited").then(|| "precisa dos sensores completos".to_string())
}

impl Dashboard {
    // ---------------------------------------------------------------- sensors banner
    /// "Nova versão" bar (see update.rs).
    pub(crate) fn update_bar(&self, cx: &mut Context<Self>) -> Option<Div> {
        use crate::update::{self, State};
        let st = update::state();
        let (msg, action): (String, Option<(&'static str, Option<String>)>) = match &st {
            State::Idle => return None,
            State::Restarting => {
                // the update script is waiting for Mark II to close
                cx.defer(|cx| {
                    markii_core::log::info("Mark II encerrado para atualizar");
                    crate::stop_helpers();
                    cx.quit();
                });
                ("Atualizando: o Mark II vai fechar e abrir de novo na versão nova…".into(), None)
            }
            State::Available(v, _) if update::can_install() => (format!("Nova versão do Mark II: v{v}."), Some(("Atualizar", None))),
            State::Available(v, page) => (format!("Nova versão do Mark II: v{v}."), Some(("Ver no GitHub", Some(page.clone())))),
            State::Working(m) => (format!("Atualizando: {m}"), None),
            State::Failed(e) => (format!("A atualização não deu certo: {e}"), Some(("Tentar de novo", None))),
        };
        Some(
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .px(px(12.))
                .py(px(8.))
                .rounded(px(6.))
                .border_1()
                .border_color(ca(GREEN, 0.45))
                .bg(ca(GREEN, 0.08))
                .text_size(px(12.5))
                .child(div().text_color(c(GREEN)).child("⬆"))
                .child(div().flex_1().min_w_0().child(msg))
                .when_some(action, |d, (txt, page)| {
                    d.child(
                        div()
                            .id("update-action")
                            .px(px(12.))
                            .py(px(5.))
                            .rounded(px(4.))
                            .bg(c(GREEN))
                            .text_color(c(0x08140a))
                            .font_family(NUM_FONT)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(px(12.))
                            .cursor_pointer()
                            .on_click(cx.listener(move |_, _, _, cx| {
                                match &page {
                                    Some(url) => update::open_page(url),
                                    None => update::install(),
                                }
                                cx.notify();
                            }))
                            .child(txt),
                    )
                }),
        )
    }

    pub(crate) fn sensors_bar(&self, t: &Telemetry, cx: &mut Context<Self>) -> Option<Div> {
        let interval = self.cfg.lock().unwrap().interval_ms;
        let (msg, action): (String, Option<(&'static str, bool)>) = match t.lhm_state.as_str() {
            "running" | "off" if t.pawnio => return None,
            "starting" | "installing" => (t.lhm_msg.clone(), None),
            _ if !t.pawnio => (
                "Sensores limitados: falta o driver PawnIO para ler CPU (potência, Vcore), ventoinhas e placa-mãe.".into(),
                Some(("Instalar PawnIO", true)),
            ),
            "limited" => (
                "Rodando sem administrador: GPU e discos funcionam, mas CPU, ventoinhas e placa-mãe precisam da permissão (UAC).".into(),
                Some(("Tentar com administrador", false)),
            ),
            _ => (format!("Sensores: {}", t.lhm_msg), Some(("Tentar de novo", false))),
        };
        Some(
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .px(px(12.))
                .py(px(8.))
                .rounded(px(6.))
                .border_1()
                .border_color(ca(ORANGE, 0.45))
                .bg(ca(ORANGE, 0.08))
                .text_size(px(12.5))
                .child(div().text_color(c(ORANGE)).child("⚠"))
                .child(div().flex_1().min_w_0().child(msg))
                .when_some(action, |d, (txt, install)| {
                    d.child(
                        div()
                            .id("sensors-action")
                            .px(px(12.))
                            .py(px(5.))
                            .rounded(px(4.))
                            .bg(c(ORANGE))
                            .text_color(c(0x1a1205))
                            .font_family(NUM_FONT)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(px(12.))
                            .cursor_pointer()
                            .on_click(cx.listener(move |_, _, _, cx| {
                                if install {
                                    lhm::global().install_then_start(interval);
                                } else {
                                    lhm::global().start(interval, true);
                                }
                                cx.notify();
                            }))
                            .child(txt),
                    )
                }),
        )
    }

    // ---------------------------------------------------------------- CPU
    pub(crate) fn cpu_panel(&self, t: &Telemetry, cx: &mut Context<Self>) -> Div {
        let cpu = &t.cpu;
        let view = self.cfg.lock().unwrap().cpu_view.clone();
        let phys_ok = !cpu.phys.is_empty();
        let both = view == "both";
        let threads_view = view == "threads" || (!phys_ok && !both);
        let hybrid = cpu.phys.iter().any(|c| c.kind == "E");
        // physical cores and which threads each one runs
        let ncores = if phys_ok { cpu.phys.len() } else { self.pc.as_ref().map_or(0, |p| p.cpu_cores as usize) };
        let tmap = thread_map(ncores, cpu.cores.len());
        let synth: Vec<CoreT>;
        let list: &Vec<CoreT> = if threads_view {
            &cpu.cores
        } else if phys_ok {
            &cpu.phys
        } else {
            // no sensors: build each core from its threads
            synth = tmap
                .iter()
                .enumerate()
                .map(|(i, ts)| {
                    let th: Vec<&CoreT> = ts.iter().filter_map(|t| cpu.cores.get(*t)).collect();
                    CoreT {
                        idx: i,
                        load: th.iter().map(|t| t.load).sum::<f32>() / th.len().max(1) as f32,
                        mhz: th.iter().map(|t| t.mhz).max().unwrap_or(0),
                        ..Default::default()
                    }
                })
                .collect();
            &synth
        };
        let thread_bars = |core: usize| {
            let ts = tmap.get(core).cloned().unwrap_or_default();
            div().flex().flex_col().gap(px(2.)).mt(px(3.)).children(ts.into_iter().filter_map(|i| cpu.cores.get(i)).map(|th| {
                let col = load_col(Some(th.load as f64));
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .font_family(NUM_FONT)
                    .text_size(px(9.))
                    .text_color(c(MUTED))
                    .child(div().w(px(20.)).child(format!("T{}", th.idx)))
                    .child(div().flex_1().child(segbar(th.load as f64, 8, col, 3.).mt(px(0.))))
                    .child(div().w(px(26.)).flex().justify_end().child(format!("{:.0}%", th.load)))
            }))
        };
        let tile = |c0: &CoreT| {
            let col = if c0.temp.is_some() && !threads_view { temp_col(c0.temp) } else { load_col(Some(c0.load as f64)) };
            let big = match (threads_view, c0.temp) {
                (false, Some(v)) => format!("{v:.0}°"),
                _ => format!("{:.0}%", c0.load),
            };
            let tag = if threads_view { format!("T{}", c0.idx) } else if hybrid { format!("{}{}", c0.kind, c0.idx) } else { format!("C{}", c0.idx) };
            let sub = match c0.volt {
                Some(v) if !threads_view => format!("{:.2} GHz · {v:.2} V", c0.mhz as f64 / 1000.0),
                _ => format!("{:.2} GHz", c0.mhz as f64 / 1000.0),
            };
            div()
                .flex_1()
                .min_w_0()
                .rounded(px(3.))
                .border_1()
                .border_color(ca(col, 0.45))
                .bg(mix(col, 0x161b22, 0.14))
                .px(px(6.))
                .pt(px(3.))
                .pb(px(5.))
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .font_family(NUM_FONT)
                        .text_size(px(9.5))
                        .text_color(c(MUTED))
                        .child(tag)
                        .child(format!("{:.0}%", c0.load)),
                )
                .child(num(big, 18.).text_color(c(col)))
                .child(div().font_family(NUM_FONT).text_size(px(9.5)).text_color(c(MUTED)).child(sub))
                .when(!both, |d| d.child(segbar(c0.load as f64, 10, col, 3.)))
                .when(both, |d| d.child(thread_bars(c0.idx)))
        };
        let per_row = match list.len() {
            0..=8 => 2,
            9..=16 => 4,
            17..=24 => 4,
            _ => 6,
        };
        let mut rows = Vec::new();
        for chunk in list.chunks(per_row) {
            let mut r: Vec<Div> = chunk.iter().map(&tile).collect();
            while r.len() < per_row {
                r.push(div().flex_1());
            }
            rows.push(div().flex().gap(px(5.)).children(r));
        }
        let die = div()
            .flex_1()
            .min_w_0()
            .border_1()
            .border_color(c(0x3a4452))
            .rounded(px(4.))
            .bg(c(0x152a1e))
            .p(px(6.))
            // the die stretches to the stats column's height: the inner box fills it and
            // centers the cores, instead of leaving an empty block under 2-4 cores
            .flex()
            .flex_col()
            .child(div().h(px(4.)).mx(px(2.)).mb(px(6.)).bg(ca(0xc9a227, 0.5)))
            .child(
                div()
                    .flex_1()
                    .border_1()
                    .border_color(c(0x5a6472))
                    .rounded(px(3.))
                    .bg(c(0x1f252d))
                    .p(px(7.))
                    .flex()
                    .flex_col()
                    .justify_center()
                    .gap(px(5.))
                    .child(div().flex().justify_center().child(label(&format!(
                        "{} núcleos · {} threads",
                        if ncores > 0 { ncores } else { cpu.cores.len().max(1) },
                        cpu.threads
                    ))))
                    .children(rows),
            )
            .child(div().h(px(4.)).mx(px(2.)).mt(px(6.)).bg(ca(0xc9a227, 0.5)));
        let need = needs_sensors(t);
        let stat = |l: &str, v: String, col: Option<u32>, sub: Option<String>| {
            let missing = v == "—";
            div()
                .child(label(l))
                .child(num(v, 17.).text_color(c(if missing { DIM } else { col.unwrap_or(TEXT) })))
                .when_some(sub, |d, s| d.child(div().text_size(px(10.)).text_color(c(MUTED)).child(s)))
        };
        let ghz = |m: u64| if m == 0 { "—".to_string() } else { format!("{:.2}", m as f64 / 1000.0) };
        let side = div()
            .w(px(128.))
            .flex()
            .flex_col()
            .gap(px(7.))
            .child(stat("Carga", format!("{:.0} %", cpu.load), Some(load_col(Some(cpu.load as f64))), None))
            .child(stat("Pacote", opt(cpu.package_temp, |v| format!("{v:.0} °C")), Some(temp_col(cpu.package_temp)), if cpu.package_temp.is_none() { need.clone() } else { None }))
            .child(stat("Core max", opt(cpu.core_max, |v| format!("{v:.0} °C")), Some(temp_col(cpu.core_max)), None))
            .child(stat(
                "Potência",
                opt(cpu.power, |v| format!("{v:.1} W")),
                None,
                cpu.cores_power.map(|v| format!("núcleos {v:.1} W")).or(if cpu.power.is_none() { need.clone() } else { None }),
            ))
            .child(stat("Vcore", opt(cpu.vcore, |v| format!("{v:.3} V")), Some(YELLOW), cpu.vid.filter(|_| cpu.vcore.is_some()).map(|v| format!("VID pedido {v:.3} V"))))
            .child(stat(
                "Clock agora",
                format!("{}–{} GHz", ghz(cpu.mhz_min), ghz(cpu.mhz_max)),
                None,
                Some(format!("sessão {}–{} GHz", ghz(cpu.session_min), ghz(cpu.session_max))),
            ))
            .when_some(cpu.bus_mhz, |d, b| d.child(stat("Barramento", format!("{b:.1} MHz"), None, None)));
        let toggle = seg::<()>(cx, "cpuview", &[("cores", "Núcleos"), ("threads", "Threads"), ("both", "Ambos")], if both { "both" } else if threads_view { "threads" } else { "cores" }, |this, v| {
            let mut c0 = this.cfg.lock().unwrap();
            c0.cpu_view = v.into();
            let _ = c0.save(&this.cfg_path);
        });
        let brand = brands::cpu(&cpu.name);
        let short = crate::view::clean_cpu(&cpu.name).to_uppercase();
        // the model is already in the name: show only the generation / series
        let gen_ = brand.line.rsplit(" · ").next().filter(|_| brand.line.contains(" · ")).unwrap_or("");
        let title = if gen_.is_empty() { short } else { format!("{short} · {}", gen_.to_uppercase()) };
        panel_logo(INDIGO, brand_mark(&brand, 28.), title, toggle.into_any_element(), t.alerts.iter().any(|a| a.key == "cpu_temp"))
            .child(div().flex().gap(px(12.)).child(die).child(side))
            .when(!phys_ok && view == "cores", |d| d.child(small("Os núcleos físicos aparecem quando os sensores completos estão ativos; mostrando threads.", DIM).mt(px(6.))))
    }

    // ---------------------------------------------------------------- GPU (compact)
    pub(crate) fn gpu_panel(&self, t: &Telemetry) -> Div {
        let g = &t.gpu;
        let short = g.name.replace("NVIDIA ", "").replace("AMD ", "");
        let used = match (g.vram_used, g.vram_total) {
            (Some(u), Some(tt)) if tt > 0 => u as f64 / tt as f64,
            _ => 0.0,
        };
        // One memory chip per 32 bits of bus (GDDR5/6/6X/7 are x32 devices).
        // No VRAM size reported: an integrated GPU (Intel HD/UHD/Iris, AMD APU) that
        // shares the system RAM, so there are no memory chips to draw.
        let shared = g.vram_total.is_none() && g.bus_width.is_none();
        let chips = if shared { 0 } else { g.bus_width.map(|b| (b / 32).clamp(1, 16) as usize).unwrap_or(4) };
        let chip = |i: usize| {
            let fill_ = (used * chips as f64 - i as f64).clamp(0.0, 1.0);
            let col = if fill_ > 0.0 { load_col(g.mem_load.map(|v| v as f64)) } else { DIM };
            div().w(px(16.)).h(px(12.)).rounded(px(1.)).border_1().border_color(ca(col, 0.6)).bg(ca(col, 0.12 + fill_ as f32 * 0.35))
        };
        let half = chips.div_ceil(2);
        let die_t = g.temp.map(|v| v as f64).or(if shared { t.cpu.package_temp } else { None });
        let die_col = temp_col(die_t);
        let card = div()
            .flex()
            .items_center()
            .gap(px(12.))
            .border_1()
            .border_color(c(0x3a4452))
            .rounded(px(4.))
            .bg(c(0x0e1318))
            .px(px(12.))
            .py(px(10.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(5.))
                    .child(div().flex().gap(px(4.)).children((0..half).map(chip)))
                    .child(
                        div()
                            .w(px(84.))
                            .h(px(58.))
                            .rounded(px(3.))
                            .border_1()
                            .border_color(ca(die_col, 0.6))
                            .bg(c(0x1b2028))
                            .flex()
                            .flex_col()
                            .items_center()
                            .justify_center()
                            .child(num(die_t.map_or("—".into(), |v| format!("{v:.0}°")), 22.).text_color(c(die_col)))
                            .child(div().font_family(NUM_FONT).text_size(px(9.5)).text_color(c(MUTED)).child(g.clock.map_or("—".into(), |v| format!("{:.2} GHz", v as f64 / 1000.0)))),
                    )
                    .child(div().flex().gap(px(4.)).children((half..chips).map(chip))),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(label("Uso"))
                    .child(num(format!("{} %", g.load.map_or("—".into(), |v| v.to_string())), 26.).text_color(c(load_col(g.load.map(|v| v as f64)))))
                    .child(segbar(g.load.unwrap_or(0) as f64, 20, GREEN, 5.))
                    .child(small(
                        if shared {
                            "integrada · usa a memória RAM do sistema".to_string()
                        } else {
                            format!("VRAM {} / {} GB · {} chips{}", g.vram_used.map_or("—".into(), gb), g.vram_total.map_or("—".into(), gb), chips, g.bus_width.map_or(String::new(), |b| format!(" · {b}-bit")))
                        },
                        MUTED,
                    )),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .child(draw::fan(self.gpu_fan_angle, 54.))
                    .child(div().font_family(NUM_FONT).text_size(px(10.5)).text_color(c(MUTED)).child(match (g.fan, g.fan_rpm) {
                        (Some(p), Some(r)) => format!("{p}% · {r:.0} rpm"),
                        (Some(p), None) => format!("{p}%"),
                        (None, Some(r)) => format!("{r:.0} rpm"),
                        _ => "fan —".into(),
                    })),
            );
        let no = Some("sem sensor nesta placa".to_string());
        let t1 = |v: Option<f64>| opt(v, |v| format!("{v:.0} °C"));
        // integrated GPU: same chip as the CPU (its temperature is the package's), no
        // hotspot / memory / VRM sensors and no memory of its own
        let die = g.temp.map(|v| v as f64).or(if shared { t.cpu.package_temp } else { None });
        let die_note = if g.temp.is_some() {
            None
        } else if shared && die.is_some() {
            Some("mesmo chip do processador".to_string())
        } else {
            needs_sensors(t).or(no.clone())
        };
        let cells = if shared {
            vec![
                cell("Temperatura", t1(die), Some(temp_col(die)), die_note),
                cell("Consumo", opt(g.power_w, |v| format!("{v:.1} W")), None, g.power_limit_w.map(|l| format!("limite {l:.0} W"))),
                cell("Clock", g.clock.map_or("—".into(), |v| format!("{v} MHz")), None, if g.clock.is_none() { needs_sensors(t) } else { None }),
                cell("Memória", "RAM".into(), None, Some("compartilhada com o sistema".into())),
            ]
        } else {
            // Memory / VRM temperatures only when the card has those sensors (most NVIDIA
            // GDDR6 cards don't expose them); their places go to numbers it does have.
            let mut v = vec![
                cell("Temperatura", t1(g.temp.map(|v| v as f64)), Some(temp_col(g.temp.map(|v| v as f64))), if g.temp.is_none() { needs_sensors(t).or(no.clone()) } else { None }),
                cell("Hotspot", t1(g.hotspot), Some(temp_col(g.hotspot)), if g.hotspot.is_none() { needs_sensors(t).or(no.clone()) } else { None }),
            ];
            if g.mem_temp.is_some() {
                v.push(cell("Memória", t1(g.mem_temp), Some(temp_col(g.mem_temp)), None));
            }
            if g.vrm_temp.is_some() {
                v.push(cell("VRM", t1(g.vrm_temp), Some(temp_col(g.vrm_temp)), None));
            }
            v.push(cell("Consumo", opt(g.power_w, |v| format!("{v:.0} W")), None, g.power_limit_w.map(|l| format!("limite {l:.0} W"))));
            v.push(cell("Tensão", opt(g.voltage, |v| format!("{v:.3} V")), Some(YELLOW), if g.voltage.is_none() { needs_sensors(t) } else { None }));
            v.push(cell("Clock memória", g.mem_clock.map_or("—".into(), |v| format!("{v} MHz")), None, g.mem_load.map(|v| format!("controlador {v}%"))));
            v.push(cell("VRAM", g.vram_used.map_or("—".into(), |u| format!("{} GB", gb(u))), None, Some(format!("{:.0}% de {} GB", used * 100.0, g.vram_total.map_or("—".into(), gb)))));
            let mut extra = vec![
                cell("Clock núcleo", g.clock.map_or("—".into(), |v| format!("{v} MHz")), Some(CYAN), None),
                cell(
                    "Uso da memória",
                    g.mem_load.map_or("—".into(), |v| format!("{v}%")),
                    Some(load_col(g.mem_load.map(|v| v as f64))),
                    Some("controlador de memória".into()),
                ),
            ]
            .into_iter();
            // keep the grid full (2 rows of 4): fill the missing sensors' places
            let mut at = 2;
            while v.len() < 8 {
                match extra.next() {
                    Some(c0) => {
                        v.insert(at, c0);
                        at += 1;
                    }
                    None => break,
                }
            }
            v
        };
        let cap = g.error.clone();
        let gname = if g.name.is_empty() { self.pc.as_ref().map(|p| brands::main_gpu(&p.gpus)).unwrap_or_default() } else { g.name.clone() };
        let brand = brands::gpu(&gname);
        let vram_tag = div().text_size(px(12.)).text_color(c(MUTED)).child(g.vram_total.map_or(String::new(), |v| format!("{} GB", gb(v))));
        panel_logo(GREEN, brand_mark(&brand, 28.), short.to_uppercase(), vram_tag.into_any_element(), t.alerts.iter().any(|a| a.key == "gpu_temp"))
            .child(card)
            .child(div().mt(px(8.)).child(grid(cells, 4)))
            .when_some(cap, |d, e| d.child(small(e, DIM).mt(px(6.))))
    }

    // ---------------------------------------------------------------- memory
    pub(crate) fn ram_panel(&self, t: &Telemetry) -> Div {
        let pct_ = t.mem_pct().unwrap_or(0.0);
        let pc = self.pc.as_ref();
        let mods = pc.map(|p| p.ram.clone()).unwrap_or_default();
        let m0 = mods.first();
        let mts = m0.map(|m| if m.configured > 0 { m.configured } else { m.speed }).unwrap_or(0);
        let kind = m0.map(|m| m.type_name()).unwrap_or("RAM");
        let spd = t.hw.spd.first();
        let cyc = |v: Option<f64>| v.and_then(|v| ram::cycles(v, mts));
        let cl = cyc(spd.and_then(|s| s.taa));
        let timings = match (cl, cyc(spd.and_then(|s| s.trcd)), cyc(spd.and_then(|s| s.trp)), cyc(spd.and_then(|s| s.tras))) {
            (Some(a), Some(b), Some(c0), Some(d)) => Some(format!("{a}-{b}-{c0}-{d}")),
            _ => None,
        };
        let volt = m0.filter(|m| m.mv > 0).map(|m| m.mv as f64 / 1000.0).or(t.hw.board.dram_v);
        let temp = t.hw.spd.iter().filter_map(|s| s.temp).reduce(f64::max);
        let slots_n = pc.map(|p| ram::slot_count(p) as u32).unwrap_or(0);
        let mode = pc.map(ram::channel_mode).unwrap_or("—");
        let cells = vec![
            cell("Frequência", if mts > 0 { format!("{mts} MT/s") } else { "—".into() }, None, (mts > 0).then(|| format!("{} MHz reais · {kind}", mts / 2))),
            cell(
                "Latência",
                cl.map_or("—".into(), |v| format!("CL{v}")),
                Some(CYAN),
                timings.map(|tm| format!("{tm} · {:.2} ns", cl.and_then(|c0| ram::cas_ns(c0, mts)).unwrap_or(0.0))).or(needs_sensors(t)),
            ),
            cell("Tensão", opt(volt, |v| format!("{v:.2} V")), Some(YELLOW), None),
            cell("Temperatura", opt(temp, |v| format!("{v:.0} °C")), Some(temp_col(temp)), if temp.is_none() { Some("o pente não tem sensor".into()) } else { None }),
            cell("Consumo", opt(t.cpu.mem_power, |v| format!("{v:.1} W")), None, t.cpu.mem_power.map(|_| "controlador de memória".into())),
            cell("Canais", mode.replace(" channel", "").to_string(), None, Some(format!("{} de {} slots", mods.len(), slots_n.max(mods.len() as u32)))),
        ];
        let module_rows = mods.iter().enumerate().map(|(i, m)| {
            let s = t.hw.spd.get(i);
            let extra = s
                .and_then(|s| s.trfc.and_then(|v| ram::cycles(v, mts)).map(|rfc| format!(" · tRFC {rfc}")))
                .unwrap_or_default();
            let b = brands::ram(m);
            let name = if b.name.is_empty() { String::new() } else { format!("{} ", b.name) };
            div().flex().items_center().gap(px(8.)).children(brand_mark(&b, 18.)).child(div().flex_1().min_w_0().child(row(
                format!("{} · {:.0} GB {name}{}", m.slot, m.size_gb, m.part),
                format!("{} MT/s{}{extra}", if m.configured > 0 { m.configured } else { m.speed }, if m.speed > m.configured && m.configured > 0 { format!(" (suporta {})", m.speed) } else { String::new() }),
            )))
        });
        panel(AMBER, "MEMÓRIA RAM", format!("{} · {} GB", kind, gb(t.mem.total)), t.alerts.iter().any(|a| a.key == "ram"))
            .child(
                div()
                    .flex()
                    .items_end()
                    .gap(px(10.))
                    .child(num(format!("{pct_:.0}%"), 30.).text_color(c(load_col(Some(pct_)))))
                    .child(div().pb(px(5.)).text_color(c(MUTED)).text_size(px(12.5)).child(format!(
                        "{} de {} GB em uso · virtual {} / {} GB",
                        gb(t.mem.used),
                        gb(t.mem.total),
                        gb(t.mem.swap_used),
                        gb(t.mem.swap_total)
                    ))),
            )
            .child(segbar(pct_, 30, AMBER, 6.))
            .child(div().mt(px(10.)).child(grid(cells, 3)))
            .child(div().mt(px(6.)).children(module_rows))
    }

    // ---------------------------------------------------------------- motherboard
    pub(crate) fn board_panel(&self, t: &Telemetry) -> Div {
        let b = &t.hw.board;
        let Some(pc) = self.pc.as_ref() else {
            return panel(ORANGE, "PLACA-MÃE", "identificando…", false).child(small("lendo informações da placa…", DIM));
        };
        let slots = ram::slots(pc);
        let fans = &t.hw.fans;
        let in_use = fans.iter().filter(|f| f.rpm > 0.0).count();

        // --- sprite: slots + fan headers + RGB ---
        let slot_el = |s: &ram::Slot| {
            let on = s.module.is_some();
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(3.))
                .child(
                    div()
                        .w(px(16.))
                        .h(px(92.))
                        .rounded(px(2.))
                        .border_1()
                        .border_color(c(if on { AMBER } else { 0x3a4452 }))
                        .bg(if on { ca(AMBER, 0.25) } else { c(0x161b22) })
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(div().w(px(4.)).h(px(70.)).rounded(px(1.)).bg(if on { c(AMBER) } else { c(0x222a35) })),
                )
                .child(div().font_family(NUM_FONT).text_size(px(9.5)).text_color(c(if on { TEXT } else { DIM })).child(s.name.clone()))
                .child(div().font_family(NUM_FONT).text_size(px(9.)).text_color(c(MUTED)).child(s.module.as_ref().map_or("vazio".into(), |m| format!("{:.0} GB", m.size_gb))))
        };
        let header_el = |i: usize, f: &markii_core::hw::Fan| {
            let on = f.rpm > 0.0;
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(2.))
                .child(
                    div()
                        .flex()
                        .gap(px(2.))
                        .p(px(3.))
                        .rounded(px(2.))
                        .border_1()
                        .border_color(c(if on { CYAN } else { 0x3a4452 }))
                        .bg(if on { ca(CYAN, 0.15) } else { c(0x161b22) })
                        .children((0..4).map(move |_| div().size(px(4.)).rounded(px(1.)).bg(c(if on { CYAN } else { 0x4a5462 })))),
                )
                .child(div().font_family(NUM_FONT).text_size(px(9.)).text_color(c(if on { TEXT } else { DIM })).child(if f.label.starts_with("Fan #") { format!("F{}", i + 1) } else { f.label.clone() }))
                .child(div().font_family(NUM_FONT).text_size(px(9.)).text_color(c(MUTED)).child(if on { format!("{:.0}", f.rpm) } else { "—".into() }))
        };
        let sprite = div()
            .flex()
            .gap(px(14.))
            .border_1()
            .border_color(c(0x2f3a2c))
            .rounded(px(4.))
            .bg(c(0x101712))
            .p(px(10.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(label("Slots de RAM"))
                    .child(div().flex().gap(px(6.)).children(slots.iter().map(slot_el))),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .child(label(&format!("Conectores de fan · {in_use} de {} em uso", b.headers)))
                            .child(if fans.is_empty() {
                                small("sem leitura do chip da placa", DIM).into_any_element()
                            } else {
                                div().flex().flex_wrap().gap(px(8.)).children(fans.iter().enumerate().map(|(i, f)| header_el(i, f))).into_any_element()
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .child(label("RGB"))
                            .child(if pc.rgb_devices.is_empty() {
                                small("nenhum controlador RGB por USB encontrado", MUTED)
                            } else {
                                div().flex().flex_wrap().gap(px(4.)).children(pc.rgb_devices.iter().map(|d| {
                                    div().px(px(6.)).py(px(1.)).rounded(px(4.)).border_1().border_color(ca(PINK, 0.5)).text_size(px(11.)).text_color(c(PINK)).child(d.clone())
                                }))
                            })
                            .child(small("Conectores RGB/ARGB da placa (5 V/12 V) são passivos: nenhum software consegue ver se estão em uso.", DIM)),
                    ),
            );

        let t1 = |v: Option<f64>| opt(v, |v| format!("{v:.0} °C"));
        let no = || Some("a placa não tem esse sensor".to_string());
        let measured = t.cpu.power.unwrap_or(0.0) + t.gpu.power_w.unwrap_or(0.0);
        let cells = vec![
            cell("VRM da CPU", t1(b.vrm), Some(temp_col(b.vrm)), if b.vrm.is_none() { needs_sensors(t).or(no()) } else { None }),
            cell("VRM da memória", t1(b.vrm_mem), Some(temp_col(b.vrm_mem)), if b.vrm_mem.is_none() { needs_sensors(t).or(no()) } else { None }),
            cell("Chipset", t1(b.pch), Some(temp_col(b.pch)), if b.pch.is_none() { no() } else { None }),
            cell(
                "Consumo da placa",
                opt(b.power, |v| format!("{v:.0} W")),
                None,
                Some(if b.power.is_some() { "sensor da placa".into() } else if measured > 0.0 { format!("sem sensor · CPU+GPU medidos {measured:.0} W") } else { "sem sensor".into() }),
            ),
        ];
        let named_t: Vec<_> = b.temps.iter().filter(|x| !x.label.starts_with("Temperature #")).collect();
        let named_v: Vec<_> = b.volts.iter().filter(|x| !x.label.starts_with("Voltage #")).collect();
        let unnamed = b.temps.len() - named_t.len() + b.volts.len() - named_v.len();
        let chipsub = if b.chip.is_empty() { "chip de sensores não lido".to_string() } else { b.chip.clone() };
        let brand = brands::board(&pc.board_maker, &pc.board);
        let who = if brand.name.is_empty() { pc.board_maker.clone() } else { brand.name.clone() };
        let chip_tag = div().text_size(px(12.)).text_color(c(MUTED)).child(chipsub);
        panel_logo(ORANGE, brand_mark(&brand, 28.), format!("PLACA-MÃE · {who} {}", pc.board).to_uppercase(), chip_tag.into_any_element(), false)
            .child(sprite)
            .child(div().mt(px(8.)).child(grid(cells, 4)))
            .when(!named_t.is_empty() || !named_v.is_empty(), |d| {
                d.child(
                    div()
                        .mt(px(8.))
                        .flex()
                        .flex_wrap()
                        .gap(px(6.))
                        .children(named_t.iter().map(|x| chip_kv(&x.label, format!("{:.0} °C", x.value))))
                        .children(named_v.iter().map(|x| chip_kv(&x.label, format!("{:.3} V", x.value)))),
                )
            })
            .when(unnamed > 0, |d| d.child(small(format!("+{unnamed} leituras sem nome no chip (esta placa não identifica os canais)"), DIM).mt(px(4.))))
    }

    // ---------------------------------------------------------------- fans
    pub(crate) fn fans_panel(&self, t: &Telemetry) -> Div {
        let fans = &t.hw.fans;
        let in_use = fans.iter().filter(|f| f.rpm > 0.0).count();
        let gpu_fan = t.gpu.fan.is_some() || t.gpu.fan_rpm.is_some();
        let total = in_use + usize::from(gpu_fan);
        let mut rows: Vec<Div> = Vec::new();
        for f in fans {
            let on = f.rpm > 0.0;
            rows.push(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .py(px(6.))
                    .border_b_1()
                    .border_color(c(LINE))
                    .text_size(px(12.5))
                    .child(div().size(px(7.)).rounded(px(4.)).bg(c(if on { CYAN } else { 0x3a4452 })))
                    .child(div().flex_1().min_w_0().text_color(c(if on { TEXT } else { DIM })).child(f.label.clone()))
                    .child(div().w(px(90.)).child(segbar(f.pct.unwrap_or(0.0), 10, if on { CYAN } else { DIM }, 4.)))
                    .child(div().w(px(110.)).flex().justify_end().font_family(NUM_FONT).text_color(c(MUTED)).child(if on {
                        format!("{:.0} rpm · {}", f.rpm, f.pct.map_or("—".into(), |p| format!("{p:.0}%")))
                    } else {
                        "desconectado".into()
                    })),
            );
        }
        if gpu_fan {
            rows.push(row(
                "GPU",
                match (t.gpu.fan, t.gpu.fan_rpm) {
                    (Some(p), Some(r)) => format!("{r:.0} rpm · {p}%"),
                    (Some(p), None) => format!("{p}%"),
                    (None, Some(r)) => format!("{r:.0} rpm"),
                    _ => "—".into(),
                },
            ));
        }
        let tag = if fans.is_empty() { "sem leitura".to_string() } else { format!("{total} girando · {} conectores", fans.len()) };
        let p = panel(CYAN, "VENTOINHAS", tag, false).child(
            div()
                .flex()
                .items_end()
                .gap(px(6.))
                .mb(px(6.))
                .child(num(total.to_string(), 30.).text_color(c(CYAN)))
                .child(div().pb(px(5.)).text_color(c(MUTED)).text_size(px(12.5)).child(if total == 1 { "ventoinha detectada" } else { "ventoinhas detectadas" })),
        );
        let p = p.children(rows);
        if fans.is_empty() {
            p.child(small(
                if cfg!(windows) {
                    "Ative os sensores completos (PawnIO + administrador) para ler as ventoinhas da placa-mãe."
                } else {
                    "O kernel não expôs o chip de sensores da placa (hwmon). Em muitas placas é preciso carregar o módulo, por exemplo: sudo modprobe nct6775 (ou rode sensors-detect)."
                },
                DIM,
            )
            .pt(px(6.)))
        } else {
            p.child(small("Ventoinhas ligadas em hub/controladora aparecem como uma só no conector.", DIM).pt(px(6.)))
        }
    }

    // ---------------------------------------------------------------- drive health
    pub(crate) fn drives_rows(&self, t: &Telemetry) -> Vec<AnyElement> {
        t.hw
            .drives
            .iter()
            .map(|d| {
                let mut v = Vec::new();
                if let Some(x) = d.temp {
                    v.push(format!("{x:.0} °C"));
                }
                if let Some(x) = d.life {
                    v.push(format!("vida {x:.0}%"));
                }
                if let Some(x) = d.hours {
                    v.push(format!("{:.0} h ligado", x));
                }
                row(d.name.clone(), v.join(" · ")).into_any_element()
            })
            .collect()
    }
}

fn chip_kv(k: &str, v: String) -> Div {
    div()
        .flex()
        .gap(px(5.))
        .px(px(7.))
        .py(px(2.))
        .rounded(px(4.))
        .border_1()
        .border_color(c(LINE2))
        .bg(c(PANEL2))
        .text_size(px(11.5))
        .child(div().text_color(c(MUTED)).child(k.to_string()))
        .child(div().font_family(NUM_FONT).child(v))
}

// ---------------------------------------------------------------- phone / headset
impl Dashboard {
    /// One panel per phone or headset running the Mark II app that reported in the
    /// last seconds (battery, heat, Wi-Fi...). None when nothing is connected.
    pub(crate) fn devices_panel(&self) -> Option<Div> {
        let devs = markii_core::remote::global().devices();
        let vr = crate::vr::global().batteries();
        if devs.is_empty() && vr.is_empty() {
            return None;
        }
        let vr_panel = (!vr.is_empty()).then(|| {
            let low = vr.iter().any(|b| b.pct <= 15.0 && !b.charging);
            let rows = vr.into_iter().map(|b| {
                let col = if b.pct <= 15.0 { RED } else if b.pct <= 30.0 { ORANGE } else { GREEN };
                div()
                    .mt(px(8.))
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .text_size(px(12.5))
                            .child(div().text_color(c(MUTED)).child(if b.charging { format!("{} · carregando", b.name) } else { b.name }))
                            .child(num(format!("{:.0}%", b.pct), 15.).text_color(c(col))),
                    )
                    .child(segbar(b.pct as f64, 30, col, 5.))
            });
            panel(CYAN, "STEAMVR", "baterias".to_string(), low).children(rows)
        });
        let panels = devs.into_iter().map(|d| {
            let bat = d.battery;
            let bat_col = match bat {
                Some(b) if b <= 15.0 => RED,
                Some(b) if b <= 30.0 => ORANGE,
                Some(_) => GREEN,
                None => MUTED,
            };
            let thermal = d.thermal.map(|t| match t {
                0 => ("normal", GREEN),
                1 => ("morno", GREEN),
                2 => ("quente", YELLOW),
                3 => ("muito quente", ORANGE),
                _ => ("crítico", RED),
            });
            let wifi = d.wifi_rssi.map(|r| {
                let q = if r >= -55 { "ótimo" } else if r >= -67 { "bom" } else if r >= -75 { "fraco" } else { "ruim" };
                let col = if r >= -67 { GREEN } else if r >= -75 { YELLOW } else { RED };
                (format!("{r} dBm"), q, col)
            });
            let up = d.uptime.map(|s| {
                let (h, m) = (s / 3600, s % 3600 / 60);
                if h >= 24 { format!("{}d {}h", h / 24, h % 24) } else { format!("{h}h {m}m") }
            });
            let cells = vec![
                cell(
                    "Aquecimento",
                    thermal.map_or("—".into(), |t| t.0.to_string()),
                    thermal.map(|t| t.1),
                    d.cpu_temp.map(|v| format!("processador {v:.0} °C")),
                ),
                cell("Temp. da bateria", opt(d.battery_temp, |v| format!("{v:.0} °C")), Some(temp_col(d.battery_temp.map(|v| v + 25.0))), None),
                cell(
                    "Memória",
                    match (d.ram_used, d.ram_total) {
                        (Some(u), Some(t)) if t > 0 => format!("{:.0}%", u as f64 * 100.0 / t as f64),
                        _ => "—".into(),
                    },
                    None,
                    d.ram_total.map(|t| format!("de {} GB", gb(t))),
                ),
                cell(
                    "Armazenamento",
                    d.storage_free.map_or("—".into(), |f| format!("{} GB livres", gb(f))),
                    None,
                    d.storage_total.map(|t| format!("de {} GB", gb(t))),
                ),
                cell("Wi-Fi", wifi.as_ref().map_or("—".into(), |w| w.1.to_string()), wifi.as_ref().map(|w| w.2), wifi.as_ref().map(|w| {
                    format!("{}{}", w.0, d.wifi_mbps.map(|m| format!(" · {m} Mbit/s")).unwrap_or_default())
                })),
                cell("Ligado há", up.unwrap_or_else(|| "—".into()), None, None),
            ];
            panel(CYAN, "ÓCULOS / CELULAR", d.name.clone(), bat.is_some_and(|b| b <= 15.0) || d.thermal.is_some_and(|t| t >= 3))
                .child(
                    div()
                        .flex()
                        .items_end()
                        .gap(px(10.))
                        .child(num(bat.map_or("—".into(), |b| format!("{b:.0}%")), 30.).text_color(c(bat_col)))
                        .child(div().pb(px(5.)).text_color(c(MUTED)).text_size(px(12.5)).child(if d.charging {
                            "bateria · carregando".to_string()
                        } else {
                            "bateria".to_string()
                        })),
                )
                .child(segbar(bat.unwrap_or(0.0), 30, bat_col, 6.))
                .child(div().mt(px(10.)).child(grid(cells, 3)))
        });
        Some(div().flex().gap(px(12.)).children(panels.map(|p| p.flex_1())).children(vr_panel.map(|p| p.flex_1())))
    }
}
