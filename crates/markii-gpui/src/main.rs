#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! Mark II — native GPUI front end.

mod draw;
mod panels;
mod rgb_view;
mod theme;
mod view;
mod vr;

use gpui::{px, size, App, AppContext, Application, Bounds, TitlebarOptions, WindowBounds, WindowOptions};
use markii_core::{pcinfo, Collector, Config, PcInfo, Telemetry};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::path::PathBuf;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};
#[cfg(windows)]
use tray_icon::menu::{Menu, MenuEvent, MenuItem};
#[cfg(windows)]
use tray_icon::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use view::Dashboard;

pub fn notify(title: &str, body: &str) {
    let _ = notify_rust::Notification::new().summary(title).body(body).show();
}

/// Same file the Tauri build uses, so settings carry over.
/// Windows: %APPDATA%, Linux: $XDG_CONFIG_HOME or ~/.config.
fn config_path() -> PathBuf {
    #[cfg(windows)]
    let base = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(not(windows))]
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")));
    base.unwrap_or_else(|| PathBuf::from(".")).join("com.akaineko.markii").join("config.json")
}

/// Starts the read-only server for the Android app; paired devices are saved in the config.
pub fn start_remote(cfg: &Arc<Mutex<Config>>, path: &std::path::Path) {
    let (port, tokens) = {
        let c = cfg.lock().unwrap();
        (c.remote_port, c.remote_tokens.clone())
    };
    let (cfg, path) = (cfg.clone(), path.to_path_buf());
    markii_core::remote::global().start(
        port,
        tokens,
        Box::new(move |t| {
            let mut c = cfg.lock().unwrap();
            c.remote_tokens = t;
            let _ = c.save(&path);
        }),
    );
}

/// The tray needs GTK's own loop on Linux, which GPUI does not run: tray only on Windows.
pub const HAS_TRAY: bool = cfg!(windows);

static HWND: AtomicIsize = AtomicIsize::new(0);

#[cfg(windows)]
mod win {
    use windows_sys::Win32::UI::WindowsAndMessaging::{IsIconic, IsWindowVisible, SetForegroundWindow, ShowWindow, SW_RESTORE, SW_SHOW};
    pub fn show(h: isize) {
        unsafe {
            if IsIconic(h as _) != 0 {
                ShowWindow(h as _, SW_RESTORE);
            }
            ShowWindow(h as _, SW_SHOW);
            SetForegroundWindow(h as _);
        }
    }
    pub fn visible(h: isize) -> bool {
        unsafe { IsWindowVisible(h as _) != 0 && IsIconic(h as _) == 0 }
    }
}

#[cfg(windows)]
fn tray_icon() -> Option<tray_icon::Icon> {
    let bytes = include_bytes!("../../../src-tauri/icons/32x32.png");
    let dec = png::Decoder::new(std::io::Cursor::new(&bytes[..]));
    let mut r = dec.read_info().ok()?;
    let mut buf = vec![0; r.output_buffer_size()];
    let info = r.next_frame(&mut buf).ok()?;
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => buf[..info.buffer_size()].to_vec(),
        png::ColorType::Rgb => buf[..info.buffer_size()].chunks(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        _ => return None,
    };
    tray_icon::Icon::from_rgba(rgba, info.width, info.height).ok()
}

fn main() {
    // Login entry ("Reaplicar ao ligar o PC"): apply the saved colors and exit, no window.
    if std::env::args().any(|a| a == "--apply-rgb") {
        let _ = markii_core::rgb::apply_saved();
        return;
    }
    // SteamVR wrist panel, started by Mark II itself (see vr.rs).
    if std::env::args().any(|a| a == "--vr-overlay") {
        std::process::exit(vr::run_overlay());
    }
    // Dev check of the wrist panel drawing: markii --vr-preview out.png
    if let Some(i) = std::env::args().position(|a| a == "--vr-preview") {
        let out = std::env::args().nth(i + 1).unwrap_or_else(|| "vr-preview.png".into());
        let s = markii_core::snapshot::Snapshot {
            v: 1,
            host: "DESKTOP-MARKII".into(),
            cpu_load: 37.0,
            cpu_temp: Some(64.0),
            cpu_power: Some(71.0),
            gpu_load: Some(92),
            gpu_temp: Some(78),
            gpu_power: Some(186.0),
            vram_used: Some(6_800_000_000),
            vram_total: Some(8_589_934_592),
            ram_used: 19_000_000_000,
            ram_total: 34_359_738_368,
            fans: vec![("CPU Fan".into(), 1180.0), ("System Fan #1".into(), 950.0), ("Pump".into(), 2400.0)],
            ..Default::default()
        };
        let _ = vr::preview_png(std::path::Path::new(&out), &s);
        return;
    }
    let cfg_path = config_path();
    let cfg = Arc::new(Mutex::new(Config::load(&cfg_path)));
    theme::set_theme(&cfg.lock().unwrap().theme);
    let pc: Arc<Mutex<Option<PcInfo>>> = Arc::new(Mutex::new(None));

    {
        let pc = pc.clone();
        std::thread::spawn(move || *pc.lock().unwrap() = Some(pcinfo::collect()));
    }

    if cfg.lock().unwrap().vr_overlay {
        vr::global().start();
    }
    if cfg.lock().unwrap().remote_enabled {
        start_remote(&cfg, &cfg_path);
    }

    let (tx, rx) = mpsc::channel::<(Telemetry, Vec<markii_core::Active>)>();
    {
        let (cfg, pc) = (cfg.clone(), pc.clone());
        std::thread::spawn(move || {
            let mut col = Collector::new();
            loop {
                let c = cfg.lock().unwrap().clone();
                std::thread::sleep(Duration::from_millis(c.interval_ms.clamp(250, 10_000)));
                let p = pc.lock().unwrap().clone();
                let sample = col.sample(&c, p.as_ref());
                let vr_on = vr::global().running();
                let remote_on = markii_core::remote::global().running();
                if vr_on || remote_on {
                    let snap = markii_core::snapshot::Snapshot::from(&sample.0);
                    if vr_on {
                        vr::global().send(&snap);
                    }
                    if remote_on {
                        markii_core::remote::global().publish(snap);
                    }
                }
                if tx.send(sample).is_err() {
                    break;
                }
            }
        });
    }

    Application::new().run(move |cx: &mut App| {
        // Fit the window to the screen it opens on (small or high-DPI screens would get a
        // window bigger than the screen) and center it there.
        let display = cx.primary_display();
        let (dw, dh) = display
            .as_ref()
            .map(|d| {
                let b = d.bounds();
                (f32::from(b.size.width), f32::from(b.size.height))
            })
            .unwrap_or((1920., 1080.));
        // the height leaves room for the taskbar and the title bar
        let (w, h) = (1120f32.min(dw * 0.9), 860f32.min(dh * 0.8));
        let bounds = Bounds::centered(display.map(|d| d.id()), size(px(w), px(h)), cx);
        let opts = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions { title: Some("Mark II".into()), ..Default::default() }),
            window_min_size: Some(size(px(940f32.min(w)), px(600f32.min(h)))),
            ..Default::default()
        };
        let view = cx.new(|_| Dashboard {
            t: None,
            pc: None,
            cfg: cfg.clone(),
            cfg_path: cfg_path.clone(),
            hist: Default::default(),
            fan_angle: 0.0,
            gpu_fan_angle: 0.0,
            pump_angle: 0.0,
            settings: None,
            save_msg: "".into(),
            rgb: None,
        });
        let v2 = view.clone();
        cx.open_window(opts, move |window, cx| {
            if let Ok(h) = window.window_handle() {
                if let RawWindowHandle::Win32(w) = h.as_raw() {
                    HWND.store(w.hwnd.get(), Ordering::Relaxed);
                }
            }
            // Closing the window always ends Mark II and its helpers (OpenRGB, sensors):
            // nothing is left running in the background.
            window.on_window_should_close(cx, move |_, cx| {
                markii_core::rgb::global().stop();
                vr::global().stop();
                markii_core::remote::global().stop();
                cx.quit();
                true
            });
            v2
        })
        .expect("failed to open window");

        // Tray icon with Show / Quit.
        #[cfg(windows)]
        let show_i = MenuItem::new("Mostrar Mark II", true, None);
        #[cfg(windows)]
        let (tray, show_i, quit_i) = {
            let quit_i = MenuItem::new("Sair", true, None);
            let menu = Menu::new();
            let _ = menu.append(&show_i);
            let _ = menu.append(&quit_i);
            let mut tb = TrayIconBuilder::new().with_tooltip("Mark II").with_menu(Box::new(menu)).with_menu_on_left_click(false);
            if let Some(i) = tray_icon() {
                tb = tb.with_icon(i);
            }
            (tb.build().ok(), show_i, quit_i)
        };
        #[cfg(windows)]
        let (show_id, quit_id) = (show_i.id().clone(), quit_i.id().clone());

        let cfg = cfg.clone();
        let pc = pc.clone();
        cx.spawn(async move |cx| {
            #[cfg(windows)]
            let _keep = (show_i, quit_i);
            let mut last_anim = Instant::now();
            loop {
                cx.background_executor().timer(Duration::from_millis(80)).await;
                #[cfg(windows)]
                let hwnd = HWND.load(Ordering::Relaxed);
                #[cfg(windows)]
                let visible = win::visible(hwnd);
                #[cfg(not(windows))]
                let visible = true;

                // New samples from the collector thread.
                while let Ok((t, fire)) = rx.try_recv() {
                    if cfg.lock().unwrap().notify {
                        for a in &fire {
                            notify(&format!("Mark II · {}", a.title), &format!("{:.0} {} (limite {:.0} {})", a.value, a.unit, a.limit, a.unit));
                        }
                    }
                    #[cfg(windows)]
                    if let Some(tr) = &tray {
                        let _ = tr.set_tooltip(Some(t.tooltip()));
                    }
                    let pcv = pc.lock().unwrap().clone();
                    let _ = view.update(cx, |v, cx| {
                        if v.pc.is_none() {
                            v.pc = pcv;
                        }
                        v.ingest(t);
                        cx.notify();
                    });
                }

                // Tray clicks and menu.
                #[cfg(windows)]
                while let Ok(ev) = TrayIconEvent::receiver().try_recv() {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = ev {
                        #[cfg(windows)]
                        win::show(hwnd);
                    }
                }
                #[cfg(windows)]
                while let Ok(ev) = MenuEvent::receiver().try_recv() {
                    if ev.id == show_id {
                        #[cfg(windows)]
                        win::show(hwnd);
                    } else if ev.id == quit_id {
                        markii_core::rgb::global().stop();
                        vr::global().stop();
                        markii_core::remote::global().stop();
                        let _ = cx.update(|cx| cx.quit());
                    }
                }

                // Sprite animation (~12 fps) only while the window is on screen.
                let dt = last_anim.elapsed().as_secs_f32();
                last_anim = Instant::now();
                if visible {
                    let _ = view.update(cx, |v, cx| {
                        v.animate(dt);
                        cx.notify();
                    });
                }
            }
        })
        .detach();
        cx.activate(true);
    });
}
