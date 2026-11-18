#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use markii_core::{alerts::Config, pcinfo, Collector};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, Manager, State, WindowEvent};
use tauri_plugin_notification::NotificationExt;

struct Shared {
    cfg: Mutex<Config>,
    path: PathBuf,
    pc: Mutex<Option<pcinfo::PcInfo>>,
}

#[tauri::command]
fn get_pcinfo(st: State<Arc<Shared>>) -> Option<pcinfo::PcInfo> {
    st.pc.lock().unwrap().clone()
}

impl Shared {
    fn cfg(&self) -> Config {
        self.cfg.lock().unwrap().clone()
    }
}

#[tauri::command]
fn get_config(st: State<Arc<Shared>>) -> Config {
    st.cfg()
}

#[tauri::command]
fn save_config(st: State<Arc<Shared>>, cfg: Config) -> Result<Config, String> {
    let mut cfg = cfg;
    cfg.interval_ms = cfg.interval_ms.clamp(250, 10_000);
    cfg.hold_s = cfg.hold_s.min(600);
    cfg.save(&st.path)?;
    *st.cfg.lock().unwrap() = cfg.clone();
    Ok(cfg)
}

#[tauri::command]
fn set_interval(st: State<Arc<Shared>>, ms: u64) -> Result<(), String> {
    let mut c = st.cfg.lock().unwrap();
    c.interval_ms = ms.clamp(250, 10_000);
    c.save(&st.path)
}

#[tauri::command]
fn test_alert(app: tauri::AppHandle) -> Result<(), String> {
    app.notification()
        .builder()
        .title("Mark II · teste")
        .body("As notificações estão funcionando.")
        .show()
        .map_err(|e| e.to_string())
}

fn worker(app: tauri::AppHandle, shared: Arc<Shared>) {
    let mut col = Collector::new();
    loop {
        let cfg = shared.cfg();
        std::thread::sleep(Duration::from_millis(cfg.interval_ms));
        let (t, fire) = {
            let pc = shared.pc.lock().unwrap().clone();
            col.sample(&cfg, pc.as_ref())
        };
        if cfg.notify {
            for a in &fire {
                let _ = app
                    .notification()
                    .builder()
                    .title(format!("Mark II · {}", a.title))
                    .body(format!("{:.0} {} (limite {:.0} {})", a.value, a.unit, a.limit, a.unit))
                    .show();
            }
        }
        if let Some(tray) = app.tray_by_id("main") {
            let _ = tray.set_tooltip(Some(t.tooltip()));
        }
        let _ = app.emit("telemetry", &t);
    }
}

fn show_main(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .invoke_handler(tauri::generate_handler![get_config, save_config, set_interval, test_alert, get_pcinfo])
        .on_window_event(|w, ev| {
            if let WindowEvent::CloseRequested { api, .. } = ev {
                let st = w.app_handle().state::<Arc<Shared>>();
                if st.cfg().close_to_tray {
                    api.prevent_close();
                    let _ = w.hide();
                }
            }
        })
        .setup(|app| {
            let path = app.path().app_config_dir()?.join("config.json");
            let shared = Arc::new(Shared { cfg: Mutex::new(Config::load(&path)), path, pc: Mutex::new(None) });
            app.manage(shared.clone());
            {
                let (sh, h) = (shared.clone(), app.handle().clone());
                std::thread::spawn(move || {
                    let info = pcinfo::collect();
                    *sh.pc.lock().unwrap() = Some(info.clone());
                    let _ = h.emit("pcinfo", &info);
                });
            }

            let show = MenuItem::with_id(app, "show", "Mostrar Mark II", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Sair", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;
            TrayIconBuilder::with_id("main")
                .icon(app.default_window_icon().cloned().ok_or("no icon")?)
                .tooltip("Mark II")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, ev| match ev.id().as_ref() {
                    "show" => show_main(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, ev| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = ev {
                        let app = tray.app_handle();
                        match app.get_webview_window("main") {
                            Some(w) if w.is_visible().unwrap_or(false) && !w.is_minimized().unwrap_or(false) => { let _ = w.hide(); }
                            _ => show_main(app),
                        }
                    }
                })
                .build(app)?;

            let handle = app.handle().clone();
            std::thread::spawn(move || worker(handle, shared));
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Mark II");
}
