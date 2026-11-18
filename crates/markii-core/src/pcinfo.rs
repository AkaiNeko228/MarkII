//! One-time PC identification (WMI via PowerShell) and cooler-type detection.

use crate::hw::Fan;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct RamModule {
    pub slot: String,
    pub size_gb: f64,
    pub speed: u32,
    pub maker: String,
    pub part: String,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct DiskModel {
    pub model: String,
    pub size_gb: f64,
    pub media: String,
    pub bus: String,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct PcInfo {
    pub maker: String,
    pub model: String,
    pub board_maker: String,
    pub board: String,
    pub bios: String,
    pub os: String,
    pub os_build: String,
    pub cpu: String,
    pub cpu_cores: u32,
    pub cpu_threads: u32,
    pub cpu_max_mhz: u32,
    pub cpu_l3_kb: u32,
    pub cpu_socket: String,
    pub gpus: Vec<String>,
    pub gpu_driver: String,
    pub ram: Vec<RamModule>,
    pub disks: Vec<DiskModel>,
    /// USB/PnP devices that look like liquid-cooling controllers.
    pub cooling_devices: Vec<String>,
    pub error: Option<String>,
}

const SCRIPT: &str = r#"
$ErrorActionPreference='SilentlyContinue'
$cs=Get-CimInstance Win32_ComputerSystem; $bb=Get-CimInstance Win32_BaseBoard; $bi=Get-CimInstance Win32_BIOS
$os=Get-CimInstance Win32_OperatingSystem; $p=Get-CimInstance Win32_Processor | Select-Object -First 1
$gpu=Get-CimInstance Win32_VideoController | Where-Object { $_.Name -notmatch 'Virtual|Parsec|Remote|Basic Display|Meta|Mirage' }
$ram=Get-CimInstance Win32_PhysicalMemory | ForEach-Object { [pscustomobject]@{slot=[string]$_.DeviceLocator; size_gb=[math]::Round($_.Capacity/1GB,1); speed=[int]$_.Speed; maker=([string]$_.Manufacturer).Trim(); part=([string]$_.PartNumber).Trim()} }
$disks=Get-PhysicalDisk | ForEach-Object { [pscustomobject]@{model=([string]$_.FriendlyName).Trim(); size_gb=[math]::Round($_.Size/1GB); media=[string]$_.MediaType; bus=[string]$_.BusType} }
$cool=Get-CimInstance Win32_PnPEntity | Where-Object { $_.Name -match 'Kraken|NZXT|Corsair|iCUE|Hydro|Commander|Capellix|Galahad|Lian Li|Asetek|AIO|Liquid|Deepcool|CoreLiquid|Cooler Master|Aqua|Arctic|EKWB|Thermaltake|Riing|Pump' } | Select-Object -ExpandProperty Name -Unique
[pscustomobject]@{
 maker=[string]$cs.Manufacturer; model=[string]$cs.Model; board_maker=[string]$bb.Manufacturer; board=[string]$bb.Product
 bios=("{0} {1}" -f $bi.Manufacturer,$bi.SMBIOSBIOSVersion).Trim(); os=[string]$os.Caption; os_build=("{0} build {1}" -f $os.Version,$os.BuildNumber)
 cpu=([string]$p.Name).Trim(); cpu_cores=[int]$p.NumberOfCores; cpu_threads=[int]$p.NumberOfLogicalProcessors; cpu_max_mhz=[int]$p.MaxClockSpeed
 cpu_l3_kb=[int]$p.L3CacheSize; cpu_socket=[string]$p.SocketDesignation
 gpus=@($gpu | ForEach-Object { [string]$_.Name }); gpu_driver=[string]($gpu | Select-Object -First 1).DriverVersion
 ram=@($ram); disks=@($disks); cooling_devices=@($cool)
} | ConvertTo-Json -Depth 4 -Compress
"#;

pub fn collect() -> PcInfo {
    let mut cmd = std::process::Command::new("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", SCRIPT]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    match cmd.output() {
        Ok(o) => {
            let s = String::from_utf8_lossy(&o.stdout);
            serde_json::from_str::<PcInfo>(s.trim()).unwrap_or_else(|e| PcInfo {
                error: Some(format!("não foi possível ler as informações do sistema: {e}")),
                ..Default::default()
            })
        }
        Err(e) => PcInfo { error: Some(e.to_string()), ..Default::default() },
    }
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct Cooling {
    /// "water" | "air" | "unknown"
    pub kind: String,
    /// "manual" | "high" | "medium" | "low"
    pub confidence: String,
    pub reason: String,
    pub pump_rpm: Option<f64>,
    pub cpu_fan_rpm: Option<f64>,
    pub fans: Vec<Fan>,
}

fn pump_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?i)pump|aio|w_pump|opt").unwrap())
}
fn cpu_fan_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?i)^cpu( fan)?$|cpu_fan|cpu fan").unwrap())
}

pub fn detect(mode: &str, info: Option<&PcInfo>, fans: &[Fan], hwinfo_ok: bool) -> Cooling {
    let pump = fans.iter().find(|f| pump_re().is_match(&f.label) && f.rpm > 300.0);
    let cpu_fan = fans.iter().find(|f| cpu_fan_re().is_match(&f.label) && f.rpm > 0.0);
    let mut c = Cooling {
        pump_rpm: pump.map(|f| f.rpm),
        cpu_fan_rpm: cpu_fan.map(|f| f.rpm),
        fans: fans.to_vec(),
        ..Default::default()
    };
    let devices = info.map(|i| i.cooling_devices.clone()).unwrap_or_default();

    let (kind, conf, reason) = match mode {
        "water" => ("water", "manual", "definido nas configurações".to_string()),
        "air" => ("air", "manual", "definido nas configurações".to_string()),
        _ => {
            if let Some(p) = pump {
                ("water", "high", format!("bomba detectada em “{}” ({:.0} rpm)", p.label, p.rpm))
            } else if let Some(d) = devices.first() {
                ("water", "medium", format!("controlador de water cooler encontrado: {d}"))
            } else if let Some(f) = cpu_fan {
                ("air", "medium", format!("ventoinha da CPU em “{}” ({:.0} rpm) e nenhuma bomba", f.label, f.rpm))
            } else if hwinfo_ok {
                ("air", "low", "nenhuma bomba nos sensores".to_string())
            } else {
                ("unknown", "low", "abra o HWiNFO ou escolha nas configurações".to_string())
            }
        }
    };
    c.kind = kind.into();
    c.confidence = conf.into();
    c.reason = reason;
    c
}
