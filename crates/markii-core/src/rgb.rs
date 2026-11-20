//! RGB control through OpenRGB (GPL-2.0), shipped unmodified next to Mark II.
//!
//! Mark II starts OpenRGB as a headless SDK server bound to 127.0.0.1 and talks
//! to it with the OpenRGB SDK protocol (protocol version 3: modes with
//! brightness, SaveMode and profiles). OpenRGB does the hardware work: SMBus
//! for RAM and GPUs (through the PawnIO driver on Windows), HID/USB for fan
//! hubs and coolers.
//!
//! Keeping colors without Mark II open:
//! - modes that the device can store ("save to device") are saved in the device;
//! - the whole setup is saved as the OpenRGB profile "MarkII", and a login entry
//!   runs OpenRGB once with `--profile MarkII` (Windows: scheduled task with
//!   highest privileges, needed for SMBus; Linux: ~/.config/autostart).

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

pub const PORT: u16 = 6742;
pub const PROFILE: &str = "MarkII";
const PROTOCOL: u32 = 3;

// packet ids
const REQUEST_CONTROLLER_COUNT: u32 = 0;
const REQUEST_CONTROLLER_DATA: u32 = 1;
const REQUEST_PROTOCOL_VERSION: u32 = 40;
const SET_CLIENT_NAME: u32 = 50;
const PROFILE_SAVE: u32 = 151;
const UPDATE_LEDS: u32 = 1050;
const SET_CUSTOM_MODE: u32 = 1100;
const UPDATE_MODE: u32 = 1101;
const SAVE_MODE: u32 = 1102;

// mode flags / color modes
pub const FLAG_HAS_SPEED: u32 = 1 << 0;
pub const FLAG_HAS_BRIGHTNESS: u32 = 1 << 4;
pub const FLAG_PER_LED: u32 = 1 << 5;
pub const FLAG_MODE_COLOR: u32 = 1 << 6;
pub const FLAG_MANUAL_SAVE: u32 = 1 << 8;
pub const COLORS_NONE: u32 = 0;
pub const COLORS_PER_LED: u32 = 1;
pub const COLORS_MODE_SPECIFIC: u32 = 2;

/// 0xRRGGBB (Mark II) <-> OpenRGB's 0x00BBGGRR.
pub fn to_orgb(rgb: u32) -> u32 {
    let (r, g, b) = ((rgb >> 16) & 255, (rgb >> 8) & 255, rgb & 255);
    (b << 16) | (g << 8) | r
}
pub fn from_orgb(c: u32) -> u32 {
    let (r, g, b) = (c & 255, (c >> 8) & 255, (c >> 16) & 255);
    (r << 16) | (g << 8) | b
}

#[derive(Clone, Debug, Default)]
pub struct Mode {
    pub name: String,
    pub value: i32,
    pub flags: u32,
    pub speed_min: u32,
    pub speed_max: u32,
    pub bright_min: u32,
    pub bright_max: u32,
    pub colors_min: u32,
    pub colors_max: u32,
    pub speed: u32,
    pub brightness: u32,
    pub direction: u32,
    pub color_mode: u32,
    /// OpenRGB order (0x00BBGGRR).
    pub colors: Vec<u32>,
}

impl Mode {
    pub fn takes_color(&self) -> bool {
        matches!(self.color_mode, COLORS_PER_LED | COLORS_MODE_SPECIFIC) || self.flags & (FLAG_PER_LED | FLAG_MODE_COLOR) != 0
    }
    pub fn can_save(&self) -> bool {
        self.flags & FLAG_MANUAL_SAVE != 0
    }
}

#[derive(Clone, Debug, Default)]
pub struct Zone {
    pub name: String,
    pub leds: u32,
}

#[derive(Clone, Debug, Default)]
pub struct Device {
    pub idx: u32,
    /// 0 motherboard, 1 DRAM, 2 GPU, 3 cooler, 4 LED strip, 5 keyboard, 6 mouse ...
    pub kind: i32,
    pub name: String,
    pub vendor: String,
    pub description: String,
    pub location: String,
    pub active_mode: i32,
    pub modes: Vec<Mode>,
    pub zones: Vec<Zone>,
    pub leds: usize,
    pub colors: Vec<u32>,
}

impl Device {
    /// Mark II grouping of OpenRGB device types.
    pub fn group(&self) -> &'static str {
        match self.kind {
            1 => "Memória RAM",
            2 => "Placa de vídeo",
            3 => "Cooler",
            0 => "Placa-mãe",
            4 | 11 | 15 => "Ventoinhas e fitas",
            5..=10 | 12 | 13 => "Periféricos",
            _ => "Outros",
        }
    }
}

// ---------------------------------------------------------------- wire format

struct Rd<'a> {
    b: &'a [u8],
    i: usize,
}
impl Rd<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], String> {
        let s = self.b.get(self.i..self.i + n).ok_or("pacote do OpenRGB truncado")?;
        self.i += n;
        Ok(s)
    }
    fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn i32(&mut self) -> Result<i32, String> {
        Ok(self.u32()? as i32)
    }
    fn str(&mut self) -> Result<String, String> {
        let n = self.u16()? as usize;
        let s = self.take(n)?;
        Ok(String::from_utf8_lossy(s).trim_end_matches('\0').to_string())
    }
    fn mode(&mut self) -> Result<Mode, String> {
        let name = self.str()?;
        let value = self.i32()?;
        let flags = self.u32()?;
        let (speed_min, speed_max, bright_min, bright_max) = (self.u32()?, self.u32()?, self.u32()?, self.u32()?);
        let (colors_min, colors_max, speed, brightness) = (self.u32()?, self.u32()?, self.u32()?, self.u32()?);
        let (direction, color_mode) = (self.u32()?, self.u32()?);
        let n = self.u16()?;
        let colors = (0..n).map(|_| self.u32()).collect::<Result<_, _>>()?;
        Ok(Mode { name, value, flags, speed_min, speed_max, bright_min, bright_max, colors_min, colors_max, speed, brightness, direction, color_mode, colors })
    }
}

fn put_str(v: &mut Vec<u8>, s: &str) {
    v.extend(((s.len() + 1) as u16).to_le_bytes());
    v.extend(s.as_bytes());
    v.push(0);
}

fn put_mode(v: &mut Vec<u8>, m: &Mode) {
    put_str(v, &m.name);
    for x in [m.value as u32, m.flags, m.speed_min, m.speed_max, m.bright_min, m.bright_max, m.colors_min, m.colors_max, m.speed, m.brightness, m.direction, m.color_mode] {
        v.extend(x.to_le_bytes());
    }
    v.extend((m.colors.len() as u16).to_le_bytes());
    for c in &m.colors {
        v.extend(c.to_le_bytes());
    }
}

fn parse_device(idx: u32, data: &[u8]) -> Result<Device, String> {
    let mut r = Rd { b: data, i: 4 }; // skip data_size
    let kind = r.i32()?;
    let name = r.str()?;
    let vendor = r.str()?;
    let description = r.str()?;
    let _version = r.str()?;
    let _serial = r.str()?;
    let location = r.str()?;
    let nmodes = r.u16()?;
    let active_mode = r.i32()?;
    let modes = (0..nmodes).map(|_| r.mode()).collect::<Result<Vec<_>, _>>()?;
    let nzones = r.u16()?;
    let mut zones = Vec::new();
    for _ in 0..nzones {
        let name = r.str()?;
        let _ztype = r.i32()?;
        let (_min, _max, leds) = (r.u32()?, r.u32()?, r.u32()?);
        let mlen = r.u16()? as usize;
        r.take(mlen)?; // matrix map, not needed
        zones.push(Zone { name, leds });
    }
    let nleds = r.u16()? as usize;
    for _ in 0..nleds {
        r.str()?;
        r.u32()?;
    }
    let ncolors = r.u16()?;
    let colors = (0..ncolors).map(|_| r.u32()).collect::<Result<_, _>>()?;
    Ok(Device { idx, kind, name, vendor, description, location, active_mode, modes, zones, leds: nleds, colors })
}

// ---------------------------------------------------------------- client

pub struct Client {
    s: TcpStream,
    pub protocol: u32,
}

impl Client {
    pub fn connect(port: u16) -> Result<Self, String> {
        let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
        let s = TcpStream::connect_timeout(&addr, Duration::from_millis(800)).map_err(|e| e.to_string())?;
        s.set_read_timeout(Some(Duration::from_secs(5))).ok();
        s.set_nodelay(true).ok();
        let mut c = Self { s, protocol: 0 };
        c.send(0, REQUEST_PROTOCOL_VERSION, &PROTOCOL.to_le_bytes())?;
        let v = c.recv(REQUEST_PROTOCOL_VERSION).ok().and_then(|d| d.get(..4).map(|b| u32::from_le_bytes(b.try_into().unwrap()))).unwrap_or(0);
        c.protocol = v.min(PROTOCOL);
        if c.protocol < 3 {
            return Err(format!("OpenRGB antigo demais (protocolo {v})"));
        }
        let mut name = b"Mark II".to_vec();
        name.push(0);
        c.send(0, SET_CLIENT_NAME, &name)?;
        Ok(c)
    }

    fn send(&mut self, dev: u32, id: u32, data: &[u8]) -> Result<(), String> {
        let mut p = Vec::with_capacity(16 + data.len());
        p.extend(b"ORGB");
        p.extend(dev.to_le_bytes());
        p.extend(id.to_le_bytes());
        p.extend((data.len() as u32).to_le_bytes());
        p.extend(data);
        self.s.write_all(&p).map_err(|e| e.to_string())
    }

    /// Next packet with this id (skips notifications such as "device list updated").
    fn recv(&mut self, want: u32) -> Result<Vec<u8>, String> {
        for _ in 0..64 {
            let mut h = [0u8; 16];
            self.s.read_exact(&mut h).map_err(|e| e.to_string())?;
            if &h[..4] != b"ORGB" {
                return Err("resposta inválida do OpenRGB".into());
            }
            let id = u32::from_le_bytes(h[8..12].try_into().unwrap());
            let n = u32::from_le_bytes(h[12..16].try_into().unwrap()) as usize;
            let mut d = vec![0u8; n];
            self.s.read_exact(&mut d).map_err(|e| e.to_string())?;
            if id == want {
                return Ok(d);
            }
        }
        Err("o OpenRGB não respondeu".into())
    }

    pub fn count(&mut self) -> Result<u32, String> {
        self.send(0, REQUEST_CONTROLLER_COUNT, &[])?;
        let d = self.recv(REQUEST_CONTROLLER_COUNT)?;
        Ok(u32::from_le_bytes(d.get(..4).ok_or("vazio")?.try_into().unwrap()))
    }

    pub fn device(&mut self, idx: u32) -> Result<Device, String> {
        self.send(idx, REQUEST_CONTROLLER_DATA, &self.protocol.to_le_bytes())?;
        parse_device(idx, &self.recv(REQUEST_CONTROLLER_DATA)?)
    }

    pub fn devices(&mut self) -> Result<Vec<Device>, String> {
        let n = self.count()?;
        (0..n).map(|i| self.device(i)).collect()
    }

    fn mode_packet(idx: i32, m: &Mode) -> Vec<u8> {
        let mut body = idx.to_le_bytes().to_vec();
        put_mode(&mut body, m);
        let mut v = ((body.len() + 4) as u32).to_le_bytes().to_vec();
        v.extend(body);
        v
    }

    pub fn update_mode(&mut self, dev: u32, idx: i32, m: &Mode) -> Result<(), String> {
        self.send(dev, UPDATE_MODE, &Self::mode_packet(idx, m))
    }

    pub fn save_mode(&mut self, dev: u32, idx: i32, m: &Mode) -> Result<(), String> {
        self.send(dev, SAVE_MODE, &Self::mode_packet(idx, m))
    }

    pub fn custom_mode(&mut self, dev: u32) -> Result<(), String> {
        self.send(dev, SET_CUSTOM_MODE, &[])
    }

    /// Colors in OpenRGB order, one per LED.
    pub fn update_leds(&mut self, dev: u32, colors: &[u32]) -> Result<(), String> {
        let mut v = ((4 + 2 + colors.len() * 4) as u32).to_le_bytes().to_vec();
        v.extend((colors.len() as u16).to_le_bytes());
        for c in colors {
            v.extend(c.to_le_bytes());
        }
        self.send(dev, UPDATE_LEDS, &v)
    }

    pub fn save_profile(&mut self, name: &str) -> Result<(), String> {
        let mut v = name.as_bytes().to_vec();
        v.push(0);
        self.send(0, PROFILE_SAVE, &v)
    }

    /// Applies one color + effect to a device. `mode` indexes `dev.modes`;
    /// `brightness` is 0..=100 (scaled to the mode's range).
    pub fn apply(&mut self, dev: &Device, mode: usize, rgb: u32, brightness: u8, save: bool) -> Result<(), String> {
        let mut m = dev.modes.get(mode).cloned().ok_or("efeito inválido")?;
        let c = to_orgb(rgb);
        if m.flags & FLAG_HAS_BRIGHTNESS != 0 && m.bright_max != m.bright_min {
            let (lo, hi) = (m.bright_min.min(m.bright_max), m.bright_min.max(m.bright_max));
            m.brightness = lo + ((hi - lo) as f64 * brightness as f64 / 100.0).round() as u32;
        }
        if m.color_mode == COLORS_MODE_SPECIFIC || (m.flags & FLAG_MODE_COLOR != 0 && m.color_mode != COLORS_PER_LED) {
            let n = (m.colors.len() as u32).max(m.colors_min).max(1) as usize;
            m.colors = vec![c; n];
            m.color_mode = COLORS_MODE_SPECIFIC;
        }
        self.update_mode(dev.idx, mode as i32, &m)?;
        if m.color_mode == COLORS_PER_LED && dev.leds > 0 {
            // per-LED modes ("Direct"/"Static" on many devices) take the LED colors
            let scale = |v: u32| (v as f64 * brightness as f64 / 100.0).round() as u32;
            let dim = if m.flags & FLAG_HAS_BRIGHTNESS != 0 { c } else { scale(c & 255) | (scale((c >> 8) & 255) << 8) | (scale((c >> 16) & 255) << 16) };
            self.update_leds(dev.idx, &vec![dim; dev.leds])?;
        }
        if save && m.can_save() {
            self.save_mode(dev.idx, mode as i32, &m)?;
        }
        Ok(())
    }
}

/// The mode to preselect for a device: a fixed color the hardware can keep.
pub fn default_mode(d: &Device) -> usize {
    let find = |names: &[&str]| d.modes.iter().position(|m| names.iter().any(|n| m.name.eq_ignore_ascii_case(n)));
    find(&["Static", "Estático"]).or_else(|| find(&["Direct"])).unwrap_or(d.active_mode.max(0) as usize)
}

// ---------------------------------------------------------------- the OpenRGB process

#[derive(Clone, Debug, PartialEq, Default)]
pub enum Status {
    #[default]
    Off,
    /// OpenRGB is starting and detecting devices (takes a few seconds).
    Starting,
    Running,
    NotShipped,
    Failed(String),
}

impl Status {
    pub fn text(&self) -> String {
        match self {
            Status::Off => "desligado".into(),
            Status::Starting => "iniciando o OpenRGB e procurando dispositivos…".into(),
            Status::Running => "conectado".into(),
            Status::NotShipped => "o OpenRGB não está na pasta do Mark II; extraia o zip completo".into(),
            Status::Failed(e) => format!("falhou: {e}"),
        }
    }
}

#[derive(Default)]
struct State {
    status: Status,
    devices: Vec<Device>,
    #[cfg(windows)]
    process: Option<isize>,
    #[cfg(not(windows))]
    child: Option<std::process::Child>,
}

pub struct Rgb {
    st: Arc<Mutex<State>>,
}

pub fn global() -> &'static Rgb {
    static R: OnceLock<Rgb> = OnceLock::new();
    R.get_or_init(|| Rgb { st: Arc::new(Mutex::new(State::default())) })
}

/// OpenRGB shipped with Mark II: `openrgb/OpenRGB.exe` (Windows) or
/// `OpenRGB.AppImage` (Linux) next to the app; `vendor/openrgb/` in a dev tree.
pub fn openrgb_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("MARKII_OPENRGB").map(PathBuf::from).filter(|p| p.exists()) {
        return Some(p);
    }
    #[cfg(windows)]
    let rel = ["openrgb/OpenRGB.exe", "OpenRGB.exe"];
    #[cfg(not(windows))]
    let rel = ["OpenRGB.AppImage", "openrgb/OpenRGB.AppImage"];
    let d = crate::lhm::exe_dir()?;
    rel.iter()
        .flat_map(|r| [d.join(r), d.join("..").join("..").join("vendor").join("openrgb").join(r)])
        .find(|p| p.exists())
}

/// OpenRGB's own settings and profiles live with Mark II's config.
pub fn config_dir() -> PathBuf {
    #[cfg(windows)]
    let base = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(not(windows))]
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")));
    base.unwrap_or_else(|| PathBuf::from(".")).join("com.akaineko.markii").join("openrgb")
}

fn quote(s: &str) -> String {
    format!("\"{s}\"")
}

impl Rgb {
    pub fn status(&self) -> Status {
        self.st.lock().unwrap().status.clone()
    }

    pub fn devices(&self) -> Vec<Device> {
        self.st.lock().unwrap().devices.clone()
    }

    fn set(&self, s: Status) {
        self.st.lock().unwrap().status = s;
    }

    /// Starts OpenRGB as a local SDK server (or connects to one already running)
    /// and loads the device list. Non-blocking. On Windows `admin` asks for UAC,
    /// which OpenRGB needs for RAM, GPU and motherboard (SMBus).
    pub fn start(&'static self, admin: bool) {
        {
            let mut st = self.st.lock().unwrap();
            if matches!(st.status, Status::Starting | Status::Running) {
                return;
            }
            st.status = Status::Starting;
        }
        std::thread::spawn(move || {
            if Client::connect(PORT).is_err() {
                if let Err(e) = self.launch(admin) {
                    self.set(e);
                    return;
                }
            }
            // device detection runs inside OpenRGB; wait until the list settles
            let t0 = Instant::now();
            let mut last = (usize::MAX, Instant::now());
            while t0.elapsed() < Duration::from_secs(40) {
                std::thread::sleep(Duration::from_millis(700));
                let Ok(mut c) = Client::connect(PORT) else { continue };
                match c.devices() {
                    Ok(d) => {
                        let n = d.len();
                        self.st.lock().unwrap().devices = d;
                        if n != last.0 {
                            last = (n, Instant::now());
                        } else if last.1.elapsed() > Duration::from_secs(4) {
                            break;
                        }
                    }
                    Err(e) => {
                        self.set(Status::Failed(e));
                        return;
                    }
                }
            }
            self.set(if t0.elapsed() >= Duration::from_secs(40) && last.0 == usize::MAX {
                Status::Failed("o OpenRGB não respondeu".into())
            } else {
                Status::Running
            });
        });
    }

    fn launch(&self, admin: bool) -> Result<(), Status> {
        let exe = openrgb_path().ok_or(Status::NotShipped)?;
        let cfg = config_dir();
        let _ = std::fs::create_dir_all(&cfg);
        let cfg_s = cfg.to_string_lossy().to_string();
        let args = ["--server", "--server-host", "127.0.0.1", "--server-port", &PORT.to_string(), "--noautoconnect", "--config", &cfg_s];
        #[cfg(windows)]
        {
            let line = args.iter().map(|a| if a.contains(' ') { quote(a) } else { a.to_string() }).collect::<Vec<_>>().join(" ");
            let h = if admin {
                crate::lhm::elevate(&exe, &line, false).or_else(|e| if e == 1223 { spawn(&exe, &args) } else { Err(format!("erro {e}")) })
            } else {
                spawn(&exe, &args)
            }
            .map_err(Status::Failed)?;
            self.st.lock().unwrap().process = Some(h as isize);
        }
        #[cfg(not(windows))]
        {
            let _ = admin;
            let child = std::process::Command::new(&exe).args(args).spawn().map_err(|e| Status::Failed(e.to_string()))?;
            self.st.lock().unwrap().child = Some(child);
        }
        Ok(())
    }

    /// Closes the OpenRGB server Mark II started (called when Mark II quits).
    pub fn stop(&self) {
        let mut st = self.st.lock().unwrap();
        #[cfg(windows)]
        if let Some(h) = st.process.take() {
            unsafe {
                windows_sys::Win32::System::Threading::TerminateProcess(h as _, 0);
                windows_sys::Win32::Foundation::CloseHandle(h as _);
            }
        }
        #[cfg(not(windows))]
        if let Some(mut c) = st.child.take() {
            let _ = c.kill();
        }
        st.status = Status::Off;
    }

    /// Re-reads one device after a change.
    pub fn refresh(&self, idx: u32) {
        if let Ok(d) = Client::connect(PORT).and_then(|mut c| c.device(idx)) {
            if let Some(slot) = self.st.lock().unwrap().devices.iter_mut().find(|x| x.idx == idx) {
                *slot = d;
            }
        }
    }
}

#[cfg(windows)]
fn spawn(exe: &std::path::Path, args: &[&str]) -> Result<windows_sys::Win32::Foundation::HANDLE, String> {
    use std::os::windows::io::IntoRawHandle;
    use std::os::windows::process::CommandExt;
    let child = std::process::Command::new(exe)
        .args(args)
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(child.into_raw_handle() as _)
}

// ---------------------------------------------------------------- apply at login

#[cfg(windows)]
const TASK: &str = "Mark II RGB";

/// True when the colors are reapplied at login.
pub fn autostart_enabled() -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("schtasks")
            .args(["/Query", "/TN", TASK])
            .creation_flags(0x0800_0000)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
    #[cfg(not(windows))]
    {
        autostart_file().map(|f| f.exists()).unwrap_or(false)
    }
}

#[cfg(not(windows))]
fn autostart_file() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("autostart").join("markii-rgb.desktop"))
}

/// Saves the current setup as the "MarkII" profile and makes OpenRGB apply it
/// at every login, without Mark II open. Windows asks for UAC once (the task
/// runs with highest privileges so RAM/GPU over SMBus work at login).
pub fn enable_autostart(c: &mut Client) -> Result<(), String> {
    c.save_profile(PROFILE)?;
    let exe = openrgb_path().ok_or_else(|| Status::NotShipped.text())?;
    let cfg = config_dir().to_string_lossy().to_string();
    #[cfg(windows)]
    {
        let tr = format!("\\\"{}\\\" --profile {PROFILE} --noautoconnect --config \\\"{}\\\"", exe.display(), cfg);
        let args = format!("/Create /F /TN \"{TASK}\" /SC ONLOGON /RL HIGHEST /DELAY 0000:20 /TR \"{tr}\"");
        let schtasks = std::path::Path::new("schtasks.exe");
        let h = crate::lhm::elevate(schtasks, &args, true).map_err(|e| if e == 1223 { "permissão de administrador recusada".to_string() } else { format!("erro {e}") })?;
        unsafe { windows_sys::Win32::Foundation::CloseHandle(h) };
        if !autostart_enabled() {
            return Err("não foi possível criar a tarefa de login".into());
        }
    }
    #[cfg(not(windows))]
    {
        let f = autostart_file().ok_or("pasta de configuração não encontrada")?;
        std::fs::create_dir_all(f.parent().unwrap()).map_err(|e| e.to_string())?;
        let body = format!(
            "[Desktop Entry]\nType=Application\nName=Mark II RGB\nComment=Aplica as cores salvas no Mark II\nExec=\"{}\" --profile {PROFILE} --noautoconnect --config \"{}\"\nTerminal=false\nX-GNOME-Autostart-Delay=15\n",
            exe.display(),
            cfg
        );
        std::fs::write(&f, body).map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn disable_autostart() -> Result<(), String> {
    #[cfg(windows)]
    {
        let h = crate::lhm::elevate(std::path::Path::new("schtasks.exe"), &format!("/Delete /F /TN \"{TASK}\""), true)
            .map_err(|e| if e == 1223 { "permissão de administrador recusada".to_string() } else { format!("erro {e}") })?;
        unsafe { windows_sys::Win32::Foundation::CloseHandle(h) };
    }
    #[cfg(not(windows))]
    if let Some(f) = autostart_file() {
        let _ = std::fs::remove_file(f);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_and_modes_roundtrip() {
        assert_eq!(to_orgb(0xFF8000), 0x0080FF);
        assert_eq!(from_orgb(to_orgb(0x123456)), 0x123456);
        let m = Mode { name: "Static".into(), value: 1, flags: FLAG_MODE_COLOR | FLAG_MANUAL_SAVE, color_mode: COLORS_MODE_SPECIFIC, colors: vec![0xFF], colors_min: 1, colors_max: 1, ..Default::default() };
        // device data: size, type, 6 strings, 1 mode, 1 zone, 2 leds, 2 colors
        let mut d = vec![0u8; 4];
        d.extend(1i32.to_le_bytes());
        for s in ["Kingston FURY", "Kingston", "DRAM", "1.0", "", "I2C: 0x61"] {
            put_str(&mut d, s);
        }
        d.extend(1u16.to_le_bytes());
        d.extend(0i32.to_le_bytes());
        put_mode(&mut d, &m);
        d.extend(1u16.to_le_bytes());
        put_str(&mut d, "DRAM");
        d.extend(1i32.to_le_bytes());
        for x in [0u32, 2, 2] {
            d.extend(x.to_le_bytes());
        }
        d.extend(0u16.to_le_bytes());
        d.extend(2u16.to_le_bytes());
        for i in 0..2u32 {
            put_str(&mut d, &format!("LED {i}"));
            d.extend(i.to_le_bytes());
        }
        d.extend(2u16.to_le_bytes());
        d.extend(0u32.to_le_bytes());
        d.extend(0u32.to_le_bytes());
        let dev = parse_device(3, &d).unwrap();
        assert_eq!((dev.kind, dev.name.as_str(), dev.leds, dev.zones[0].leds), (1, "Kingston FURY", 2, 2));
        assert_eq!(dev.modes[0].name, "Static");
        assert!(dev.modes[0].can_save() && dev.modes[0].takes_color());
        assert_eq!(dev.group(), "Memória RAM");
        assert_eq!(default_mode(&dev), 0);
    }
}

/// Talks to a real OpenRGB server: `cargo test -p markii-core --lib live_openrgb -- --ignored`
#[cfg(test)]
#[test]
#[ignore]
fn live_openrgb() {
    let mut c = Client::connect(PORT).expect("OpenRGB SDK server on 127.0.0.1:6742");
    assert!(c.protocol >= 3);
    let devs = c.devices().expect("device list");
    for d in &devs {
        println!("{} [{}] {} modes, {} leds", d.name, d.group(), d.modes.len(), d.leds);
    }
    c.save_profile("MarkIITest").expect("save profile");
}
