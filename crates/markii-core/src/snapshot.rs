//! Compact, read-only view of the telemetry for the SteamVR overlay and the
//! Android app (`/v1/snapshot`). Only numbers to show; nothing that identifies
//! files, processes or the network beyond the PC name.
use crate::Telemetry;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Default, Debug, PartialEq)]
pub struct Snapshot {
    pub v: u32,
    pub host: String,
    pub uptime: u64,
    pub cpu_name: String,
    pub cpu_load: f32,
    pub cpu_temp: Option<f64>,
    pub cpu_power: Option<f64>,
    pub cpu_mhz: u64,
    pub gpu_name: String,
    pub gpu_load: Option<u32>,
    pub gpu_temp: Option<u32>,
    pub gpu_hotspot: Option<f64>,
    pub gpu_power: Option<f64>,
    pub gpu_clock: Option<u32>,
    pub vram_used: Option<u64>,
    pub vram_total: Option<u64>,
    pub gpu_fan: Option<u32>,
    pub ram_used: u64,
    pub ram_total: u64,
    pub net_rx: f64,
    pub net_tx: f64,
    /// (name, rpm)
    pub fans: Vec<(String, f64)>,
    /// titles of active alerts
    pub alerts: Vec<String>,
}

impl Snapshot {
    pub fn from(t: &Telemetry) -> Self {
        Self {
            v: 1,
            host: t.host.clone(),
            uptime: t.uptime,
            cpu_name: t.cpu.name.clone(),
            cpu_load: t.cpu.load,
            cpu_temp: t.cpu.package_temp,
            cpu_power: t.cpu.power,
            cpu_mhz: t.cpu.mhz_avg,
            gpu_name: t.gpu.name.clone(),
            gpu_load: t.gpu.load,
            gpu_temp: t.gpu.temp,
            gpu_hotspot: t.gpu.hotspot,
            gpu_power: t.gpu.power_w,
            gpu_clock: t.gpu.clock,
            vram_used: t.gpu.vram_used,
            vram_total: t.gpu.vram_total,
            gpu_fan: t.gpu.fan,
            ram_used: t.mem.used,
            ram_total: t.mem.total,
            net_rx: t.net.rx_bps,
            net_tx: t.net.tx_bps,
            fans: t.fans.iter().map(|f| (f.label.clone(), f.rpm)).collect(),
            alerts: t.alerts.iter().map(|a| a.title.clone()).collect(),
        }
    }

    pub fn ram_pct(&self) -> f64 {
        if self.ram_total == 0 { 0.0 } else { self.ram_used as f64 * 100.0 / self.ram_total as f64 }
    }
}
