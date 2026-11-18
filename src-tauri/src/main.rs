#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod hw;
mod hwinfo;

use nvml_wrapper::enum_wrappers::device::{Clock, TemperatureSensor};
use nvml_wrapper::Nvml;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use sysinfo::{Disks, Networks, ProcessesToUpdate, System};
use tauri::{Emitter, Manager, State};

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
}

struct Shared {
    interval_ms: AtomicU64,
}

#[tauri::command]
fn set_interval(st: State<Arc<Shared>>, ms: u64) {
    st.interval_ms.store(ms.clamp(250, 10_000), Ordering::Relaxed);
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

    loop {
        std::thread::sleep(Duration::from_millis(shared.interval_ms.load(Ordering::Relaxed)));
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

        let t = Telemetry {
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
        };
        let _ = app.emit("telemetry", &t);
    }
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![set_interval])
        .setup(|app| {
            let shared = Arc::new(Shared { interval_ms: AtomicU64::new(1000) });
            app.manage(shared.clone());
            let handle = app.handle().clone();
            std::thread::spawn(move || worker(handle, shared));
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Mark II");
}
