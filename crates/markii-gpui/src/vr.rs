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

use markii_core::snapshot::Snapshot;
use std::io::{BufRead, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------- Mark II side

pub struct Vr {
    child: Mutex<Option<Child>>,
}

pub fn global() -> &'static Vr {
    static V: OnceLock<Vr> = OnceLock::new();
    V.get_or_init(|| Vr { child: Mutex::new(None) })
}

impl Vr {
    pub fn start(&self) {
        let mut c = self.child.lock().unwrap();
        if c.as_mut().is_some_and(|ch| ch.try_wait().ok().flatten().is_none()) {
            return;
        }
        let Ok(exe) = std::env::current_exe() else { return };
        let mut cmd = Command::new(exe);
        cmd.arg("--vr-overlay").stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        *c = cmd.spawn().ok();
    }

    pub fn stop(&self) {
        if let Some(mut ch) = self.child.lock().unwrap().take() {
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
            let mut s = Self { lib, sys, ov, handle: 0, attached: false };
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
    let wait = |d: Duration, alive: &AtomicBool| {
        let t0 = Instant::now();
        while t0.elapsed() < d && alive.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(100));
        }
    };
    while alive.load(Ordering::SeqCst) {
        if !steamvr_running() {
            wait(Duration::from_secs(4), &alive);
            continue;
        }
        let Ok(mut s) = Session::open() else {
            wait(Duration::from_secs(6), &alive);
            continue;
        };
        let mut last_draw = Instant::now() - Duration::from_secs(5);
        let mut last_place = Instant::now() - Duration::from_secs(5);
        while alive.load(Ordering::SeqCst) {
            if s.quit_requested() {
                break;
            }
            if !s.attached && last_place.elapsed() > Duration::from_secs(2) {
                s.place();
                last_place = Instant::now();
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
    let host = if s.host.chars().count() > 24 { s.host.chars().take(24).collect::<String>() + "…" } else { s.host.clone() };
    cv.text(font, &host, 300.0, 42.0, 18.0, MUTED);

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
