//! LibreHardwareMonitor helper (`markii-sensors.exe`, needs admin + the PawnIO driver).
//!
//! The app (normal user) creates a named pipe and starts the helper elevated with UAC.
//! The helper connects and streams one JSON line per sample. It never reads from the
//! pipe, so the elevated process takes no input from us besides its command line.
//!
//! [`map`] turns a sample into the same [`HwSnapshot`] the HWiNFO reader produces,
//! matching LHM's sensor names for Intel, AMD and NVIDIA/AMD GPUs generically.

use crate::hw::{Board, Core, Cpu, Drive, Fan, Gpu, HwSnapshot, Named, Spd};
use regex::Regex;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

#[derive(Deserialize, Clone, Debug, Default)]
pub struct Sample {
    #[serde(default)]
    pub pawnio: bool,
    #[serde(default)]
    pub hw: Vec<Hw>,
}

#[derive(Deserialize, Clone, Debug, Default)]
pub struct Hw {
    pub t: String,
    pub n: String,
    pub id: String,
    #[serde(default)]
    pub p: Option<String>,
    #[serde(default)]
    pub s: Vec<Sensor>,
}

#[derive(Deserialize, Clone, Debug, Default)]
pub struct Sensor {
    pub t: String,
    pub n: String,
    pub v: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub enum Status {
    #[default]
    Off,
    /// Waiting for the user to answer UAC / helper to connect.
    Starting,
    /// Running the PawnIO installer.
    Installing,
    Running,
    /// User said no to UAC: helper runs without admin (GPU and drives only).
    Limited,
    Failed(String),
    /// Helper went away after running.
    Stopped,
}

impl Status {
    pub fn text(&self) -> String {
        match self {
            Status::Off => "desligado".into(),
            Status::Starting => "iniciando — confirme o pedido de administrador".into(),
            Status::Installing => "instalando o driver PawnIO — confirme o pedido de administrador".into(),
            Status::Running => "ativo".into(),
            Status::Limited => "sem administrador: só GPU e discos (CPU, ventoinhas e placa-mãe precisam do UAC)".into(),
            Status::Failed(e) => format!("falhou: {e}"),
            Status::Stopped => "o serviço de sensores parou".into(),
        }
    }
}

#[derive(Default)]
struct State {
    status: Status,
    last: Option<(Instant, Sample)>,
    /// Bumped on every (re)start; an older reader thread sees the change and quits,
    /// which closes its pipe and makes the old helper exit.
    gen: u64,
}

/// Process-wide handle to the helper.
pub struct Sensors {
    st: Arc<Mutex<State>>,
}

pub fn global() -> &'static Sensors {
    static S: OnceLock<Sensors> = OnceLock::new();
    S.get_or_init(|| Sensors { st: Arc::new(Mutex::new(State::default())) })
}

/// True when the PawnIO driver is installed (needed for CPU/board sensors).
pub fn pawnio_installed() -> bool {
    // Linux reads the sensor chips through the kernel: no extra driver needed.
    if cfg!(not(windows)) {
        return true;
    }
    let pf = std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".into());
    PathBuf::from(pf).join("PawnIO").join("PawnIOLib.dll").exists()
}

pub(crate) fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe().ok()?.parent().map(|p| p.to_path_buf())
}

/// Finds a file shipped next to the app (release zip) or in the dev build tree.
pub(crate) fn find_shipped(name: &str) -> Option<PathBuf> {
    let d = exe_dir()?;
    [
        d.join(name),
        d.join("sensors").join(name),
        d.join("..").join("sensors").join(name),
        // dev tree: target/<profile>/ -> vendor/pawnio/
        d.join("..").join("..").join("vendor").join("pawnio").join(name),
    ]
        .into_iter()
        .find(|p| p.exists())
}

pub fn helper_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("MARKII_SENSORS").map(PathBuf::from).filter(|p| p.exists()) {
        return Some(p);
    }
    find_shipped("markii-sensors.exe")
}

impl Sensors {
    pub fn status(&self) -> Status {
        self.st.lock().unwrap().status.clone()
    }

    /// Latest sample if it is fresh.
    pub fn latest(&self) -> Option<Sample> {
        let st = self.st.lock().unwrap();
        st.last.as_ref().filter(|(t, _)| t.elapsed() < Duration::from_secs(6)).map(|(_, s)| s.clone())
    }

    fn set(&self, gen: u64, s: Status) {
        let mut st = self.st.lock().unwrap();
        if st.gen == gen {
            st.status = s;
        }
    }

    fn current(&self, gen: u64) -> bool {
        self.st.lock().unwrap().gen == gen
    }

    /// Starts the helper unless it is already starting or fully running. Non-blocking.
    /// `admin = false` starts it without UAC (limited sensors). Calling it while the
    /// helper runs in limited mode replaces that helper.
    pub fn start(&'static self, interval_ms: u64, admin: bool) {
        let gen = {
            let mut st = self.st.lock().unwrap();
            if matches!(st.status, Status::Starting | Status::Running | Status::Installing) {
                return;
            }
            if st.status == Status::Limited && !admin {
                return;
            }
            st.status = Status::Starting;
            st.gen += 1;
            st.gen
        };
        std::thread::spawn(move || {
            if let Err(e) = self.run(interval_ms, admin, gen) {
                self.set(gen, e);
            }
        });
    }

    /// Installs PawnIO (UAC), then restarts the helper with admin. Non-blocking.
    pub fn install_then_start(&'static self, interval_ms: u64) {
        let gen = {
            let mut st = self.st.lock().unwrap();
            if st.status == Status::Installing {
                return;
            }
            st.status = Status::Installing;
            st.gen += 1;
            st.gen
        };
        std::thread::spawn(move || match install_pawnio() {
            Ok(()) => {
                self.set(gen, Status::Off);
                self.start(interval_ms, true);
            }
            Err(e) => self.set(gen, Status::Failed(e)),
        });
    }

    /// Linux: read hwmon/sysfs in-process on a timer (no helper, no admin).
    #[cfg(target_os = "linux")]
    fn run(&'static self, interval_ms: u64, _admin: bool, gen: u64) -> Result<(), Status> {
        let mut r = crate::linux::Reader::new();
        while self.current(gen) {
            let sample = r.sample();
            {
                let mut st = self.st.lock().unwrap();
                if st.gen != gen {
                    break;
                }
                st.last = Some((Instant::now(), sample));
                st.status = Status::Running;
            }
            std::thread::sleep(Duration::from_millis(interval_ms.clamp(250, 5000)));
        }
        Ok(())
    }

    #[cfg(not(any(windows, target_os = "linux")))]
    fn run(&'static self, _interval_ms: u64, _admin: bool, _gen: u64) -> Result<(), Status> {
        Err(Status::Failed("sistema não suportado".into()))
    }

    #[cfg(windows)]
    fn run(&'static self, interval_ms: u64, admin: bool, gen: u64) -> Result<(), Status> {
        use std::io::{BufRead, BufReader};
        use std::os::windows::io::FromRawHandle;
        use std::sync::atomic::{AtomicBool, Ordering};
        use windows_sys::Win32::Foundation::*;
        use windows_sys::Win32::Storage::FileSystem::*;
        use windows_sys::Win32::System::Pipes::*;
        use windows_sys::Win32::System::Threading::*;

        let exe = helper_path().ok_or_else(|| Status::Failed("markii-sensors.exe não encontrado ao lado do app".into()))?;
        let id = format!("markii-sensors-{}-{gen}", std::process::id());
        let name = format!(r"\\.\pipe\{id}");
        let wname: Vec<u16> = name.encode_utf16().chain([0]).collect();
        let pipe = unsafe {
            CreateNamedPipeW(
                wname.as_ptr(),
                PIPE_ACCESS_INBOUND | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                0,
                1 << 16,
                0,
                std::ptr::null(),
            )
        };
        if pipe == INVALID_HANDLE_VALUE {
            return Err(Status::Failed(format!("pipe: erro {}", unsafe { GetLastError() })));
        }
        let pipe_ = pipe as isize;

        let args = format!("--pipe {id} --interval {}", interval_ms.clamp(250, 10000));
        // Admin first; if UAC is refused (or not wanted) run it as the user: GPU/drives still work.
        let mut limited = !admin;
        let elevated = if admin { elevate(&exe, &args, false) } else { Err(ERROR_CANCELLED) };
        let proc = match elevated {
            Ok(h) => h,
            Err(code) if code == ERROR_CANCELLED => {
                limited = true;
                match spawn_plain(&exe, &args) {
                    Ok(h) => h,
                    Err(e) => {
                        unsafe { CloseHandle(pipe) };
                        return Err(Status::Failed(e));
                    }
                }
            }
            Err(code) => {
                unsafe { CloseHandle(pipe) };
                return Err(Status::Failed(format!("UAC: erro {code}")));
            }
        };

        // Watchdog: if the helper dies or never connects, poke the pipe so ConnectNamedPipe returns.
        let connected = Arc::new(AtomicBool::new(false));
        let gave_up = Arc::new(AtomicBool::new(false));
        {
            let (connected, gave_up, wname) = (connected.clone(), gave_up.clone(), wname.clone());
            let proc_ = proc as isize;
            std::thread::spawn(move || {
                let t0 = Instant::now();
                while !connected.load(Ordering::SeqCst) {
                    let exited = unsafe { WaitForSingleObject(proc_ as HANDLE, 250) } == WAIT_OBJECT_0;
                    if exited || t0.elapsed() > Duration::from_secs(30) {
                        gave_up.store(true, Ordering::SeqCst);
                        let h = unsafe { CreateFileW(wname.as_ptr(), GENERIC_WRITE, 0, std::ptr::null(), OPEN_EXISTING, 0, std::ptr::null_mut()) };
                        if h != INVALID_HANDLE_VALUE {
                            unsafe { CloseHandle(h) };
                        }
                        break;
                    }
                }
                unsafe { CloseHandle(proc_ as HANDLE) };
            });
        }

        let ok = unsafe { ConnectNamedPipe(pipe_ as HANDLE, std::ptr::null_mut()) } != 0 || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED;
        if !ok || gave_up.load(Ordering::SeqCst) {
            unsafe { CloseHandle(pipe_ as HANDLE) };
            return Err(Status::Failed("o serviço de sensores não respondeu".into()));
        }
        connected.store(true, Ordering::SeqCst);
        self.set(gen, if limited { Status::Limited } else { Status::Running });

        let file = unsafe { std::fs::File::from_raw_handle(pipe_ as _) };
        for line in BufReader::new(file).lines() {
            let Ok(line) = line else { break };
            if !self.current(gen) {
                return Ok(()); // replaced by a newer helper; dropping the pipe stops this one
            }
            if let Ok(s) = serde_json::from_str::<Sample>(&line) {
                self.st.lock().unwrap().last = Some((Instant::now(), s));
            }
        }
        Err(Status::Stopped)
    }
}

/// Runs `exe args` through UAC ("runas"). Returns the process handle or the Win32 error.
#[cfg(windows)]
pub(crate) fn elevate(exe: &std::path::Path, args: &str, wait: bool) -> Result<windows_sys::Win32::Foundation::HANDLE, u32> {
    use windows_sys::Win32::Foundation::GetLastError;
    use windows_sys::Win32::System::Threading::{WaitForSingleObject, INFINITE};
    use windows_sys::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE;
    let w = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let (verb, file, params) = (w("runas"), w(&exe.to_string_lossy()), w(args));
    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_NOCLOSEPROCESS;
    info.lpVerb = verb.as_ptr();
    info.lpFile = file.as_ptr();
    info.lpParameters = params.as_ptr();
    info.nShow = SW_HIDE;
    if unsafe { ShellExecuteExW(&mut info) } == 0 {
        return Err(unsafe { GetLastError() });
    }
    if wait && !info.hProcess.is_null() {
        unsafe { WaitForSingleObject(info.hProcess, INFINITE) };
    }
    Ok(info.hProcess)
}

/// Starts the helper as the current user (no UAC). Returns its process handle.
#[cfg(windows)]
fn spawn_plain(exe: &std::path::Path, args: &str) -> Result<windows_sys::Win32::Foundation::HANDLE, String> {
    use std::os::windows::io::IntoRawHandle;
    use std::os::windows::process::CommandExt;
    let child = std::process::Command::new(exe)
        .args(args.split_whitespace())
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(child.into_raw_handle() as _)
}

/// Installs the PawnIO driver with the official installer shipped next to the app.
/// Blocks until the installer exits. Nothing is downloaded.
pub fn install_pawnio() -> Result<(), String> {
    #[cfg(windows)]
    {
        if let Some(setup) = find_shipped("PawnIO_setup.exe") {
            return match elevate(&setup, "-install -silent", true) {
                Ok(h) => {
                    if !h.is_null() {
                        unsafe { windows_sys::Win32::Foundation::CloseHandle(h) };
                    }
                    if pawnio_installed() { Ok(()) } else { Err("a instalação do PawnIO não terminou".into()) }
                }
                Err(1223) => Err("permissão de administrador recusada".into()),
                Err(e) => Err(format!("erro {e}")),
            };
        }
        Err("PawnIO_setup.exe não está na pasta do Mark II; extraia o zip completo".into())
    }
    #[cfg(not(windows))]
    Err("só no Windows".into())
}

// ---------------------------------------------------------------- mapping

fn core_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    // "CPU Core #3", "P-Core #2", "E-Core #1", "Core #4", "CPU Core #1 Thread #2", "Core #1 VID", "Core #2 (SMU)"
    R.get_or_init(|| Regex::new(r"(?i)^(?:CPU )?(?:([PE])-)?Core #(\d+)(?: Thread #\d+)?(?: (VID|\(.*\)))?$").unwrap())
}

fn val(h: &Hw, kind: &str, names: &[&str]) -> Option<f64> {
    for exact in [true, false] {
        for n in names {
            let n = n.to_lowercase();
            if let Some(v) = h
                .s
                .iter()
                .filter(|s| s.t == kind)
                .find(|s| {
                    let l = s.n.to_lowercase();
                    if exact { l == n } else { l.contains(&n) }
                })
                .and_then(|s| s.v)
            {
                return Some(v);
            }
        }
    }
    None
}

/// Unconnected thermistor inputs on Super I/O chips read ~100-127 °C; drop them.
fn plausible_temp(v: f64) -> bool {
    v > 1.0 && v < 100.0
}

pub fn map(sm: &Sample) -> HwSnapshot {
    fn of<'a>(sm: &'a Sample, t: &'static str) -> impl Iterator<Item = &'a Hw> { sm.hw.iter().filter(move |h| h.t == t) }
    let by_type = |t: &'static str| of(sm, t);
    let mut snap = HwSnapshot::default();

    // ---- CPU ----
    if let Some(h) = by_type("Cpu").next() {
        let mut cores: BTreeMap<(u8, u32), Core> = BTreeMap::new();
        for s in &h.s {
            let Some(cap) = core_re().captures(&s.n) else { continue };
            let kind = cap.get(1).map_or("P", |m| m.as_str()).to_uppercase();
            let n: u32 = cap[2].parse().unwrap_or(1);
            let suffix = cap.get(3).map(|m| m.as_str()).unwrap_or("");
            let key = (if kind == "E" { 1 } else { 0 }, n);
            let e = cores.entry(key).or_insert_with(|| Core { kind: kind.clone(), idx: n.saturating_sub(1), ..Default::default() });
            let Some(v) = s.v else { continue };
            match s.t.as_str() {
                "Temperature" if !s.n.contains("Distance") => {
                    e.temp.get_or_insert(v);
                }
                "Clock" => {
                    e.clock.get_or_insert(v);
                }
                "Voltage" if suffix.is_empty() || suffix.eq_ignore_ascii_case("VID") => {
                    e.vid.get_or_insert(v);
                }
                "Load" => e.load = Some(e.load.map_or(v, |o: f64| o.max(v))),
                _ => {}
            }
        }
        let cores: Vec<Core> = cores.into_values().filter(|c| c.load.is_some() || c.clock.is_some()).collect();
        let core_max = val(h, "Temperature", &["Core Max"]).or_else(|| cores.iter().filter_map(|c| c.temp).reduce(f64::max));
        snap.cpu = Cpu {
            name: h.n.clone(),
            package: val(h, "Temperature", &["CPU Package", "Core (Tctl/Tdie)", "Core (Tctl)", "Package", "Tctl/Tdie", "CPU"]),
            core_max,
            // Without the driver the energy counters read 0 W: treat that as "no reading".
            power: val(h, "Power", &["CPU Package", "Package"]).filter(|v| *v > 0.0),
            ia_power: val(h, "Power", &["CPU Cores", "Cores"]).filter(|v| *v > 0.0),
            sa_power: val(h, "Power", &["CPU System Agent"]).filter(|v| *v > 0.0),
            mem_power: val(h, "Power", &["CPU Memory", "Memory"]).filter(|v| *v > 0.0),
            vid: val(h, "Voltage", &["CPU Core", "Core (SVI2 TFN)", "Core (SVI3 TFN)", "Core"]).filter(|v| *v > 0.0),
            bus: val(h, "Clock", &["Bus Speed"]),
            load: val(h, "Load", &["CPU Total"]),
            cores,
            ..Default::default()
        };
    }

    // ---- motherboard sensor chip(s) ----
    let mut board = Board::default();
    let mut fans: Vec<Fan> = Vec::new();
    for h in by_type("SuperIO").chain(by_type("EmbeddedController")) {
        if board.chip.is_empty() {
            board.chip = h.n.clone();
        }
        for s in &h.s {
            let Some(v) = s.v else { continue };
            match s.t.as_str() {
                "Fan" => {
                    let pct = h.s.iter().find(|c| c.t == "Control" && c.n == s.n).and_then(|c| c.v);
                    fans.push(Fan { label: s.n.clone(), rpm: v.max(0.0), pct });
                }
                "Temperature" if plausible_temp(v) => board.temps.push(Named { label: s.n.clone(), value: v }),
                "Voltage" if v > 0.05 => board.volts.push(Named { label: s.n.clone(), value: v }),
                "Power" => {
                    board.power.get_or_insert(v);
                }
                _ => {}
            }
        }
    }
    board.headers = fans.len();
    let find_t = |pats: &[&str]| {
        board.temps.iter().find(|t| pats.iter().any(|p| t.label.to_lowercase().contains(p))).map(|t| t.value)
    };
    board.vrm = find_t(&["vrm mos", "vrm", "mos", "vcore"]).filter(|_| true);
    board.vrm_mem = find_t(&["dram vrm", "vrm dram", "memory vrm", "vrm soc", "vsoc"]);
    board.pch = find_t(&["pch", "chipset"]);
    board.dram_v = board
        .volts
        .iter()
        .find(|v| ["dram", "dimm", "vddq", "memory"].iter().any(|p| v.label.to_lowercase().contains(p)))
        .map(|v| v.value);
    let cpu_vcore = board.volts.iter().find(|v| v.label.eq_ignore_ascii_case("Vcore") || v.label.eq_ignore_ascii_case("CPU Core")).map(|v| v.value);
    snap.cpu.vcore = cpu_vcore.or(snap.cpu.vid);
    snap.board = board;

    // ---- memory modules (SPD) ----
    for h in by_type("Memory").filter(|h| h.id.contains("/dimm")) {
        let t = |n: &str| h.s.iter().find(|s| s.t == "Timing" && s.n.starts_with(n)).and_then(|s| s.v);
        snap.spd.push(Spd {
            name: h.n.clone(),
            capacity_gb: val(h, "Data", &["Capacity"]),
            tck_min: t("tCKAVGmin"),
            taa: t("tAA"),
            trcd: t("tRCD"),
            trp: t("tRP"),
            tras: t("tRAS"),
            trc: t("tRC "),
            trfc: t("tRFC1").or_else(|| t("tRFC")),
            temp: h.s.iter().find(|s| s.t == "Temperature").and_then(|s| s.v),
        });
    }

    // ---- GPU (first discrete) ----
    if let Some(h) = sm.hw.iter().find(|h| h.t == "GpuNvidia" || h.t == "GpuAmd").or_else(|| by_type("GpuIntel").next()) {
        let fan = h.s.iter().find(|s| s.t == "Fan").and_then(|s| s.v);
        snap.gpu = Gpu {
            name: h.n.clone(),
            temp: val(h, "Temperature", &["GPU Core", "GPU"]),
            hotspot: val(h, "Temperature", &["GPU Hot Spot", "Hot Spot"]),
            mem_junction: val(h, "Temperature", &["GPU Memory Junction", "GPU Memory"]),
            vrm: val(h, "Temperature", &["GPU VR", "VRM", "GPU VRM"]),
            clock: val(h, "Clock", &["GPU Core"]),
            mem_clock: val(h, "Clock", &["GPU Memory"]),
            load: val(h, "Load", &["GPU Core", "D3D 3D"]),
            mem_load: val(h, "Load", &["GPU Memory Controller"]),
            voltage: val(h, "Voltage", &["GPU Core Voltage", "GPU Core", "GPU SoC"]),
            power: val(h, "Power", &["GPU Package", "GPU Power", "GPU Board Power", "GPU Core"]),
            fan_rpm: fan,
            fan_pct: h.s.iter().find(|s| s.t == "Control").and_then(|s| s.v),
            vram_used_mb: val(h, "SmallData", &["GPU Memory Used", "D3D Dedicated Memory Used"]),
            vram_total_mb: val(h, "SmallData", &["GPU Memory Total"]),
            ..Default::default()
        };
    }

    // ---- drives ----
    for h in by_type("Storage") {
        snap.drives.push(Drive {
            name: h.n.trim().to_string(),
            temp: val(h, "Temperature", &["Temperature", "Composite Temperature"]),
            life: val(h, "Level", &["Life", "Percentage Used"]),
            hours: val(h, "Factor", &["Power On Hours"]),
        });
    }

    snap.fans = fans;
    snap
}
