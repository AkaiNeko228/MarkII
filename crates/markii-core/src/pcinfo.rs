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
    /// Speed the module is actually running at (MT/s).
    pub configured: u32,
    /// Configured voltage in millivolts (0 = not reported).
    pub mv: u32,
    /// SMBIOS memory type code (26 = DDR4, 34 = DDR5, 24 = DDR3).
    pub kind: u32,
    pub bank: String,
    pub maker: String,
    pub part: String,
}

impl RamModule {
    pub fn type_name(&self) -> &'static str {
        match self.kind {
            34 => "DDR5",
            26 => "DDR4",
            24 => "DDR3",
            30 => "LPDDR4",
            35 => "LPDDR5",
            _ => "RAM",
        }
    }
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
    pub board_version: String,
    /// Memory slots the board reports in SMBIOS.
    pub ram_slots: u32,
    pub ram_max_gb: f64,
    /// RGB / lighting controllers found among PnP devices.
    pub rgb_devices: Vec<String>,
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

#[cfg(not(target_os = "linux"))]
const SCRIPT: &str = r#"
$ErrorActionPreference='SilentlyContinue'
$cs=Get-CimInstance Win32_ComputerSystem; $bb=Get-CimInstance Win32_BaseBoard; $bi=Get-CimInstance Win32_BIOS
$os=Get-CimInstance Win32_OperatingSystem; $p=Get-CimInstance Win32_Processor | Select-Object -First 1
$gpu=Get-CimInstance Win32_VideoController | Where-Object { $_.Name -notmatch 'Virtual|Parsec|Remote|Basic Display|Meta|Mirage' }
$ram=Get-CimInstance Win32_PhysicalMemory | ForEach-Object { [pscustomobject]@{slot=[string]$_.DeviceLocator; size_gb=[math]::Round($_.Capacity/1GB,1); speed=[int]$_.Speed; configured=[int]$_.ConfiguredClockSpeed; mv=[int]$_.ConfiguredVoltage; kind=[int]$_.SMBIOSMemoryType; bank=[string]$_.BankLabel; maker=([string]$_.Manufacturer).Trim(); part=([string]$_.PartNumber).Trim()} }
$arr=@(Get-CimInstance Win32_PhysicalMemoryArray | Where-Object { $_.Use -eq 3 }); $slots=($arr | Measure-Object MemoryDevices -Sum).Sum; $maxkb=($arr | Measure-Object MaxCapacityEx -Sum).Sum
$rgb=Get-CimInstance Win32_PnPEntity | Where-Object { $_.Name -match 'RGB|Aura|Mystic Light|Polychrome|LED Controller|Lighting|Chroma|ENE |Spectrix|Fusion|iCUE' -and $_.Name -notmatch 'Audio|Display|Monitor' } | Select-Object -ExpandProperty Name -Unique
$disks=Get-PhysicalDisk | ForEach-Object { [pscustomobject]@{model=([string]$_.FriendlyName).Trim(); size_gb=[math]::Round($_.Size/1GB); media=[string]$_.MediaType; bus=[string]$_.BusType} }
$cool=Get-CimInstance Win32_PnPEntity | Where-Object { ($_.Name -match 'Kraken|NZXT|Corsair|iCUE|Hydro|Commander|Capellix|Galahad|Lian Li|Asetek|AIO|Liquid|Deepcool|Cooler Master|Aqua|Arctic|EK|Thermaltake|Floe|Pump|Water|Ryujin|Ryuo|ROG|TUF|AORUS|Glacier|Thermalright|Frozen|Aqua Elite|Vision' -and $_.PNPClass -notin @('AudioEndpoint','MEDIA','Net','Keyboard','Mouse','Display','Monitor','SoftwareDevice','Image','Camera','Bluetooth','Printer')) -or $_.DeviceID -match 'VID_87AD&PID_70DB|VID_0416&PID_5302' } | ForEach-Object { if ($_.DeviceID -match 'VID_87AD&PID_70DB|VID_0416&PID_5302') { 'Thermalright water cooler LCD (USB)' } else { $_.Name } } | Select-Object -Unique
[pscustomobject]@{
 maker=[string]$cs.Manufacturer; model=[string]$cs.Model; board_maker=[string]$bb.Manufacturer; board=[string]$bb.Product; board_version=[string]$bb.Version; ram_slots=[int]$slots; ram_max_gb=[math]::Round($maxkb/1MB); rgb_devices=@($rgb)
 bios=("{0} {1}" -f $bi.Manufacturer,$bi.SMBIOSBIOSVersion).Trim(); os=[string]$os.Caption; os_build=("{0} build {1}" -f $os.Version,$os.BuildNumber)
 cpu=([string]$p.Name).Trim(); cpu_cores=[int]$p.NumberOfCores; cpu_threads=[int]$p.NumberOfLogicalProcessors; cpu_max_mhz=[int]$p.MaxClockSpeed
 cpu_l3_kb=[int]$p.L3CacheSize; cpu_socket=[string]$p.SocketDesignation
 gpus=@($gpu | ForEach-Object { [string]$_.Name }); gpu_driver=[string]($gpu | Select-Object -First 1).DriverVersion
 ram=@($ram); disks=@($disks); cooling_devices=@($cool)
} | ConvertTo-Json -Depth 4 -Compress
"#;

#[cfg(target_os = "linux")]
pub fn collect() -> PcInfo {
    crate::pcinfo_linux::collect()
}

#[cfg(not(target_os = "linux"))]
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
            let mut info = serde_json::from_str::<PcInfo>(s.trim()).unwrap_or_else(|e| PcInfo {
                error: Some(format!("não foi possível ler as informações do sistema: {e}")),
                ..Default::default()
            });
            info.cooling_devices.retain(|d| is_cooler_device(d));
            info
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

/// USB devices that really are liquid-cooling controllers. Brand names alone are not
/// enough (Corsair / Cooler Master / Thermaltake also make mice, keyboards, headsets and
/// air coolers; "AIO" hides inside "VoiceMeeter VAIO" and Sony VAIO; "Aqua" inside the
/// Aquantia network chip), so match product lines and whole words.
pub fn is_cooler_device(name: &str) -> bool {
    static R: OnceLock<Regex> = OnceLock::new();
    let re = R.get_or_init(|| {
        Regex::new(concat!(
            r"(?i)\bkraken\b|\bhydro (series|x)\b|\bcapellix\b|\bcommander core\b|\bicue link\b|\bh\d{2,3}[ix]\b|",
            r"\brog (ryujin|ryuo|strix lc)|\btuf gaming lc\b|\baorus (waterforce|liquid)|\bwaterforce\b|\bglacier one\b|",
            r"\bgalahad\b|\basetek\b|\bcoreliquid\b|\bmaster ?liquid\b|\bliquid ?freezer\b|\bliquid ?cool|\bwater ?cool|",
            r"\baio\b|\bpump\b|\bek-?(aio|nucleus)\b|\baquacomputer\b|\baquaero\b|\bd5 next\b|\btoughliquid\b|\bfloe\b|",
            r"\bdeepcool (ls|lt|le|castle|mystique|captain)\d*|",
            r"\bfrozen (notte|magic|prism|warframe|edge|horizon|vision|infinity|guardian)\b|\baqua elite\b|",
            r"\bthermalright\b.*\b(water ?cooler|lcd|aio)\b|\b(grand|core) vision\b"
        ))
        .unwrap()
    });
    re.is_match(name)
}

/// Fan headers that carry a pump. "OPT" (CPU_OPT) is often a second air-cooler fan, so
/// it counts only at pump speeds (see `detect`).
fn pump_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?i)pump|\baio\b|w_pump").unwrap())
}
/// Headers the board does not name ("Fan #3", "fan2"): LibreHardwareMonitor only knows
/// the names on some boards.
fn generic_fan_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?i)^fan\s*#?\s*\d+$").unwrap())
}

/// Tells pumps from fans on unnamed headers by watching them over time. Speed alone is
/// not enough: an air cooler's fan also reaches 1800+ rpm when the CPU is hot. A pump
/// runs flat out all the time, so a header counts as one only after it was seen at pump
/// speed with the CPU cool (fans slow down then) and never seen slow.
#[derive(Default, Debug)]
pub struct PumpWatch {
    /// label -> (seconds at pump speed with the CPU cool, ever seen slow)
    seen: std::collections::HashMap<String, (u32, bool)>,
}

impl PumpWatch {
    const PUMP_RPM: f64 = 1750.0;
    const SLOW_RPM: f64 = 1500.0;
    const COOL_C: f64 = 55.0;
    const NEEDED: u32 = 20;
    /// A pump on the CPU_FAN header (Thermalright and others tell you to plug it
    /// there): air-cooler fans rarely pass ~2000 rpm, and never with the CPU cool.
    const CPU_HEADER_PUMP_RPM: f64 = 2600.0;

    pub fn update(&mut self, fans: &[Fan], cpu_temp: Option<f64>) {
        for f in fans.iter().filter(|f| generic_fan_re().is_match(&f.label) || cpu_fan_re().is_match(&f.label)) {
            let pump_rpm = if cpu_fan_re().is_match(&f.label) { Self::CPU_HEADER_PUMP_RPM } else { Self::PUMP_RPM };
            let e = self.seen.entry(f.label.clone()).or_default();
            if f.rpm > 0.0 && f.rpm < Self::SLOW_RPM {
                e.1 = true;
            } else if f.rpm >= pump_rpm && f.rpm <= 5500.0 && cpu_temp.is_some_and(|t| t <= Self::COOL_C) {
                e.0 = e.0.saturating_add(1);
            }
        }
    }

    /// Headers that behave like a pump so far.
    pub fn pumps(&self) -> Vec<String> {
        self.seen.iter().filter(|(_, (n, slow))| *n >= Self::NEEDED && !*slow).map(|(l, _)| l.clone()).collect()
    }
}

fn opt_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?i)\bopt\b|_opt\b").unwrap())
}
fn cpu_fan_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?i)^cpu( fan)?$|cpu_fan|cpu fan").unwrap())
}

/// `steady_pumps`: unnamed headers that `PumpWatch` saw behaving like a pump.
pub fn detect(mode: &str, info: Option<&PcInfo>, fans: &[Fan], hwinfo_ok: bool, steady_pumps: &[String]) -> Cooling {
    // a pump runs fast (2000-4800 rpm); a CPU_OPT header at fan speeds is a fan
    let pump = fans
        .iter()
        .find(|f| pump_re().is_match(&f.label) && f.rpm > 300.0)
        .or_else(|| fans.iter().find(|f| opt_re().is_match(&f.label) && f.rpm >= 2200.0));
    // Unnamed headers: only one that kept pump speed (~1800+ rpm; Cooler Master's are
    // 2000 ±10%) with the CPU cool, see `PumpWatch`
    let unnamed_pump = fans.iter().filter(|f| steady_pumps.contains(&f.label) && f.rpm > 300.0).max_by(|a, b| a.rpm.total_cmp(&b.rpm));
    // Named "CPU fan" header, or — on boards whose headers are unnamed — the only one spinning.
    let spinning: Vec<&Fan> = fans.iter().filter(|f| f.rpm > 0.0).collect();
    let cpu_fan = fans
        .iter()
        .find(|f| cpu_fan_re().is_match(&f.label) && f.rpm > 0.0)
        .or_else(|| (spinning.len() == 1).then(|| spinning[0]))
        // the CPU_FAN header carries the pump: the radiator fans are elsewhere
        .filter(|f| unnamed_pump.map_or(true, |p| p.label != f.label));
    let mut c = Cooling {
        pump_rpm: pump.or(unnamed_pump).map(|f| f.rpm),
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
            } else if let Some(p) = unnamed_pump {
                if cpu_fan_re().is_match(&p.label) {
                    ("water", "medium", format!("“{}” fica a {:.0} rpm mesmo com a CPU fria: é a bomba ligada no conector da CPU (como na Thermalright)", p.label, p.rpm))
                } else {
                    ("water", "low", format!("“{}” fica a {:.0} rpm mesmo com a CPU fria, como uma bomba (a placa não diz o nome dos conectores)", p.label, p.rpm))
                }
            } else if let Some(f) = cpu_fan {
                ("air", "medium", format!("ventoinha da CPU em “{}” ({:.0} rpm) e nenhuma bomba", f.label, f.rpm))
            } else if fans.iter().any(|f| f.rpm > 0.0) {
                let n = fans.iter().filter(|f| f.rpm > 0.0).count();
                ("air", "low", format!("{n} ventoinha(s) girando na placa e nenhuma bomba ou controlador de water cooler"))
            } else if hwinfo_ok {
                ("air", "low", "nenhuma bomba nos sensores".to_string())
            } else {
                ("unknown", "low", "ative os sensores avançados ou escolha nas configurações".to_string())
            }
        }
    };
    c.kind = kind.into();
    c.confidence = conf.into();
    c.reason = reason;
    c
}

#[cfg(test)]
mod cooler_tests {
    use super::*;

    #[test]
    fn cooler_devices_without_false_positives() {
        for yes in ["NZXT Kraken X63", "Corsair iCUE H150i ELITE CAPELLIX", "iCUE COMMANDER CORE", "ASUS AIO Pump", "Lian Li Galahad AIO", "ARCTIC Liquid Freezer II", "DeepCool LS720", "MSI MAG CoreLiquid 240R"] {
            assert!(is_cooler_device(yes), "{yes}");
        }
        for no in ["VB-Audio VoiceMeeter VAIO", "VoiceMeeter Aux VAIO", "Corsair K70 RGB Keyboard", "Corsair HS80 Headset", "Aquantia AQtion 10Gbit Network Adapter", "ARCTIC P12 PWM", "Cooler Master MM711", "Thermaltake Level 20 Mouse", "Sony VAIO"] {
            assert!(!is_cooler_device(no), "{no}");
        }
    }

    #[test]
    fn opt_header_is_a_pump_only_at_pump_speed() {
        let fan = |l: &str, rpm: f64| Fan { label: l.into(), rpm, ..Default::default() };
        let c = detect("auto", None, &[fan("CPU Fan", 1200.0), fan("CPU_OPT", 1100.0)], true, &[]);
        assert_eq!(c.kind, "air");
        let c = detect("auto", None, &[fan("CPU Fan", 1200.0), fan("CPU_OPT", 2900.0)], true, &[]);
        assert_eq!(c.kind, "water");
    }

    #[test]
    fn unnamed_header_at_pump_speed_is_a_pump() {
        let fan = |l: &str, rpm: f64| Fan { label: l.into(), rpm, ..Default::default() };
        // water: Fan #2 stays at pump speed while the CPU idles cool
        let water = [fan("Fan #1", 700.0), fan("Fan #2", 1840.0), fan("Fan #3", 1100.0)];
        let mut w = PumpWatch::default();
        for _ in 0..25 {
            w.update(&water, Some(44.0));
        }
        let c = detect("auto", None, &water, true, &w.pumps());
        assert_eq!((c.kind.as_str(), c.pump_rpm), ("water", Some(1840.0)));
        // air: Fan #2 reaches 1781 rpm only with the CPU hot, and slows down when it cools
        let mut a = PumpWatch::default();
        for _ in 0..60 {
            a.update(&[fan("Fan #1", 900.0), fan("Fan #2", 1781.0)], Some(72.0));
        }
        assert!(a.pumps().is_empty());
        let hot = [fan("Fan #1", 900.0), fan("Fan #2", 1781.0)];
        assert_eq!(detect("auto", None, &hot, true, &a.pumps()).kind, "air");
        for _ in 0..30 {
            a.update(&[fan("Fan #1", 700.0), fan("Fan #2", 1760.0)], Some(54.0));
        }
        a.update(&[fan("Fan #1", 600.0), fan("Fan #2", 900.0)], Some(45.0));
        assert!(a.pumps().is_empty(), "a fan that slows down is not a pump");
        for yes in ["Cooler Master Master Liquid ML240L", "MasterLiquid PL360 Flux", "ROG RYUJIN III 360", "ROG STRIX LC II 240", "AORUS WATERFORCE X 360", "Corsair iCUE LINK System Hub", "Corsair H100x RGB ELITE"] {
            assert!(is_cooler_device(yes), "{yes}");
        }
        for no in ["ROG STRIX B550-F GAMING", "TUF GAMING X570-PLUS", "AORUS ELITE", "NVIDIA H100"] {
            assert!(!is_cooler_device(no), "{no}");
        }
    }

    #[test]
    fn thermalright() {
        for yes in ["Thermalright Frozen Notte 360 ARGB", "Frozen Warframe 240", "Thermalright Aqua Elite 360 V3", "Frozen Prism 240", "Thermalright water cooler LCD (USB)", "Grand Vision 360"] {
            assert!(is_cooler_device(yes), "{yes}");
        }
        // air coolers and the rest of the brand
        for no in ["Thermalright Peerless Assassin 120 SE", "Thermalright Phantom Spirit 120", "Thermalright TL-C12C", "Frozen II"] {
            assert!(!is_cooler_device(no), "{no}");
        }
        let fan = |l: &str, rpm: f64| Fan { label: l.into(), rpm, ..Default::default() };
        // pump on CPU_FAN at ~3300 rpm with the CPU cool, radiator fans on SYS_FAN
        let water = [fan("CPU Fan", 3320.0), fan("System Fan #1", 900.0)];
        let mut w = PumpWatch::default();
        for _ in 0..25 {
            w.update(&water, Some(42.0));
        }
        let c = detect("auto", None, &water, true, &w.pumps());
        assert_eq!((c.kind.as_str(), c.pump_rpm, c.cpu_fan_rpm), ("water", Some(3320.0), None));
        // air cooler fan on CPU_FAN: fast only when hot, so not a pump
        let mut a = PumpWatch::default();
        for _ in 0..25 {
            a.update(&[fan("CPU Fan", 2700.0)], Some(80.0));
        }
        for _ in 0..25 {
            a.update(&[fan("CPU Fan", 1100.0)], Some(45.0));
        }
        assert!(a.pumps().is_empty());
        assert_eq!(detect("auto", None, &[fan("CPU Fan", 1100.0)], true, &a.pumps()).kind, "air");
    }
}
