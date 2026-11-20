//! SteamVR overlay: a small Mark II panel on the left wrist while SteamVR runs.
//!
//! It runs as a separate process (`markii --vr-overlay`) so that SteamVR, which
//! ends overlay apps when it quits, can never take the Mark II window with it.
//! Mark II writes one JSON `Snapshot` per line to the child's stdin; when Mark II
//! ends (even if it is killed) the pipe closes and the child exits, so nothing
//! stays in the background. The child never starts SteamVR: it waits for
//! vrserver to be running, attaches, and detaches when SteamVR quits.
//!
//! OpenVR is used through its C function tables (openvr_capi.h, v2.15.6),
//! loading the official openvr_api.dll / libopenvr_api.so shipped next to Mark II.

use markii_core::snapshot::{Snapshot, VrBattery};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------- Mark II side

pub struct Vr {
    child: Mutex<Option<Child>>,
    /// Last battery report from the panel process, and when it came.
    batteries: Arc<Mutex<(Instant, Vec<VrBattery>)>>,
}

pub fn global() -> &'static Vr {
    static V: OnceLock<Vr> = OnceLock::new();
    V.get_or_init(|| Vr { child: Mutex::new(None), batteries: Arc::new(Mutex::new((Instant::now(), Vec::new()))) })
}

/// Prefix of the battery lines the panel process prints on stdout.
const BATTERY_MARK: &str = "@markii-vr ";

impl Vr {
    pub fn start(&self) {
        let mut c = self.child.lock().unwrap();
        if c.as_mut().is_some_and(|ch| ch.try_wait().ok().flatten().is_none()) {
            return;
        }
        let Ok(exe) = std::env::current_exe() else { return };
        let mut cmd = Command::new(exe);
        // stdout carries the panel's log lines back to Mark II
        cmd.arg("--vr-overlay").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        *c = match cmd.spawn() {
            Ok(mut ch) => {
                if let Some(out) = ch.stdout.take() {
                    let bats = self.batteries.clone();
                    std::thread::spawn(move || {
                        for line in BufReader::new(out).lines() {
                            let Ok(line) = line else { break };
                            if let Some(j) = line.strip_prefix(BATTERY_MARK) {
                                if let Ok(v) = serde_json::from_str::<Vec<VrBattery>>(j) {
                                    *bats.lock().unwrap() = (Instant::now(), v);
                                }
                                continue;
                            }
                            markii_core::log::push_child(&line);
                        }
                    });
                }
                markii_core::log::info("modo VR ligado (painel do pulso aguardando o SteamVR)");
                Some(ch)
            }
            Err(e) => {
                markii_core::log::error(format!("modo VR: não consegui abrir o processo do painel: {e}"));
                None
            }
        };
    }

    pub fn stop(&self) {
        self.batteries.lock().unwrap().1.clear();
        if let Some(mut ch) = self.child.lock().unwrap().take() {
            markii_core::log::info("modo VR desligado");
            drop(ch.stdin.take()); // EOF: the child detaches from SteamVR and exits
            let t0 = Instant::now();
            while t0.elapsed() < Duration::from_millis(1500) {
                if ch.try_wait().ok().flatten().is_some() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            let _ = ch.kill();
            let _ = ch.wait();
        }
    }

    pub fn running(&self) -> bool {
        self.child.lock().unwrap().is_some()
    }

    /// Batteries of the SteamVR devices; empty when the panel is off, SteamVR is
    /// closed or no report came in the last 30 s.
    pub fn batteries(&self) -> Vec<VrBattery> {
        if !self.running() {
            return Vec::new();
        }
        let b = self.batteries.lock().unwrap();
        if b.0.elapsed() > Duration::from_secs(30) { Vec::new() } else { b.1.clone() }
    }

    pub fn send(&self, s: &Snapshot) {
        let mut c = self.child.lock().unwrap();
        let ok = c.as_mut().and_then(|ch| ch.stdin.as_mut()).map(|w| {
            let line = serde_json::to_string(s).unwrap_or_default();
            writeln!(w, "{line}").and_then(|_| w.flush()).is_ok()
        });
        if ok == Some(false) {
            *c = None; // the child is gone
        }
    }
}

// ---------------------------------------------------------------- overlay process

const W: u32 = 640;
const H: u32 = 340;
const EVENT_SIZE: u32 = 64; // sizeof(VREvent_t), openvr_capi.h 2.x
const EVENT_QUIT: u32 = 700;
const ROLE_LEFT_HAND: i32 = 1;
const INVALID_DEVICE: u32 = u32::MAX;
// function-table indices (openvr_capi.h v2.15.6: IVRSystem_026, IVROverlay_028)
const SYS_CONTROLLER_FOR_ROLE: usize = 18;
const SYS_ROLE_FOR_DEVICE: usize = 19;
const SYS_DEVICE_CLASS: usize = 20;
const SYS_IS_CONNECTED: usize = 21;
const SYS_GET_BOOL: usize = 22;
const SYS_GET_FLOAT: usize = 23;
const SYS_GET_STRING: usize = 28;
const PROP_TRACKING_SYSTEM: i32 = 1000;
const PROP_MODEL: i32 = 1001;
const PROP_MANUFACTURER: i32 = 1005;
const MAX_DEVICES: u32 = 64;
const CLASS_HMD: i32 = 1;
const CLASS_CONTROLLER: i32 = 2;
const CLASS_TRACKER: i32 = 3;
const PROP_IS_CHARGING: i32 = 1011;
const PROP_BATTERY_PCT: i32 = 1012;
const PROP_PROVIDES_BATTERY: i32 = 1026;
const SYS_POLL_NEXT_EVENT: usize = 30;
const SYS_ACK_QUIT: usize = 47;
const OV_CREATE: usize = 1;
const OV_DESTROY: usize = 3;
const OV_SET_ALPHA: usize = 16;
const OV_SET_WIDTH: usize = 22;
const OV_SET_DEVICE_RELATIVE: usize = 35;
const OV_SHOW: usize = 43;
const OV_SET_RAW: usize = 62;

type Mat34 = [[f32; 4]; 3];

fn dll_path() -> Option<std::path::PathBuf> {
    #[cfg(windows)]
    let name = "openvr_api.dll";
    #[cfg(not(windows))]
    let name = "libopenvr_api.so";
    let d = std::env::current_exe().ok()?.parent()?.to_path_buf();
    [d.join(name), d.join("openvr").join(name), d.join("..").join("..").join("vendor").join("openvr").join(name)]
        .into_iter()
        .find(|p| p.exists())
}

fn steamvr_running() -> bool {
    let mut sys = sysinfo::System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    sys.processes().values().any(|p| {
        let n = p.name().to_string_lossy().to_ascii_lowercase();
        n == "vrserver.exe" || n == "vrserver"
    })
}

struct Session {
    lib: libloading::Library,
    sys: *const usize,
    ov: *const usize,
    handle: u64,
    attached: bool,
    /// What was last logged about the devices, to log again only when it changes.
    devices_logged: String,
}

impl Session {
    fn open() -> Result<Self, String> {
        let path = dll_path().ok_or("openvr_api não encontrado")?;
        let lib = unsafe { libloading::Library::new(&path) }.map_err(|e| e.to_string())?;
        unsafe {
            let mut err = 0i32;
            let init2: Result<libloading::Symbol<unsafe extern "C" fn(*mut i32, i32, *const i8) -> isize>, _> = lib.get(b"VR_InitInternal2\0");
            match init2 {
                Ok(f) => {
                    f(&mut err, 2, std::ptr::null()); // VRApplication_Overlay
                }
                Err(_) => {
                    let f: libloading::Symbol<unsafe extern "C" fn(*mut i32, i32) -> isize> = lib.get(b"VR_InitInternal\0").map_err(|e| e.to_string())?;
                    f(&mut err, 2);
                }
            }
            if err != 0 {
                return Err(format!("VR_Init {err}"));
            }
            let get: libloading::Symbol<unsafe extern "C" fn(*const i8, *mut i32) -> isize> = lib.get(b"VR_GetGenericInterface\0").map_err(|e| e.to_string())?;
            let sys = get(b"FnTable:IVRSystem_026\0".as_ptr() as _, &mut err) as *const usize;
            let ov = get(b"FnTable:IVROverlay_028\0".as_ptr() as _, &mut err) as *const usize;
            let mut s = Self { lib, sys, ov, handle: 0, attached: false, devices_logged: String::new() };
            if sys.is_null() || ov.is_null() {
                s.shutdown();
                return Err("interface do SteamVR indisponível".into());
            }
            let create: extern "C" fn(*const i8, *const i8, *mut u64) -> i32 = std::mem::transmute(*s.ov.add(OV_CREATE));
            let e = create(b"com.akaineko.markii.wrist\0".as_ptr() as _, b"Mark II\0".as_ptr() as _, &mut s.handle);
            if e != 0 {
                s.shutdown();
                return Err(format!("CreateOverlay {e}"));
            }
            let width: extern "C" fn(u64, f32) -> i32 = std::mem::transmute(*s.ov.add(OV_SET_WIDTH));
            width(s.handle, 0.17);
            let alpha: extern "C" fn(u64, f32) -> i32 = std::mem::transmute(*s.ov.add(OV_SET_ALPHA));
            alpha(s.handle, 0.94);
            let show: extern "C" fn(u64) -> i32 = std::mem::transmute(*s.ov.add(OV_SHOW));
            show(s.handle);
            Ok(s)
        }
    }

    /// Watch-style on the left controller; until it shows up, a bit below the
    /// line of sight in front of the headset.
    fn place(&mut self) {
        if self.attached {
            return;
        }
        unsafe {
            let role: extern "C" fn(i32) -> u32 = std::mem::transmute(*self.sys.add(SYS_CONTROLLER_FOR_ROLE));
            let rel: extern "C" fn(u64, u32, *const Mat34) -> i32 = std::mem::transmute(*self.ov.add(OV_SET_DEVICE_RELATIVE));
            let left = role(ROLE_LEFT_HAND);
            if left != INVALID_DEVICE {
                // tilted -65 degrees around X: faces up and toward the eyes, over the wrist
                let a: f32 = -65f32.to_radians();
                let m: Mat34 = [[1.0, 0.0, 0.0, 0.0], [0.0, a.cos(), -a.sin(), 0.03], [0.0, a.sin(), a.cos(), 0.11]];
                self.attached = rel(self.handle, left, &m) == 0;
            } else {
                let m: Mat34 = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, -0.35], [0.0, 0.0, 1.0, -1.1]];
                rel(self.handle, 0, &m);
            }
        }
    }

    fn upload(&self, rgba: &mut [u8]) {
        unsafe {
            let raw: extern "C" fn(u64, *mut u8, u32, u32, u32) -> i32 = std::mem::transmute(*self.ov.add(OV_SET_RAW));
            raw(self.handle, rgba.as_mut_ptr(), W, H, 4);
        }
    }

    /// Battery of every connected headset / controller / tracker that reports one.
    fn batteries(&mut self) -> Vec<VrBattery> {
        let mut out = Vec::new();
        // one line per device for the log: which driver, and what it says about the battery
        let mut diag: Vec<String> = Vec::new();
        unsafe {
            let connected: extern "C" fn(u32) -> bool = std::mem::transmute(*self.sys.add(SYS_IS_CONNECTED));
            let class: extern "C" fn(u32) -> i32 = std::mem::transmute(*self.sys.add(SYS_DEVICE_CLASS));
            let role: extern "C" fn(u32) -> i32 = std::mem::transmute(*self.sys.add(SYS_ROLE_FOR_DEVICE));
            let get_bool: extern "C" fn(u32, i32, *mut i32) -> bool = std::mem::transmute(*self.sys.add(SYS_GET_BOOL));
            let get_float: extern "C" fn(u32, i32, *mut i32) -> f32 = std::mem::transmute(*self.sys.add(SYS_GET_FLOAT));
            let get_string: extern "C" fn(u32, i32, *mut u8, u32, *mut i32) -> u32 = std::mem::transmute(*self.sys.add(SYS_GET_STRING));
            let text = |i: u32, prop: i32| -> String {
                let mut buf = [0u8; 128];
                let mut e = 0i32;
                get_string(i, prop, buf.as_mut_ptr(), buf.len() as u32, &mut e);
                if e != 0 {
                    return String::new();
                }
                let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
                String::from_utf8_lossy(&buf[..end]).chars().filter(|c| !c.is_control()).take(48).collect()
            };
            for i in 0..MAX_DEVICES {
                if !connected(i) {
                    continue;
                }
                let name = match class(i) {
                    CLASS_HMD => "Óculos",
                    CLASS_CONTROLLER => match role(i) {
                        1 => "Controle esquerdo",
                        2 => "Controle direito",
                        _ => "Controle",
                    },
                    CLASS_TRACKER => "Rastreador",
                    _ => continue,
                };
                let mut e1 = 0i32;
                let provides = get_bool(i, PROP_PROVIDES_BATTERY, &mut e1) && e1 == 0;
                let mut e = 0i32;
                let frac = get_float(i, PROP_BATTERY_PCT, &mut e);
                // some drivers fill the percentage without the "provides" flag
                let ok = e == 0 && frac.is_finite() && (provides || frac > 0.0);
                diag.push(format!(
                    "{name} [{} · {} {}] informa bateria: {} (erro {e1}), porcentagem: {} (erro {e})",
                    text(i, PROP_TRACKING_SYSTEM),
                    text(i, PROP_MANUFACTURER),
                    text(i, PROP_MODEL),
                    if provides { "sim" } else { "não" },
                    if ok { "ok" } else if e == 0 && frac == 0.0 { "zero" } else { "não" },
                ));
                if !ok {
                    continue;
                }
                let mut e = 0i32;
                let charging = get_bool(i, PROP_IS_CHARGING, &mut e) && e == 0;
                out.push(VrBattery { name: name.into(), pct: (frac.clamp(0.0, 1.0) * 100.0).round(), charging });
            }
        }
        let d = diag.join(" | ");
        if d != self.devices_logged {
            markii_core::log::info(if d.is_empty() { "painel VR: nenhum óculos/controle conectado no SteamVR".to_string() } else { format!("painel VR: aparelhos: {d}") });
            self.devices_logged = d;
        }
        // headset first, then left, right, others
        let rank = |n: &str| ["Óculos", "Controle esquerdo", "Controle direito", "Controle"].iter().position(|x| *x == n).unwrap_or(9);
        out.sort_by_key(|b| rank(&b.name));
        out
    }

    /// True when SteamVR asked to quit.
    fn quit_requested(&self) -> bool {
        unsafe {
            let poll: extern "C" fn(*mut u8, u32) -> bool = std::mem::transmute(*self.sys.add(SYS_POLL_NEXT_EVENT));
            let mut ev = [0u8; EVENT_SIZE as usize];
            while poll(ev.as_mut_ptr(), EVENT_SIZE) {
                let kind = u32::from_le_bytes(ev[..4].try_into().unwrap());
                if kind == EVENT_QUIT {
                    let ack: extern "C" fn() = std::mem::transmute(*self.sys.add(SYS_ACK_QUIT));
                    ack();
                    return true;
                }
                if kind == 100 || kind == 101 {
                    // a controller was (de)activated: place again
                    return false;
                }
            }
        }
        false
    }

    fn shutdown(&mut self) {
        unsafe {
            if self.handle != 0 && !self.ov.is_null() {
                let destroy: extern "C" fn(u64) -> i32 = std::mem::transmute(*self.ov.add(OV_DESTROY));
                destroy(self.handle);
                self.handle = 0;
            }
            if let Ok(f) = self.lib.get::<unsafe extern "C" fn()>(b"VR_ShutdownInternal\0") {
                f();
            }
        }
    }
}

/// Entry point of `markii --vr-overlay`.
pub fn run_overlay() -> i32 {
    let latest: Arc<Mutex<Option<Snapshot>>> = Arc::default();
    let alive = Arc::new(AtomicBool::new(true));
    {
        let (latest, alive) = (latest.clone(), alive.clone());
        std::thread::spawn(move || {
            for line in std::io::stdin().lock().lines() {
                let Ok(line) = line else { break };
                if let Ok(s) = serde_json::from_str::<Snapshot>(&line) {
                    *latest.lock().unwrap() = Some(s);
                }
            }
            alive.store(false, Ordering::SeqCst); // Mark II is gone
        });
    }
    let font = load_font();
    if font.is_none() {
        markii_core::log::warn("painel VR: nenhuma fonte do sistema encontrada; o texto não vai aparecer");
    }
    let mut waiting_logged = false;
    let mut last_err = String::new();
    let wait = |d: Duration, alive: &AtomicBool| {
        let t0 = Instant::now();
        while t0.elapsed() < d && alive.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(100));
        }
    };
    while alive.load(Ordering::SeqCst) {
        if !steamvr_running() {
            if !waiting_logged {
                markii_core::log::info("painel VR: esperando o SteamVR abrir");
                waiting_logged = true;
            }
            wait(Duration::from_secs(4), &alive);
            continue;
        }
        let mut s = match Session::open() {
            Ok(s) => s,
            Err(e) => {
                if e != last_err {
                    markii_core::log::error(format!("painel VR: SteamVR recusou o painel: {e}"));
                    last_err = e;
                }
                wait(Duration::from_secs(6), &alive);
                continue;
            }
        };
        markii_core::log::info("painel VR: conectado ao SteamVR");
        last_err.clear();
        waiting_logged = false;
        let mut was_attached = false;
        let mut last_draw = Instant::now() - Duration::from_secs(5);
        let mut last_place = Instant::now() - Duration::from_secs(5);
        let mut last_bat = Instant::now() - Duration::from_secs(60);
        while alive.load(Ordering::SeqCst) {
            if s.quit_requested() {
                break;
            }
            if !s.attached && last_place.elapsed() > Duration::from_secs(2) {
                s.place();
                last_place = Instant::now();
            }
            if s.attached != was_attached {
                was_attached = s.attached;
                markii_core::log::info(if s.attached { "painel VR: preso ao controle esquerdo" } else { "painel VR: controle esquerdo não encontrado (painel na frente do óculos)" });
            }
            if last_bat.elapsed() >= Duration::from_secs(10) {
                let b = s.batteries();
                println!("{BATTERY_MARK}{}", serde_json::to_string(&b).unwrap_or_else(|_| "[]".into()));
                last_bat = Instant::now();
            }
            if last_draw.elapsed() >= Duration::from_millis(1000) {
                let snap = latest.lock().unwrap().clone();
                let mut px = render(snap.as_ref(), font.as_ref());
                s.upload(&mut px);
                last_draw = Instant::now();
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        s.shutdown();
        println!("{BATTERY_MARK}[]");
        markii_core::log::info(if alive.load(Ordering::SeqCst) { "painel VR: o SteamVR fechou" } else { "painel VR: encerrado junto com o Mark II" });
        // SteamVR quit: wait for it to come back, or for Mark II to end
        wait(Duration::from_secs(5), &alive);
    }
    0
}

// ---------------------------------------------------------------- drawing

fn load_font() -> Option<ab_glyph::FontVec> {
    let candidates: &[&str] = if cfg!(windows) {
        &["C:\\Windows\\Fonts\\bahnschrift.ttf", "C:\\Windows\\Fonts\\segoeui.ttf", "C:\\Windows\\Fonts\\arial.ttf"]
    } else {
        &[
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/TTF/DejaVuSans.ttf",
            "/usr/share/fonts/noto/NotoSans-Regular.ttf",
            "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
        ]
    };
    candidates.iter().find_map(|p| std::fs::read(p).ok().and_then(|b| ab_glyph::FontVec::try_from_vec(b).ok()))
}

#[derive(Clone, Copy)]
struct Rgb(u8, u8, u8);
const BG: Rgb = Rgb(0x0e, 0x11, 0x16);
const PANEL: Rgb = Rgb(0x16, 0x1b, 0x22);
const LINE: Rgb = Rgb(0x26, 0x2e, 0x39);
const TEXT: Rgb = Rgb(0xe8, 0xed, 0xf3);
const MUTED: Rgb = Rgb(0x8b, 0x96, 0xa5);
const AMBER: Rgb = Rgb(0xff, 0xb0, 0x20);
const GREEN: Rgb = Rgb(0x3f, 0xd1, 0x7a);
const RED: Rgb = Rgb(0xff, 0x5a, 0x5a);
const INDIGO: Rgb = Rgb(0x7b, 0x7f, 0xf5);
const CYAN: Rgb = Rgb(0x44, 0xc7, 0xe0);

fn temp_col(t: Option<f64>) -> Rgb {
    match t {
        Some(v) if v >= 85.0 => RED,
        Some(v) if v >= 72.0 => AMBER,
        Some(_) => GREEN,
        None => MUTED,
    }
}

struct Canvas {
    pm: tiny_skia::Pixmap,
}

impl Canvas {
    fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, c: Rgb) {
        if let Some(r) = tiny_skia::Rect::from_xywh(x, y, w.max(0.5), h) {
            let mut p = tiny_skia::Paint::default();
            p.set_color_rgba8(c.0, c.1, c.2, 255);
            self.pm.fill_rect(r, &p, tiny_skia::Transform::identity(), None);
        }
    }

    fn bar(&mut self, x: f32, y: f32, w: f32, pct: f64, c: Rgb) {
        let segs = 24;
        let on = ((pct.clamp(0.0, 100.0) / 100.0) * segs as f64).round() as usize;
        let sw = (w - (segs - 1) as f32 * 2.0) / segs as f32;
        for i in 0..segs {
            self.rect(x + i as f32 * (sw + 2.0), y, sw, 12.0, if i < on { c } else { LINE });
        }
    }

    /// Draws `s` with its baseline at `y`; returns the advance width.
    fn text(&mut self, font: Option<&ab_glyph::FontVec>, s: &str, x: f32, y: f32, size: f32, c: Rgb) -> f32 {
        use ab_glyph::{Font, ScaleFont};
        let Some(font) = font else { return 0.0 };
        let sf = font.as_scaled(size);
        let (w, h) = (self.pm.width() as i32, self.pm.height() as i32);
        let data = self.pm.data_mut();
        let mut pen = x;
        let mut prev = None;
        for ch in s.chars() {
            let id = sf.glyph_id(ch);
            if let Some(p) = prev {
                pen += sf.kern(p, id);
            }
            let g = id.with_scale_and_position(size, ab_glyph::point(pen, y));
            if let Some(og) = font.outline_glyph(g) {
                let b = og.px_bounds();
                og.draw(|gx, gy, cov| {
                    let (px, py) = (b.min.x as i32 + gx as i32, b.min.y as i32 + gy as i32);
                    if px < 0 || py < 0 || px >= w || py >= h {
                        return;
                    }
                    let i = ((py * w + px) * 4) as usize;
                    let a = cov.clamp(0.0, 1.0);
                    for (k, v) in [c.0, c.1, c.2].into_iter().enumerate() {
                        data[i + k] = (v as f32 * a + data[i + k] as f32 * (1.0 - a)).round() as u8;
                    }
                    data[i + 3] = 255;
                });
            }
            pen += sf.h_advance(id);
            prev = Some(id);
        }
        pen - x
    }
}

fn gb(b: u64) -> String {
    let g = b as f64 / 1073741824.0;
    if g >= 10.0 { format!("{g:.0}") } else { format!("{g:.1}") }
}

fn opt<T: std::fmt::Display>(v: Option<T>, unit: &str) -> String {
    v.map_or("—".into(), |v| format!("{v}{unit}"))
}

/// The wrist panel as RGBA bytes (W x H).
fn render(s: Option<&Snapshot>, font: Option<&ab_glyph::FontVec>) -> Vec<u8> {
    let mut cv = Canvas { pm: tiny_skia::Pixmap::new(W, H).unwrap() };
    cv.pm.fill(tiny_skia::Color::from_rgba8(BG.0, BG.1, BG.2, 255));
    cv.rect(0.0, 0.0, W as f32, 4.0, AMBER);
    let x0 = 22.0;
    let adv = cv.text(font, "MARK ", x0, 44.0, 26.0, TEXT);
    cv.text(font, "II", x0 + adv, 44.0, 26.0, AMBER);
    let Some(s) = s else {
        cv.text(font, "aguardando dados do Mark II…", x0, 110.0, 22.0, MUTED);
        return cv.pm.take();
    };
    if s.vr_batteries.is_empty() {
        let host = if s.host.chars().count() > 24 { s.host.chars().take(24).collect::<String>() + "…" } else { s.host.clone() };
        cv.text(font, &host, 300.0, 42.0, 18.0, MUTED);
    } else {
        // "Óculos 62%  E 80%  D 75%", red when low, cyan while charging
        let mut x = 190.0;
        for b in s.vr_batteries.iter().take(5) {
            let short = match b.name.as_str() {
                "Controle esquerdo" => "E",
                "Controle direito" => "D",
                "Controle" => "C",
                "Rastreador" => "R",
                n => n,
            };
            x += cv.text(font, &format!("{short} "), x, 42.0, 18.0, MUTED);
            let col = if b.charging { CYAN } else if b.pct <= 15.0 { RED } else if b.pct <= 30.0 { AMBER } else { GREEN };
            x += cv.text(font, &format!("{:.0}%{}", b.pct, if b.charging { "+" } else { "" }), x, 42.0, 18.0, col) + 16.0;
            if x > W as f32 - 60.0 {
                break;
            }
        }
    }

    let rows: [(&str, Rgb, f64, String, String, Rgb); 3] = [
        (
            "CPU",
            INDIGO,
            s.cpu_load as f64,
            format!("{:.0}%", s.cpu_load),
            format!("{}  ·  {}", opt(s.cpu_temp.map(|v| format!("{v:.0}")), "°C"), opt(s.cpu_power.map(|v| format!("{v:.0}")), " W")),
            temp_col(s.cpu_temp),
        ),
        (
            "GPU",
            GREEN,
            s.gpu_load.unwrap_or(0) as f64,
            opt(s.gpu_load, "%"),
            format!(
                "{}  ·  {}{}",
                opt(s.gpu_temp, "°C"),
                opt(s.gpu_power.map(|v| format!("{v:.0}")), " W"),
                match (s.vram_used, s.vram_total) {
                    (Some(u), Some(t)) if t > 0 => format!("  ·  {}/{} GB", gb(u), gb(t)),
                    _ => String::new(),
                }
            ),
            temp_col(s.gpu_temp.map(|v| v as f64)),
        ),
        ("RAM", AMBER, s.ram_pct(), format!("{:.0}%", s.ram_pct()), format!("{} / {} GB", gb(s.ram_used), gb(s.ram_total)), MUTED),
    ];
    let mut y = 66.0;
    for (name, col, pct, big, detail, dcol) in rows {
        cv.rect(14.0, y, W as f32 - 28.0, 74.0, PANEL);
        cv.rect(14.0, y + 12.0, 3.0, 18.0, col);
        cv.text(font, name, 26.0, y + 30.0, 18.0, MUTED);
        cv.text(font, &big, 26.0, y + 64.0, 32.0, TEXT);
        cv.bar(150.0, y + 18.0, W as f32 - 180.0, pct, col);
        cv.text(font, &detail, 150.0, y + 62.0, 21.0, dcol);
        y += 82.0;
    }
    if let Some(a) = s.alerts.first() {
        cv.rect(14.0, y, W as f32 - 28.0, 26.0, Rgb(0x3a, 0x14, 0x18));
        cv.text(font, &format!("⚠ {a}"), 24.0, y + 19.0, 17.0, RED);
    } else {
        let fans = s.fans.iter().take(3).map(|(n, r)| format!("{} {:.0} rpm", n.split_whitespace().next().unwrap_or(n), r)).collect::<Vec<_>>().join("  ");
        cv.text(font, if fans.is_empty() { "sem alertas" } else { &fans }, 24.0, y + 19.0, 17.0, if fans.is_empty() { GREEN } else { CYAN });
    }
    cv.pm.take()
}

/// Writes the panel to a PNG (for checking the drawing without a headset).
pub fn preview_png(path: &std::path::Path, s: &Snapshot) -> Result<(), String> {
    let px = render(Some(s), load_font().as_ref());
    let f = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(f), W, H);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().and_then(|mut w| w.write_image_data(&px)).map_err(|e| e.to_string())
}
