#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod alerts;
mod hw;
mod hwinfo;
mod pcinfo;

use alerts::{Check, Config};
use nvml_wrapper::enum_wrappers::device::{Clock, TemperatureSensor};
use nvml_wrapper::Nvml;
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use sysinfo::{Disks, Networks, ProcessesToUpdate, System};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, Manager, State, WindowEvent};
use tauri_plugin_notification::NotificationExt;

#[derive(Serialize, Default)]
struct CoreT {
    idx: usize,
    load: f32,
    mhz: u64,
    temp: Option<f64>,
}

#[derive(Serialize, Default)]
struct CpuT {
    name: String,
    load: f32,
    mhz_avg: u64,
    cores: Vec<CoreT>,
    package_temp: Option<f64>,
    core_max: Option<f64>,
    power: Option<f64>,
    vcore: Option<f64>,
}

#[derive(Serialize, Default)]
struct GpuT {
    name: String,
    temp: Option<u32>,
    load: Option<u32>,
    mem_load: Option<u32>,
    vram_used: Option<u64>,
    vram_total: Option<u64>,
    power_w: Option<f64>,
    power_limit_w: Option<f64>,
    clock: Option<u32>,
    mem_clock: Option<u32>,
    fan: Option<u32>,
    error: Option<String>,
}

#[derive(Serialize, Default)]
struct MemT {
    used: u64,
    total: u64,
    swap_used: u64,
    swap_total: u64,
}

#[derive(Serialize)]
struct DiskT {
    name: String,
    mount: String,
    kind: String,
    used: u64,
    total: u64,
}

#[derive(Serialize, Default)]
struct NetT {
    rx_bps: f64,
    tx_bps: f64,
}

#[derive(Serialize)]
struct ProcT {
    name: String,
    cpu: f32,
    mem: u64,
}

#[derive(Serialize)]
struct Telemetry {
    ts: u64,
    host: String,
    uptime: u64,
    cpu: CpuT,
    gpu: GpuT,
    mem: MemT,
    disks: Vec<DiskT>,
    net: NetT,
    procs: Vec<ProcT>,
    fans: Vec<hw::Fan>,
    hwinfo: Option<String>,
    alerts: Vec<alerts::Active>,
    cooling: pcinfo::Cooling,
}

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

fn gpu_snapshot(nvml: &Result<Nvml, String>) -> GpuT {
    let nvml = match nvml {
        Ok(n) => n,
        Err(e) => return GpuT { error: Some(e.clone()), ..Default::default() },
    };
    let dev = match nvml.device_by_index(0) {
        Ok(d) => d,
        Err(e) => return GpuT { error: Some(e.to_string()), ..Default::default() },
    };
    let util = dev.utilization_rates().ok();
    let mem = dev.memory_info().ok();
    GpuT {
        name: dev.name().unwrap_or_default(),
        temp: dev.temperature(TemperatureSensor::Gpu).ok(),
        load: util.as_ref().map(|u| u.gpu),
        mem_load: util.as_ref().map(|u| u.memory),
        vram_used: mem.as_ref().map(|m| m.used),
        vram_total: mem.as_ref().map(|m| m.total),
        power_w: dev.power_usage().ok().map(|mw| mw as f64 / 1000.0),
        power_limit_w: dev.enforced_power_limit().ok().map(|mw| mw as f64 / 1000.0),
        clock: dev.clock_info(Clock::Graphics).ok(),
        mem_clock: dev.clock_info(Clock::Memory).ok(),
        fan: dev.fan_speed(0).ok(),
        error: None,
    }
}

fn worker(app: tauri::AppHandle, shared: Arc<Shared>) {
    let mut sys = System::new_all();
    let mut disks = Disks::new_with_refreshed_list();
    let mut nets = Networks::new_with_refreshed_list();
    let nvml = Nvml::init().map_err(|e| format!("NVIDIA driver not available: {e}"));
    let no_overrides: HashMap<String, String> = HashMap::new();
    let cpu_count = sys.cpus().len().max(1) as f32;
    let mut last = Instant::now();
    let mut tick: u64 = 0;
    let mut engine = alerts::Engine::default();

    loop {
        let cfg = shared.cfg();
        std::thread::sleep(Duration::from_millis(cfg.interval_ms));
        let dt = last.elapsed().as_secs_f64().max(0.001);
        last = Instant::now();

        sys.refresh_cpu_all();
        sys.refresh_memory();
        sys.refresh_processes(ProcessesToUpdate::All, true);
        nets.refresh(true);
        if tick % 10 == 0 {
            disks.refresh(true);
        }
        tick += 1;

        // HWiNFO is optional: it fills in temperatures, power and fans.
        let (hwsnap, hwinfo_err) = match hwinfo::read() {
            Ok(r) => (Some(hw::map(&r, &no_overrides)), None),
            Err(e) => (None, Some(e)),
        };
        let core_temp = |i: usize| -> Option<f64> {
            hwsnap.as_ref()?.cpu.cores.iter().find(|c| c.idx as usize == i)?.temp
        };

        let cpus = sys.cpus();
        let cores: Vec<CoreT> = cpus
            .iter()
            .enumerate()
            .map(|(i, c)| CoreT { idx: i, load: c.cpu_usage(), mhz: c.frequency(), temp: core_temp(i) })
            .collect();
        let mhz_avg = if cores.is_empty() { 0 } else { cores.iter().map(|c| c.mhz).sum::<u64>() / cores.len() as u64 };
        let hc = hwsnap.as_ref().map(|h| &h.cpu);

        let (rx, tx) = nets.iter().fold((0u64, 0u64), |(r, t), (_, d)| (r + d.received(), t + d.transmitted()));

        let mut procs: Vec<ProcT> = sys
            .processes()
            .values()
            .filter(|p| p.pid().as_u32() != 0)
            .map(|p| ProcT {
                name: p.name().to_string_lossy().trim_end_matches(".exe").to_string(),
                cpu: p.cpu_usage() / cpu_count,
                mem: p.memory(),
            })
            .collect();
        procs.sort_by(|a, b| b.cpu.partial_cmp(&a.cpu).unwrap_or(std::cmp::Ordering::Equal));
        procs.truncate(6);

        let mut t = Telemetry {
            ts: SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0),
            host: System::host_name().unwrap_or_default(),
            uptime: System::uptime(),
            cpu: CpuT {
                name: cpus.first().map(|c| c.brand().trim().to_string()).unwrap_or_default(),
                load: sys.global_cpu_usage(),
                mhz_avg,
                cores,
                package_temp: hc.and_then(|c| c.package),
                core_max: hc.and_then(|c| c.core_max),
                power: hc.and_then(|c| c.power),
                vcore: hc.and_then(|c| c.vcore),
            },
            gpu: gpu_snapshot(&nvml),
            mem: MemT {
                used: sys.used_memory(),
                total: sys.total_memory(),
                swap_used: sys.used_swap(),
                swap_total: sys.total_swap(),
            },
            disks: disks
                .list()
                .iter()
                .map(|d| DiskT {
                    name: d.name().to_string_lossy().to_string(),
                    mount: d.mount_point().to_string_lossy().trim_end_matches('\\').to_string(),
                    kind: format!("{:?}", d.kind()),
                    used: d.total_space().saturating_sub(d.available_space()),
                    total: d.total_space(),
                })
                .collect(),
            net: NetT { rx_bps: rx as f64 / dt, tx_bps: tx as f64 / dt },
            procs,
            fans: hwsnap.as_ref().map(|h| h.fans.clone()).unwrap_or_default(),
            hwinfo: hwinfo_err,
            alerts: Vec::new(),
            cooling: Default::default(),
        };
        t.cooling = {
            let pc = shared.pc.lock().unwrap();
            pcinfo::detect(&cfg.cooling, pc.as_ref(), &t.fans, t.hwinfo.is_none())
        };

        // ---- alerts ----
        let pct = |u: u64, tot: u64| if tot > 0 { Some(u as f64 / tot as f64 * 100.0) } else { None };
        let mut checks = vec![
            Check { key: "cpu_temp".into(), title: "CPU quente".into(), value: t.cpu.package_temp, limit: cfg.cpu_temp, unit: "°C", hyst: 3.0 },
            Check { key: "gpu_temp".into(), title: "GPU quente".into(), value: t.gpu.temp.map(|v| v as f64), limit: cfg.gpu_temp, unit: "°C", hyst: 3.0 },
            Check { key: "ram".into(), title: "RAM quase cheia".into(), value: pct(t.mem.used, t.mem.total), limit: cfg.ram_pct, unit: "%", hyst: 3.0 },
        ];
        for d in &t.disks {
            checks.push(Check {
                key: format!("disk:{}", d.mount),
                title: format!("Disco {} quase cheio", d.mount),
                value: pct(d.used, d.total),
                limit: cfg.disk_pct,
                unit: "%",
                hyst: 1.0,
            });
        }
        let (active, fire) = engine.step(&cfg, checks);
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
        t.alerts = active;

        // ---- tray tooltip ----
        if let Some(tray) = app.tray_by_id("main") {
            let temp = |v: Option<f64>| v.map_or("—".to_string(), |v| format!("{v:.0}°C"));
            let mut tip = format!(
                "Mark II\nCPU {:.0}% · {}\nGPU {}% · {}\nRAM {:.0}%",
                t.cpu.load,
                temp(t.cpu.package_temp),
                t.gpu.load.map_or("—".into(), |v| v.to_string()),
                temp(t.gpu.temp.map(|v| v as f64)),
                pct(t.mem.used, t.mem.total).unwrap_or(0.0),
            );
            if !t.alerts.is_empty() {
                tip.push_str(&format!("\n⚠ {}", t.alerts.iter().map(|a| a.title.as_str()).collect::<Vec<_>>().join(", ")));
            }
            let _ = tray.set_tooltip(Some(tip));
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
