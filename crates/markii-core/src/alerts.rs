//! Threshold alerts with a hold time (avoids firing on a 1-sample spike),
//! hysteresis (re-arms only after the value drops clearly below the limit)
//! and a cooldown between repeated notifications.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct Config {
    pub alerts_enabled: bool,
    pub notify: bool,
    pub cpu_temp: f64,
    pub gpu_temp: f64,
    pub ram_pct: f64,
    pub disk_pct: f64,
    /// Seconds a value must stay above the limit before alerting.
    pub hold_s: u64,
    pub cooldown_min: u64,
    pub close_to_tray: bool,
    pub interval_ms: u64,
    /// "auto" | "air" | "water"
    pub cooling: String,
    /// Start the elevated LibreHardwareMonitor helper (fans, voltages, power, board).
    pub sensors_admin: bool,
    /// CPU panel view: "cores" | "threads" | "both"
    pub cpu_view: String,
    /// "grafite" (default) | "roxo" | "lavanda" | "menta" (dark pastel) | "algodao" | "pistache" (light pastel)
    pub theme: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            alerts_enabled: true,
            notify: true,
            cpu_temp: 85.0,
            gpu_temp: 83.0,
            ram_pct: 90.0,
            disk_pct: 95.0,
            hold_s: 5,
            cooldown_min: 5,
            close_to_tray: true,
            interval_ms: 1000,
            cooling: "auto".into(),
            sensors_admin: true,
            cpu_view: "cores".into(),
            theme: "grafite".into(),
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }
    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let s = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, s).map_err(|e| e.to_string())
    }
}

#[derive(Serialize, Clone, Debug)]
pub struct Active {
    pub key: String,
    pub title: String,
    pub value: f64,
    pub limit: f64,
    pub unit: String,
}

/// A value to check this tick: key, human label, current value, limit, unit, hysteresis.
pub struct Check {
    pub key: String,
    pub title: String,
    pub value: Option<f64>,
    pub limit: f64,
    pub unit: &'static str,
    pub hyst: f64,
}

#[derive(Default)]
struct St {
    above_since: Option<Instant>,
    active: bool,
    last_notified: Option<Instant>,
}

#[derive(Default)]
pub struct Engine {
    st: HashMap<String, St>,
}

impl Engine {
    /// Returns (currently active alerts, alerts that should notify now).
    pub fn step(&mut self, cfg: &Config, checks: Vec<Check>) -> (Vec<Active>, Vec<Active>) {
        let mut active = Vec::new();
        let mut fire = Vec::new();
        if !cfg.alerts_enabled {
            self.st.clear();
            return (active, fire);
        }
        let now = Instant::now();
        let hold = Duration::from_secs(cfg.hold_s);
        let cooldown = Duration::from_secs(cfg.cooldown_min * 60);
        for c in checks {
            let s = self.st.entry(c.key.clone()).or_default();
            let Some(v) = c.value.filter(|v| v.is_finite()) else {
                s.above_since = None;
                continue;
            };
            if v >= c.limit {
                let since = *s.above_since.get_or_insert(now);
                if !s.active && now.duration_since(since) >= hold {
                    s.active = true;
                }
            } else {
                s.above_since = None;
                if s.active && v < c.limit - c.hyst {
                    s.active = false;
                }
            }
            if s.active {
                let a = Active { key: c.key, title: c.title, value: v, limit: c.limit, unit: c.unit.into() };
                if s.last_notified.map_or(true, |t| now.duration_since(t) >= cooldown) {
                    s.last_notified = Some(now);
                    fire.push(a.clone());
                }
                active.push(a);
            }
        }
        (active, fire)
    }
}
