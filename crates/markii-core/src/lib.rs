//! Mark II core: everything that reads the PC, independent of any UI.
//! Front ends (GPUI, Tauri) create a [`Collector`] and call [`Collector::sample`] on a timer.

pub mod alerts;
pub mod cpufreq;
pub mod hw;
pub mod hwinfo;
pub mod pcinfo;

pub use alerts::{Active, Config};
pub use pcinfo::{Cooling, PcInfo};

use alerts::Check;
use nvml_wrapper::enum_wrappers::device::{Clock, TemperatureSensor};
use nvml_wrapper::Nvml;
use serde::Serialize;
use std::collections::HashMap;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use sysinfo::{Disks, Networks, ProcessesToUpdate, System};

#[derive(Serialize, Clone, Default, Debug)]
pub struct CoreT {
    pub idx: usize,
    pub load: f32,
    pub mhz: u64,
    pub temp: Option<f64>,
}

#[derive(Serialize, Clone, Default, Debug)]
pub struct CpuT {
    pub name: String,
    pub load: f32,
    pub mhz_avg: u64,
    pub cores: Vec<CoreT>,
    pub package_temp: Option<f64>,
    pub core_max: Option<f64>,
    pub power: Option<f64>,
    pub vcore: Option<f64>,
}

#[derive(Serialize, Clone, Default, Debug)]
pub struct GpuT {
    pub name: String,
    pub temp: Option<u32>,
    pub load: Option<u32>,
    pub mem_load: Option<u32>,
    pub vram_used: Option<u64>,
    pub vram_total: Option<u64>,
    pub power_w: Option<f64>,
    pub power_limit_w: Option<f64>,
    pub clock: Option<u32>,
    pub mem_clock: Option<u32>,
    pub fan: Option<u32>,
    pub error: Option<String>,
}

#[derive(Serialize, Clone, Default, Debug)]
pub struct MemT {
    pub used: u64,
    pub total: u64,
    pub swap_used: u64,
    pub swap_total: u64,
}

#[derive(Serialize, Clone, Default, Debug)]
pub struct DiskT {
    pub name: String,
    pub mount: String,
    pub kind: String,
    pub used: u64,
    pub total: u64,
}

#[derive(Serialize, Clone, Default, Debug)]
pub struct NetT {
    pub rx_bps: f64,
    pub tx_bps: f64,
}

#[derive(Serialize, Clone, Default, Debug)]
pub struct ProcT {
    pub name: String,
    pub cpu: f32,
    pub mem: u64,
}

#[derive(Serialize, Clone, Default, Debug)]
pub struct Telemetry {
    pub ts: u64,
    pub host: String,
    pub uptime: u64,
    pub cpu: CpuT,
    pub gpu: GpuT,
    pub mem: MemT,
    pub disks: Vec<DiskT>,
    pub net: NetT,
    pub procs: Vec<ProcT>,
    pub fans: Vec<hw::Fan>,
    pub hwinfo: Option<String>,
    pub alerts: Vec<Active>,
    pub cooling: Cooling,
}

impl Telemetry {
    pub fn mem_pct(&self) -> Option<f64> {
        pct(self.mem.used, self.mem.total)
    }

    /// Short multi-line summary for a tray tooltip.
    pub fn tooltip(&self) -> String {
        let temp = |v: Option<f64>| v.map_or("—".to_string(), |v| format!("{v:.0}°C"));
        let mut tip = format!(
            "Mark II\nCPU {:.0}% · {}\nGPU {}% · {}\nRAM {:.0}%",
            self.cpu.load,
            temp(self.cpu.package_temp),
            self.gpu.load.map_or("—".into(), |v| v.to_string()),
            temp(self.gpu.temp.map(|v| v as f64)),
            self.mem_pct().unwrap_or(0.0),
        );
        if !self.alerts.is_empty() {
            tip.push_str(&format!("\n⚠ {}", self.alerts.iter().map(|a| a.title.as_str()).collect::<Vec<_>>().join(", ")));
        }
        tip
    }
}

pub fn pct(u: u64, tot: u64) -> Option<f64> {
    if tot > 0 { Some(u as f64 / tot as f64 * 100.0) } else { None }
}

/// Owns all sensor handles; create it and call `sample` from one worker thread.
pub struct Collector {
    sys: System,
    disks: Disks,
    nets: Networks,
    nvml: Result<Nvml, String>,
    freq: Option<cpufreq::CpuFreq>,
    engine: alerts::Engine,
    no_overrides: HashMap<String, String>,
    last: Instant,
    tick: u64,
}

impl Default for Collector {
    fn default() -> Self {
        Self::new()
    }
}

impl Collector {
    pub fn new() -> Self {
        let sys = System::new_all();
        let n = sys.cpus().len().max(1);
        Self {
            sys,
            disks: Disks::new_with_refreshed_list(),
            nets: Networks::new_with_refreshed_list(),
            nvml: Nvml::init().map_err(|e| format!("driver NVIDIA indisponível: {e}")),
            freq: cpufreq::CpuFreq::new(n),
            engine: alerts::Engine::default(),
            no_overrides: HashMap::new(),
            last: Instant::now(),
            tick: 0,
        }
    }

    fn gpu(&self) -> GpuT {
        let nvml = match &self.nvml {
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

    /// Takes one sample. Returns the telemetry and the alerts that should notify now.
    pub fn sample(&mut self, cfg: &Config, pc: Option<&PcInfo>) -> (Telemetry, Vec<Active>) {
        let dt = self.last.elapsed().as_secs_f64().max(0.001);
        self.last = Instant::now();
        let cpu_count = self.sys.cpus().len().max(1) as f32;

        self.sys.refresh_cpu_all();
        self.sys.refresh_memory();
        self.sys.refresh_processes(ProcessesToUpdate::All, true);
        self.nets.refresh(true);
        if self.tick % 10 == 0 {
            self.disks.refresh(true);
        }
        self.tick += 1;

        // HWiNFO is optional: temperatures, power and fans.
        let (hwsnap, hwinfo_err) = match hwinfo::read() {
            Ok(r) => (Some(hw::map(&r, &self.no_overrides)), None),
            Err(e) => (None, Some(e)),
        };
        let hcore = |i: usize| hwsnap.as_ref().and_then(|h| h.cpu.cores.iter().find(|c| c.idx as usize == i));
        let real = self.freq.as_mut().map(|f| f.sample()).unwrap_or_default();
        let cpus = self.sys.cpus();
        let cores: Vec<CoreT> = cpus
            .iter()
            .enumerate()
            .map(|(i, c)| CoreT {
                idx: i,
                load: c.cpu_usage(),
                mhz: hcore(i)
                    .and_then(|h| h.clock.map(|v| v.round() as u64))
                    .or(real.get(i).copied().flatten())
                    .unwrap_or(c.frequency()),
                temp: hcore(i).and_then(|h| h.temp),
            })
            .collect();
        let mhz_avg = if cores.is_empty() { 0 } else { cores.iter().map(|c| c.mhz).sum::<u64>() / cores.len() as u64 };
        let hc = hwsnap.as_ref().map(|h| &h.cpu);

        let (rx, tx) = self.nets.iter().fold((0u64, 0u64), |(r, t), (_, d)| (r + d.received(), t + d.transmitted()));
        let mut procs: Vec<ProcT> = self
            .sys
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

        let fans = hwsnap.as_ref().map(|h| h.fans.clone()).unwrap_or_default();
        let mut t = Telemetry {
            ts: SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0),
            host: System::host_name().unwrap_or_default(),
            uptime: System::uptime(),
            cpu: CpuT {
                name: cpus.first().map(|c| c.brand().trim().to_string()).unwrap_or_default(),
                load: self.sys.global_cpu_usage(),
                mhz_avg,
                cores,
                package_temp: hc.and_then(|c| c.package),
                core_max: hc.and_then(|c| c.core_max),
                power: hc.and_then(|c| c.power),
                vcore: hc.and_then(|c| c.vcore),
            },
            gpu: self.gpu(),
            mem: MemT {
                used: self.sys.used_memory(),
                total: self.sys.total_memory(),
                swap_used: self.sys.used_swap(),
                swap_total: self.sys.total_swap(),
            },
            disks: self
                .disks
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
            cooling: pcinfo::detect(&cfg.cooling, pc, &fans, hwinfo_err.is_none()),
            fans,
            hwinfo: hwinfo_err,
            alerts: Vec::new(),
        };

        let mut checks = vec![
            Check { key: "cpu_temp".into(), title: "CPU quente".into(), value: t.cpu.package_temp, limit: cfg.cpu_temp, unit: "°C", hyst: 3.0 },
            Check { key: "gpu_temp".into(), title: "GPU quente".into(), value: t.gpu.temp.map(|v| v as f64), limit: cfg.gpu_temp, unit: "°C", hyst: 3.0 },
            Check { key: "ram".into(), title: "RAM quase cheia".into(), value: t.mem_pct(), limit: cfg.ram_pct, unit: "%", hyst: 3.0 },
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
        let (active, fire) = self.engine.step(cfg, checks);
        t.alerts = active;
        (t, fire)
    }
}
