//! Linux sensor backend. The kernel already exposes the sensor chips through
//! sysfs (hwmon), so no driver or helper process is needed. Readings are put in
//! the same shape the LibreHardwareMonitor helper uses on Windows
//! ([`lhm::Sample`]) so [`lhm::map`] turns them into a [`HwSnapshot`] the same
//! way on both systems.
//!
//! Sources: /sys/class/hwmon (coretemp, k10temp, Super I/O chips such as
//! nct6775/it87, nvme, drivetemp, amdgpu), /sys/devices/system/cpu (clocks and
//! topology), /proc/stat (per-core load) and /sys/class/powercap (RAPL; only
//! readable by root on recent kernels).

use crate::lhm::{Hw, Sample, Sensor};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

fn read(p: &Path) -> Option<String> {
    fs::read_to_string(p).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn num(p: &Path) -> Option<f64> {
    read(p)?.parse().ok()
}

fn s(t: &str, n: impl Into<String>, v: Option<f64>) -> Sensor {
    Sensor { t: t.into(), n: n.into(), v }
}

/// Inputs of one hwmon device grouped by kind: (kind, index) -> (label, value).
struct Chip {
    name: String,
    dir: PathBuf,
    /// kind = "temp" | "fan" | "in" | "power" | "pwm" | "freq"
    inputs: BTreeMap<(String, u32), (Option<String>, f64)>,
}

fn chips() -> Vec<Chip> {
    let mut out = Vec::new();
    let Ok(rd) = fs::read_dir("/sys/class/hwmon") else { return out };
    for e in rd.flatten() {
        let dir = e.path();
        let name = read(&dir.join("name")).unwrap_or_default();
        let mut inputs = BTreeMap::new();
        let Ok(files) = fs::read_dir(&dir) else { continue };
        for f in files.flatten() {
            let fname = f.file_name().to_string_lossy().to_string();
            // tempN_input, fanN_input, inN_input, powerN_average/_input, freqN_input, pwmN
            let (kind, rest) = match ["temp", "fan", "in", "power", "freq", "pwm"].iter().find(|k| fname.starts_with(**k)) {
                Some(k) => (*k, &fname[k.len()..]),
                None => continue,
            };
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            let Ok(idx) = digits.parse::<u32>() else { continue };
            let suffix = &rest[digits.len()..];
            let ok = match kind {
                "pwm" => suffix.is_empty(),
                "power" => suffix == "_average" || suffix == "_input",
                _ => suffix == "_input",
            };
            if !ok {
                continue;
            }
            let Some(raw) = num(&f.path()) else { continue };
            let v = match kind {
                "temp" => raw / 1000.0,
                "in" => raw / 1000.0,
                "power" => raw / 1_000_000.0,
                "freq" => raw / 1_000_000.0,
                "pwm" => raw / 255.0 * 100.0,
                _ => raw,
            };
            let label = read(&dir.join(format!("{kind}{idx}_label")));
            inputs.entry((kind.to_string(), idx)).or_insert((label, v));
        }
        out.push(Chip { name, dir, inputs });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

const SUPERIO: [&str; 14] = [
    "nct", "it8", "it87", "f71", "w83", "asus", "asusec", "dell_smm", "thinkpad", "applesmc", "gigabyte", "sch5", "nzxt", "corsair",
];

/// Logical CPU -> (package, core_id), from sysfs topology.
fn topology() -> Vec<(usize, u32, u32)> {
    let mut v = Vec::new();
    let Ok(rd) = fs::read_dir("/sys/devices/system/cpu") else { return v };
    for e in rd.flatten() {
        let n = e.file_name().to_string_lossy().to_string();
        let Some(i) = n.strip_prefix("cpu").and_then(|x| x.parse::<usize>().ok()) else { continue };
        let t = e.path().join("topology");
        let pkg = num(&t.join("physical_package_id")).unwrap_or(0.0) as u32;
        let core = num(&t.join("core_id")).unwrap_or(i as f64) as u32;
        v.push((i, pkg, core));
    }
    v.sort();
    v
}

pub struct Reader {
    prev_stat: Vec<(u64, u64)>,
    prev_energy: BTreeMap<String, (f64, Instant)>,
}

impl Default for Reader {
    fn default() -> Self {
        Self::new()
    }
}

impl Reader {
    pub fn new() -> Self {
        Self { prev_stat: Vec::new(), prev_energy: BTreeMap::new() }
    }

    /// Per logical CPU busy % since the previous call.
    fn loads(&mut self) -> Vec<Option<f64>> {
        let txt = fs::read_to_string("/proc/stat").unwrap_or_default();
        let now: Vec<(u64, u64)> = txt
            .lines()
            .filter(|l| l.starts_with("cpu") && l.as_bytes().get(3).is_some_and(|c| c.is_ascii_digit()))
            .map(|l| {
                let f: Vec<u64> = l.split_whitespace().skip(1).filter_map(|x| x.parse().ok()).collect();
                let idle = f.get(3).copied().unwrap_or(0) + f.get(4).copied().unwrap_or(0);
                (f.iter().sum::<u64>(), idle)
            })
            .collect();
        let out = now
            .iter()
            .enumerate()
            .map(|(i, (tot, idle))| {
                let (pt, pi) = self.prev_stat.get(i).copied()?;
                let dt = tot.checked_sub(pt)?;
                (dt > 0).then(|| 100.0 * (1.0 - (idle.saturating_sub(pi)) as f64 / dt as f64))
            })
            .collect();
        self.prev_stat = now;
        out
    }

    /// RAPL energy counter -> watts since the previous call (root-only on most kernels).
    fn rapl(&mut self, zone: &Path, key: &str) -> Option<f64> {
        let e = num(&zone.join("energy_uj"))?;
        let now = Instant::now();
        let w = self.prev_energy.get(key).and_then(|(pe, pt)| {
            let dt = now.duration_since(*pt).as_secs_f64();
            (e >= *pe && dt > 0.05).then(|| (e - pe) / 1e6 / dt)
        });
        self.prev_energy.insert(key.into(), (e, now));
        w
    }

    pub fn sample(&mut self) -> Sample {
        let mut hw = Vec::new();
        let topo = topology();
        let loads = self.loads();
        // physical cores of package 0, in core_id order
        let mut core_ids: Vec<u32> = topo.iter().filter(|t| t.1 == 0).map(|t| t.2).collect();
        core_ids.sort();
        core_ids.dedup();
        let pos = |cid: u32| core_ids.iter().position(|c| *c == cid);

        // ---- CPU ----
        let cpu_name = fs::read_to_string("/proc/cpuinfo")
            .ok()
            .and_then(|t| t.lines().find(|l| l.starts_with("model name")).and_then(|l| l.split(':').nth(1)).map(|s| s.trim().to_string()))
            .unwrap_or_else(|| "CPU".into());
        let mut cpu = Hw { t: "Cpu".into(), n: cpu_name, id: "/cpu/0".into(), ..Default::default() };
        for (k, cid) in core_ids.iter().enumerate() {
            let threads: Vec<usize> = topo.iter().filter(|t| t.1 == 0 && t.2 == *cid).map(|t| t.0).collect();
            let mhz = threads
                .iter()
                .filter_map(|i| num(Path::new(&format!("/sys/devices/system/cpu/cpu{i}/cpufreq/scaling_cur_freq"))))
                .map(|khz| khz / 1000.0)
                .reduce(f64::max);
            let ls: Vec<f64> = threads.iter().filter_map(|i| loads.get(*i).copied().flatten()).collect();
            let n = format!("CPU Core #{}", k + 1);
            cpu.s.push(s("Clock", n.clone(), mhz));
            if !ls.is_empty() {
                cpu.s.push(s("Load", n, Some(ls.iter().sum::<f64>() / ls.len() as f64)));
            }
        }
        // RAPL: package, and its "core" / "dram" sub-zones
        if let Ok(rd) = fs::read_dir("/sys/class/powercap") {
            let mut zones: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
            zones.sort();
            for z in zones {
                let fname = z.file_name().unwrap_or_default().to_string_lossy().to_string();
                if !fname.starts_with("intel-rapl:0") {
                    continue;
                }
                let zname = read(&z.join("name")).unwrap_or_default();
                let label = match zname.as_str() {
                    n if n.starts_with("package") => "CPU Package",
                    "core" => "CPU Cores",
                    "dram" => "CPU Memory",
                    _ => continue,
                };
                if let Some(w) = self.rapl(&z, &fname) {
                    cpu.s.push(s("Power", label, Some(w)));
                }
            }
        }

        let mut board: Option<Hw> = None;
        for chip in chips() {
            let name = chip.name.as_str();
            let label = |kind: &str, idx: u32, l: &Option<String>| -> String {
                l.clone().unwrap_or_else(|| match kind {
                    "temp" => format!("Temperature #{idx}"),
                    "fan" => format!("Fan #{idx}"),
                    "in" => format!("Voltage #{idx}"),
                    _ => format!("{kind}{idx}"),
                })
            };
            match name {
                "coretemp" | "k10temp" | "zenpower" => {
                    for ((kind, idx), (l, v)) in &chip.inputs {
                        if kind != "temp" {
                            continue;
                        }
                        let l = l.clone().unwrap_or_default();
                        let n = if l.starts_with("Package id") {
                            "CPU Package".to_string()
                        } else if let Some(c) = l.strip_prefix("Core ").and_then(|c| c.trim().parse::<u32>().ok()) {
                            match pos(c) {
                                Some(k) => format!("CPU Core #{}", k + 1),
                                None => continue,
                            }
                        } else if l == "Tctl" || l == "Tdie" || (l.is_empty() && *idx == 1) {
                            "Core (Tctl/Tdie)".to_string()
                        } else {
                            l
                        };
                        cpu.s.push(s("Temperature", n, Some(*v)));
                    }
                }
                "amdgpu" => {
                    let dev = chip.dir.join("device");
                    let mut g = Hw { t: "GpuAmd".into(), n: "AMD Radeon".into(), id: "/gpu-amd/0".into(), ..Default::default() };
                    for ((kind, idx), (l, v)) in &chip.inputs {
                        let l = l.clone().unwrap_or_default();
                        match (kind.as_str(), l.as_str()) {
                            ("temp", "edge") => g.s.push(s("Temperature", "GPU Core", Some(*v))),
                            ("temp", "junction") => g.s.push(s("Temperature", "GPU Hot Spot", Some(*v))),
                            ("temp", "mem") => g.s.push(s("Temperature", "GPU Memory Junction", Some(*v))),
                            ("power", _) => g.s.push(s("Power", "GPU Package", Some(*v))),
                            ("fan", _) => g.s.push(s("Fan", "GPU", Some(*v))),
                            ("pwm", _) if *idx == 1 => g.s.push(s("Control", "GPU Fan", Some(*v))),
                            ("in", "vddgfx") => g.s.push(s("Voltage", "GPU Core", Some(*v))),
                            ("freq", "sclk") => g.s.push(s("Clock", "GPU Core", Some(*v))),
                            ("freq", "mclk") => g.s.push(s("Clock", "GPU Memory", Some(*v))),
                            _ => {}
                        }
                    }
                    g.s.push(s("Load", "GPU Core", num(&dev.join("gpu_busy_percent"))));
                    g.s.push(s("SmallData", "GPU Memory Used", num(&dev.join("mem_info_vram_used")).map(|b| b / 1048576.0)));
                    g.s.push(s("SmallData", "GPU Memory Total", num(&dev.join("mem_info_vram_total")).map(|b| b / 1048576.0)));
                    hw.push(g);
                }
                "nvme" | "drivetemp" => {
                    let dev = chip.dir.join("device");
                    let model = read(&dev.join("model")).unwrap_or_else(|| name.to_string());
                    let t = chip.inputs.iter().find(|((k, _), (l, _))| k == "temp" && l.as_deref().is_none_or(|l| l == "Composite")).map(|(_, (_, v))| *v);
                    hw.push(Hw { t: "Storage".into(), n: model, id: format!("/storage/{}", chip.dir.display()), s: vec![s("Temperature", "Temperature", t)], ..Default::default() });
                }
                _ if SUPERIO.iter().any(|p| name.starts_with(p)) || chip.inputs.keys().any(|(k, _)| k == "fan") => {
                    let b = board.get_or_insert_with(|| Hw { t: "SuperIO".into(), n: name.to_uppercase(), id: "/lpc/0".into(), ..Default::default() });
                    for ((kind, idx), (l, v)) in &chip.inputs {
                        let n = label(kind, *idx, l);
                        match kind.as_str() {
                            "temp" => b.s.push(s("Temperature", n, Some(*v))),
                            "fan" => b.s.push(s("Fan", n, Some(*v))),
                            "in" => b.s.push(s("Voltage", if *idx == 0 && l.is_none() { "Vcore".into() } else { n }, Some(*v))),
                            // fan control: same label as its fan so map() pairs them
                            "pwm" => b.s.push(s("Control", label("fan", *idx, &None), Some(*v))),
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        hw.insert(0, cpu);
        if let Some(b) = board {
            hw.push(b);
        }
        Sample { pawnio: true, hw }
    }
}
