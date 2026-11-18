//! PC identification on Linux: DMI from /sys/class/dmi/id, CPU from /proc/cpuinfo
//! and sysfs, GPUs from /sys/bus/pci (names via lspci when it is installed),
//! disks from /sys/block, RAM modules from `dmidecode` (only works as root;
//! without it the module list stays empty and the RAM panel shows totals only).

use crate::pcinfo::{DiskModel, PcInfo, RamModule};
use std::fs;
use std::path::Path;
use std::process::Command;

fn read(p: impl AsRef<Path>) -> String {
    fs::read_to_string(p).map(|s| s.trim().to_string()).unwrap_or_default()
}

fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let o = Command::new(cmd).args(args).output().ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).to_string())
}

fn os_name() -> (String, String) {
    let txt = read("/etc/os-release");
    let get = |k: &str| {
        txt.lines()
            .find_map(|l| l.strip_prefix(&format!("{k}=")))
            .map(|v| v.trim_matches('"').to_string())
            .unwrap_or_default()
    };
    let name = if get("PRETTY_NAME").is_empty() { "Linux".to_string() } else { get("PRETTY_NAME") };
    (name, format!("kernel {}", read("/proc/sys/kernel/osrelease")))
}

fn cpu() -> (String, u32, u32, u32, u32) {
    let info = read("/proc/cpuinfo");
    let field = |k: &str| info.lines().find(|l| l.starts_with(k)).and_then(|l| l.split(':').nth(1)).map(|s| s.trim().to_string()).unwrap_or_default();
    let name = field("model name");
    let threads = info.lines().filter(|l| l.starts_with("processor")).count() as u32;
    // physical cores = distinct (physical id, core id)
    let mut cores = std::collections::BTreeSet::new();
    let (mut pid, mut cid) = (String::new(), String::new());
    for l in info.lines() {
        if let Some(v) = l.strip_prefix("physical id").and_then(|r| r.split(':').nth(1)) {
            pid = v.trim().into();
        }
        if let Some(v) = l.strip_prefix("core id").and_then(|r| r.split(':').nth(1)) {
            cid = v.trim().into();
        }
        if l.trim().is_empty() && !cid.is_empty() {
            cores.insert((pid.clone(), cid.clone()));
            cid.clear();
        }
    }
    if !cid.is_empty() {
        cores.insert((pid, cid));
    }
    let max_mhz = (read("/sys/devices/system/cpu/cpu0/cpufreq/cpuinfo_max_freq").parse::<f64>().unwrap_or(0.0) / 1000.0) as u32;
    let l3 = read("/sys/devices/system/cpu/cpu0/cache/index3/size");
    let l3_kb = l3.trim_end_matches('K').parse::<u32>().unwrap_or(0);
    (name, (cores.len() as u32).max(1), threads.max(1), max_mhz, l3_kb)
}

fn gpus() -> (Vec<String>, String) {
    // lspci gives marketing names; fall back to the vendor from sysfs.
    let mut names: Vec<String> = run("lspci", &["-mm"])
        .map(|o| {
            o.lines()
                .filter(|l| l.contains("\"VGA compatible controller\"") || l.contains("\"3D controller\"") || l.contains("\"Display controller\""))
                .filter_map(|l| {
                    let q: Vec<&str> = l.split('"').collect();
                    // slot "class" "vendor" "device" ...
                    let vendor = q.get(3)?.replace(" Corporation", "").replace(", Inc.", "").replace(" [AMD/ATI]", "");
                    let device = q.get(5)?.to_string();
                    // "TU116 [GeForce GTX 1660 SUPER]" -> "GeForce GTX 1660 SUPER"
                    let dev = device.split_once('[').and_then(|(_, r)| r.split_once(']')).map(|(m, _)| m.to_string()).unwrap_or(device);
                    Some(format!("{vendor} {dev}"))
                })
                .collect()
        })
        .unwrap_or_default();
    if names.is_empty() {
        if let Ok(rd) = fs::read_dir("/sys/bus/pci/devices") {
            for e in rd.flatten() {
                if !read(e.path().join("class")).starts_with("0x03") {
                    continue;
                }
                names.push(match read(e.path().join("vendor")).as_str() {
                    "0x10de" => "NVIDIA GPU".into(),
                    "0x1002" => "AMD Radeon".into(),
                    "0x8086" => "Intel Graphics".into(),
                    v => format!("GPU {v}"),
                });
            }
        }
    }
    let driver = read("/sys/module/nvidia/version");
    let driver = if driver.is_empty() { read("/sys/module/amdgpu/version") } else { driver };
    (names, driver)
}

fn disks() -> Vec<DiskModel> {
    let mut out = Vec::new();
    let Ok(rd) = fs::read_dir("/sys/block") else { return out };
    for e in rd.flatten() {
        let n = e.file_name().to_string_lossy().to_string();
        if ["loop", "ram", "zram", "dm-", "md", "sr", "fd"].iter().any(|p| n.starts_with(p)) {
            continue;
        }
        let p = e.path();
        let sectors: f64 = read(p.join("size")).parse().unwrap_or(0.0);
        if sectors <= 0.0 {
            continue;
        }
        let rot = read(p.join("queue/rotational")) == "1";
        let link = fs::read_link(&p).map(|l| l.to_string_lossy().to_string()).unwrap_or_default();
        let bus = if n.starts_with("nvme") { "NVMe" } else if link.contains("/usb") { "USB" } else { "SATA" };
        out.push(DiskModel {
            model: read(p.join("device/model")).trim().to_string().replace('_', " "),
            size_gb: (sectors * 512.0 / 1e9).round(),
            media: if n.starts_with("nvme") || !rot { "SSD".into() } else { "HDD".into() },
            bus: bus.into(),
        });
    }
    out
}

/// `dmidecode -t 17` output -> modules (needs root; empty otherwise).
fn ram() -> (Vec<RamModule>, u32, f64) {
    let Some(o) = run("dmidecode", &["-t", "16,17"]) else { return (Vec::new(), 0, 0.0) };
    let mut mods = Vec::new();
    let (mut slots, mut max_gb) = (0u32, 0.0);
    for block in o.split("\n\n") {
        let get = |k: &str| block.lines().find_map(|l| l.trim().strip_prefix(&format!("{k}: "))).unwrap_or("").trim().to_string();
        if block.contains("Physical Memory Array") {
            slots += get("Number Of Devices").parse::<u32>().unwrap_or(0);
            let mc = get("Maximum Capacity");
            max_gb += mc.trim_end_matches(" GB").parse::<f64>().unwrap_or(0.0) + mc.trim_end_matches(" TB").parse::<f64>().unwrap_or(0.0) * 1024.0;
            continue;
        }
        if !block.contains("Memory Device") {
            continue;
        }
        let size = get("Size");
        let size_gb = if let Some(g) = size.strip_suffix(" GB") {
            g.parse().unwrap_or(0.0)
        } else if let Some(m) = size.strip_suffix(" MB") {
            m.parse::<f64>().unwrap_or(0.0) / 1024.0
        } else {
            continue; // "No Module Installed"
        };
        let mts = |k: &str| get(k).split_whitespace().next().and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
        let kind = match get("Type").as_str() {
            "DDR5" => 34,
            "DDR4" => 26,
            "DDR3" => 24,
            "LPDDR4" => 30,
            "LPDDR5" => 35,
            _ => 0,
        };
        let mv = get("Configured Voltage").split_whitespace().next().and_then(|v| v.parse::<f64>().ok()).map_or(0, |v| (v * 1000.0) as u32);
        mods.push(RamModule {
            slot: get("Locator"),
            size_gb,
            speed: mts("Speed"),
            configured: mts("Configured Memory Speed"),
            mv,
            kind,
            bank: get("Bank Locator"),
            maker: get("Manufacturer"),
            part: get("Part Number"),
        });
    }
    (mods, slots, max_gb)
}

fn usb_names() -> Vec<String> {
    let mut v = Vec::new();
    if let Ok(rd) = fs::read_dir("/sys/bus/usb/devices") {
        for e in rd.flatten() {
            let p = read(e.path().join("product"));
            if !p.is_empty() {
                v.push(p);
            }
        }
    }
    v
}

pub fn collect() -> PcInfo {
    let dmi = |k: &str| read(format!("/sys/class/dmi/id/{k}"));
    let (os, os_build) = os_name();
    let (cpu, cpu_cores, cpu_threads, cpu_max_mhz, cpu_l3_kb) = cpu();
    let (gpus, gpu_driver) = gpus();
    let (ram, ram_slots, ram_max_gb) = ram();
    let usb = usb_names();
    let pick = |re: &str| {
        let r = regex::Regex::new(re).unwrap();
        usb.iter().filter(|n| r.is_match(n)).cloned().collect::<Vec<_>>()
    };
    PcInfo {
        maker: dmi("sys_vendor"),
        model: dmi("product_name"),
        board_maker: dmi("board_vendor"),
        board: dmi("board_name"),
        board_version: dmi("board_version"),
        ram_slots,
        ram_max_gb,
        rgb_devices: pick(r"(?i)RGB|Aura|Mystic Light|Polychrome|LED Controller|Lighting|Chroma|Spectrix|Fusion|iCUE"),
        bios: format!("{} {}", dmi("bios_vendor"), dmi("bios_version")).trim().to_string(),
        os,
        os_build,
        cpu,
        cpu_cores,
        cpu_threads,
        cpu_max_mhz,
        cpu_l3_kb,
        cpu_socket: String::new(),
        gpus,
        gpu_driver,
        ram,
        disks: disks(),
        cooling_devices: pick(r"(?i)Kraken|NZXT|Corsair|iCUE|Hydro|Commander|Capellix|Lian Li|Asetek|AIO|Liquid|Deepcool|Cooler Master|Aqua|Arctic|EKWB|Thermaltake|Pump"),
        error: None,
    }
}
