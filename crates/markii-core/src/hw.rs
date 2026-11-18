//! Maps raw HWiNFO readings onto the fields the dashboard shows.
//! Any field can be re-pointed with an override in config:
//!   "hwinfo_overrides": { "cpu.vcore": "Nuvoton::Vcore", "gpu.chip.D1": "MyProbe::D1" }

use crate::hwinfo::Reading;
use regex::Regex;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::sync::OnceLock;

const TEMP: u32 = 1;
const VOLT: u32 = 2;
const FAN: u32 = 3;
const POWER: u32 = 5;
const CLOCK: u32 = 6;
const USAGE: u32 = 7;

pub const CHIP_SLOTS: [&str; 8] = ["D1", "D0", "C1", "C0", "B1", "A0", "A1", "B0"];

#[derive(Serialize, Clone, Default, Debug)]
pub struct Core {
    pub kind: String,
    pub idx: u32,
    pub temp: Option<f64>,
    pub clock: Option<f64>,
    pub vid: Option<f64>,
    pub load: Option<f64>,
}

#[derive(Serialize, Clone, Default, Debug)]
pub struct Dimm {
    pub name: String,
    pub temp: Option<f64>,
    pub power: Option<f64>,
    pub voltage: Option<f64>,
}

#[derive(Serialize, Clone, Default, Debug)]
pub struct Cpu {
    pub name: String,
    pub package: Option<f64>,
    pub core_max: Option<f64>,
    pub power: Option<f64>,
    pub ia_power: Option<f64>,
    pub sa_power: Option<f64>,
    pub vcore: Option<f64>,
    pub vrm: Option<f64>,
    pub vr_vcc: Option<f64>,
    pub load: Option<f64>,
    pub ring_clock: Option<f64>,
    pub cores: Vec<Core>,
    pub dimms: Vec<Dimm>,
    /// Voltage the CPU itself reports (VID / SVI telemetry).
    pub vid: Option<f64>,
    pub bus: Option<f64>,
    pub mem_power: Option<f64>,
}

#[derive(Serialize, Clone, Default, Debug)]
pub struct Gpu {
    pub name: String,
    pub temp: Option<f64>,
    pub hotspot: Option<f64>,
    pub mem_junction: Option<f64>,
    pub clock: Option<f64>,
    pub mem_clock: Option<f64>,
    pub load: Option<f64>,
    pub mem_load: Option<f64>,
    pub voltage: Option<f64>,
    /// Board power as the card reports it.
    pub power: Option<f64>,
    pub p16: Option<f64>,
    pub v16: Option<f64>,
    pub slot: Option<f64>,
    pub vslot: Option<f64>,
    pub core_power: Option<f64>,
    pub mem_power: Option<f64>,
    pub chips: BTreeMap<String, Option<f64>>,
    pub vrm: Option<f64>,
    pub fan_rpm: Option<f64>,
    pub fan_pct: Option<f64>,
    pub vram_used_mb: Option<f64>,
    pub vram_total_mb: Option<f64>,
}

#[derive(Serialize, Clone, Default, Debug)]
pub struct Fan {
    pub label: String,
    pub rpm: f64,
    /// PWM / DC duty the header is driven at, when known.
    pub pct: Option<f64>,
}

#[derive(Serialize, Clone, Default, Debug)]
pub struct Named {
    pub label: String,
    pub value: f64,
}

/// Motherboard sensor chip (Super I/O / EC) readings.
#[derive(Serialize, Clone, Default, Debug)]
pub struct Board {
    pub chip: String,
    pub temps: Vec<Named>,
    pub volts: Vec<Named>,
    /// Number of fan headers the chip exposes.
    pub headers: usize,
    pub vrm: Option<f64>,
    pub vrm_mem: Option<f64>,
    pub pch: Option<f64>,
    pub dram_v: Option<f64>,
    pub power: Option<f64>,
}

/// One memory module as read from its SPD chip (times in ns).
#[derive(Serialize, Clone, Default, Debug)]
pub struct Spd {
    pub name: String,
    pub capacity_gb: Option<f64>,
    pub tck_min: Option<f64>,
    pub taa: Option<f64>,
    pub trcd: Option<f64>,
    pub trp: Option<f64>,
    pub tras: Option<f64>,
    pub trc: Option<f64>,
    pub trfc: Option<f64>,
    pub temp: Option<f64>,
}

#[derive(Serialize, Clone, Default, Debug)]
pub struct Drive {
    pub name: String,
    pub temp: Option<f64>,
    pub life: Option<f64>,
    pub hours: Option<f64>,
}

#[derive(Serialize, Clone, Default, Debug)]
pub struct HwSnapshot {
    pub cpu: Cpu,
    pub gpu: Gpu,
    pub fans: Vec<Fan>,
    pub board: Board,
    pub spd: Vec<Spd>,
    pub drives: Vec<Drive>,
}

struct Finder<'a> {
    r: &'a [Reading],
    ov: &'a HashMap<String, String>,
}

impl<'a> Finder<'a> {
    fn label_match(rd: &Reading, pat: &str, exact: bool) -> bool {
        let p = pat.to_lowercase();
        [&rd.label, &rd.label_user].iter().any(|l| {
            let l = l.to_lowercase();
            !l.is_empty() && if exact { l == p } else { l.contains(&p) }
        })
    }

    fn get(&self, key: &str, kind: u32, sensor: &str, labels: &[&str]) -> Option<f64> {
        if let Some(o) = self.ov.get(key) {
            let (s, l) = o.split_once("::").unwrap_or(("", o.as_str()));
            let s = s.to_lowercase();
            return self
                .r
                .iter()
                .find(|rd| rd.sensor.to_lowercase().contains(&s) && Self::label_match(rd, l, false))
                .map(|rd| rd.value)
                .filter(|v| v.is_finite());
        }
        let s = sensor.to_lowercase();
        for exact in [true, false] {
            for l in labels {
                if let Some(rd) = self.r.iter().find(|rd| {
                    rd.kind == kind
                        && rd.sensor.to_lowercase().contains(&s)
                        && Self::label_match(rd, l, exact)
                }) {
                    if rd.value.is_finite() {
                        return Some(rd.value);
                    }
                }
            }
        }
        None
    }
}

fn device_name(r: &[Reading], prefix: &str) -> String {
    r.iter()
        .find(|rd| rd.sensor.starts_with(prefix))
        .map(|rd| {
            let rest = rd.sensor[prefix.len()..].trim_start_matches(':').trim();
            rest.split(':').next().unwrap_or(rest).trim().to_string()
        })
        .unwrap_or_default()
}

fn core_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"^(?:([PE])-core |Core #?)(\d+)(?: T\d+)?(?: (.+))?$").unwrap())
}

fn dimm_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"DIMM\s*\[#(\d+)\]").unwrap())
}

pub fn map(r: &[Reading], ov: &HashMap<String, String>) -> HwSnapshot {
    let f = Finder { r, ov };
    let c = "cpu [#0]";
    let g = "gpu [#0]";

    // --- per-core ---
    let mut cores: BTreeMap<u32, Core> = BTreeMap::new();
    for rd in r.iter().filter(|rd| rd.sensor.to_lowercase().starts_with(c)) {
        let Some(cap) = core_re().captures(&rd.label) else { continue };
        let kind = cap.get(1).map_or("P", |m| m.as_str()).to_string();
        let idx: u32 = cap[2].parse().unwrap_or(0);
        let suffix = cap.get(3).map(|m| m.as_str()).unwrap_or("");
        let e = cores.entry(idx).or_insert_with(|| Core { kind, idx, ..Default::default() });
        let v = rd.value;
        match (rd.kind, suffix) {
            (TEMP, "") => { e.temp.get_or_insert(v); }
            (CLOCK, "Clock") => { e.clock.get_or_insert(v); }
            (VOLT, "VID") => { e.vid.get_or_insert(v); }
            (USAGE, "Usage") => e.load = Some(e.load.map_or(v, |o: f64| o.max(v))),
            _ => {}
        }
    }

    // --- DIMMs ---
    let mut dimms: BTreeMap<u32, Dimm> = BTreeMap::new();
    for rd in r {
        let Some(cap) = dimm_re().captures(&rd.sensor) else { continue };
        let n: u32 = cap[1].parse().unwrap_or(0);
        let d = dimms.entry(n).or_insert_with(|| Dimm { name: format!("DIMM {n}"), ..Default::default() });
        match rd.kind {
            TEMP => { d.temp.get_or_insert(rd.value); }
            POWER => { d.power.get_or_insert(rd.value); }
            VOLT if rd.label.to_uppercase().contains("VDD") => { d.voltage.get_or_insert(rd.value); }
            _ => {}
        }
    }

    // --- fans / pumps (motherboard + others, not GPU) ---
    let fans = r
        .iter()
        .filter(|rd| rd.kind == FAN && !rd.sensor.to_lowercase().starts_with(g) && rd.value.is_finite())
        .map(|rd| Fan {
            label: if rd.label_user.is_empty() { rd.label.clone() } else { rd.label_user.clone() },
            rpm: rd.value,
            pct: None,
        })
        .collect();

    let mut chips = BTreeMap::new();
    for slot in CHIP_SLOTS {
        let key = format!("gpu.chip.{slot}");
        chips.insert(slot.to_string(), if ov.contains_key(&key) { f.get(&key, TEMP, "", &[]) } else { None });
    }

    HwSnapshot {
        cpu: Cpu {
            name: device_name(r, "CPU [#0]"),
            package: f.get("cpu.package", TEMP, c, &["CPU Package"]),
            core_max: f.get("cpu.core_max", TEMP, c, &["Core Max"]),
            power: f.get("cpu.power", POWER, c, &["CPU Package Power"]),
            ia_power: f.get("cpu.ia_power", POWER, c, &["IA Cores Power"]),
            sa_power: f.get("cpu.sa_power", POWER, c, &["System Agent Power"]),
            vcore: f.get("cpu.vcore", VOLT, "", &["Vcore", "CPU Core Voltage"]),
            vrm: f.get("cpu.vrm", TEMP, "", &["VRM", "MOS"]),
            vr_vcc: f.get("cpu.vr_vcc", TEMP, c, &["VR VCC Temperature (SVID)", "VR VCC"]),
            load: f.get("cpu.load", USAGE, c, &["Total CPU Usage"]),
            ring_clock: f.get("cpu.ring_clock", CLOCK, c, &["Ring/LLC Clock", "Uncore Clock"]),
            cores: cores.into_values().collect(),
            dimms: dimms.into_values().collect(),
            vid: None,
            bus: None,
            mem_power: f.get("cpu.mem_power", POWER, c, &["DRAM Power"]),
        },
        gpu: Gpu {
            name: device_name(r, "GPU [#0]"),
            temp: f.get("gpu.temp", TEMP, g, &["GPU Temperature"]),
            hotspot: f.get("gpu.hotspot", TEMP, g, &["GPU Hot Spot Temperature", "Hot Spot"]),
            mem_junction: f.get("gpu.mem_junction", TEMP, g, &["GPU Memory Junction Temperature"]),
            clock: f.get("gpu.clock", CLOCK, g, &["GPU Clock"]),
            mem_clock: f.get("gpu.mem_clock", CLOCK, g, &["GPU Memory Clock"]),
            load: f.get("gpu.load", USAGE, g, &["GPU Core Load"]),
            mem_load: f.get("gpu.mem_load", USAGE, g, &["GPU Memory Controller Load"]),
            voltage: f.get("gpu.voltage", VOLT, g, &["GPU Core Voltage"]),
            power: f.get("gpu.power", POWER, g, &["GPU Power"]),
            p16: f.get("gpu.p16", POWER, g, &["GPU 16-pin HVPWR Power", "GPU 16-pin Input Power", "16-pin"]),
            v16: f.get("gpu.v16", VOLT, g, &["GPU 16-pin HVPWR Voltage", "GPU 16-pin Input Voltage", "16-pin"]),
            slot: f.get("gpu.slot", POWER, g, &["GPU PCIe +12V Input Power"]),
            vslot: f.get("gpu.vslot", VOLT, g, &["GPU PCIe +12V Input Voltage"]),
            core_power: f.get("gpu.core_power", POWER, g, &["GPU Core (NVVDD) Input Power (sum)", "GPU Core (NVVDD) Input Power", "NVVDD"]),
            mem_power: f.get("gpu.mem_power", POWER, g, &["GPU FBVDD Input Power", "FBVDD"]),
            chips,
            vrm: f.get("gpu.vrm", TEMP, g, &["GPU VRM Temperature", "VRM"]),
            ..Default::default()
        },
        board: Board {
            vrm: f.get("board.vrm", TEMP, "", &["VRM MOS", "VRM", "MOS"]),
            pch: f.get("board.pch", TEMP, "", &["PCH", "Chipset"]),
            dram_v: f.get("board.dram_v", VOLT, "", &["DRAM", "VDIMM"]),
            ..Default::default()
        },
        fans,
        ..Default::default()
    }
}
