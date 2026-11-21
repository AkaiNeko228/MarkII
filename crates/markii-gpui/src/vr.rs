//! SteamVR overlay: a small Mark II panel on the right wrist while SteamVR runs.
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
    /// Panel settings last sent to the child (sent again only when they change).
    sent_cfg: Mutex<String>,
    /// A mode picked on the panel itself by touch, for Mark II to save.
    picked: Arc<Mutex<Option<String>>>,
}

/// What the panel needs from the settings: mode, theme and the alert limits.
pub struct PanelCfg<'a> {
    pub mode: &'a str,
    pub theme: &'a str,
    pub cpu_limit: f64,
    pub gpu_limit: f64,
}

/// The panel's modes: (config key, button label).
pub const MODES: [(&str, &str); 4] = [("relogio", "Relógio"), ("grafico", "Gráfico"), ("baterias", "Baterias"), ("hud", "HUD")];

pub fn global() -> &'static Vr {
    static V: OnceLock<Vr> = OnceLock::new();
    V.get_or_init(|| Vr {
        child: Mutex::new(None),
        batteries: Arc::new(Mutex::new((Instant::now(), Vec::new()))),
        sent_cfg: Mutex::new(String::new()),
        picked: Arc::default(),
    })
}

/// Prefix of the battery lines the panel process prints on stdout.
const BATTERY_MARK: &str = "@markii-vr ";
/// Printed by the panel when the user picks a mode on it: "@markii-vr-mode grafico".
const MODE_MARK: &str = "@markii-vr-mode ";
/// Sent to the panel with its settings: "@cfg <mode> <theme> <cpu limit> <gpu limit>".
const CFG_MARK: &str = "@cfg ";

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
        self.sent_cfg.lock().unwrap().clear(); // a new panel needs its settings
        *c = match cmd.spawn() {
            Ok(mut ch) => {
                if let Some(out) = ch.stdout.take() {
                    let bats = self.batteries.clone();
                    let picked = self.picked.clone();
                    std::thread::spawn(move || {
                        for line in BufReader::new(out).lines() {
                            let Ok(line) = line else { break };
                            if let Some(m) = line.strip_prefix(MODE_MARK) {
                                if MODES.iter().any(|x| x.0 == m.trim()) {
                                    *picked.lock().unwrap() = Some(m.trim().to_string());
                                }
                                continue;
                            }
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
        self.write_line(&serde_json::to_string(s).unwrap_or_default());
    }

    /// Sends the panel's mode, theme and limits when they changed.
    pub fn sync(&self, p: &PanelCfg) {
        let line = format!("{CFG_MARK}{} {} {:.0} {:.0}", p.mode, p.theme, p.cpu_limit, p.gpu_limit);
        let mut sent = self.sent_cfg.lock().unwrap();
        if *sent != line {
            *sent = line.clone();
            drop(sent);
            self.write_line(&line);
        }
    }

    /// The mode the user picked on the panel since the last call, if any.
    pub fn take_picked(&self) -> Option<String> {
        self.picked.lock().unwrap().take()
    }

    fn write_line(&self, line: &str) {
        let mut c = self.child.lock().unwrap();
        let ok = c.as_mut().and_then(|ch| ch.stdin.as_mut()).map(|w| writeln!(w, "{line}").and_then(|_| w.flush()).is_ok());
        if ok == Some(false) {
            *c = None; // the child is gone
        }
    }
}

// ---------------------------------------------------------------- overlay process

const W: u32 = 640;
const H: u32 = 380;
const EVENT_SIZE: u32 = 64; // sizeof(VREvent_t), openvr_capi.h 2.x
const EVENT_QUIT: u32 = 700;
const ROLE_LEFT_HAND: i32 = 1;
const ROLE_RIGHT_HAND: i32 = 2;
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
const OV_HIDE: usize = 44;
const OV_SET_SORT_ORDER: usize = 20;
const SYS_POSES: usize = 12;
const ORIGIN_STANDING: i32 = 1;
const POSE_SIZE: usize = 80; // sizeof(TrackedDevicePose_t)
/// Panel size in the world, in meters.
const PANEL_W: f32 = 0.12;
#[cfg(test)]
const PANEL_H: f32 = PANEL_W * H as f32 / W as f32;
/// What the left stick can resize the panel to while it is held, in meters.
const SIZE_RANGE: (f32, f32) = (0.05, 0.6);
/// The tip may be this far behind the panel's face and still point at it.
const BEHIND: f32 = 0.05;
const SYS_CONTROLLER_STATE: usize = 37;
/// VRControllerState_t: 4-byte packing on Linux, natural on Windows.
#[cfg(windows)]
const STATE: (usize, usize, usize) = (64, 8, 32); // size, ulButtonPressed, rAxis[1].x (trigger; [2] is the grip)
#[cfg(not(windows))]
const STATE: (usize, usize, usize) = (60, 4, 28);
const BUTTON_TRIGGER: u64 = 1 << 33;
const BUTTON_GRIP: u64 = 1 << 2;
/// Where the tip is, in the controller's own space (forward is -Z).
const TIP: [f32; 3] = [0.0, -0.01, -0.06];
/// The cursor dot shows while the tip is this close to the panel's face.
const CURSOR_NEAR: f32 = 0.2;
const CURSOR_PX: usize = 32;

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
    /// Two overlays in the same spot: the new picture goes on the hidden one,
    /// which then swaps with the visible one, so the panel never blinks.
    handles: [u64; 2],
    front: usize,
    /// Hash of the picture on screen, to skip sending the same one again.
    shown: u64,
    /// The old picture stays under the new one for a moment (SteamVR shows
    /// an overlay before its picture is ready), then hides.
    hide_at: Option<(usize, Instant)>,
    order: u32,
    /// The user dragged the panel somewhere (saved in PLACE_FILE).
    custom: bool,
    /// While the left grip drags the panel: the panel in the left controller's space.
    grab: Option<Mat34>,
    grip_down: bool,
    /// Panel width in meters (the left stick changes it while held).
    width: f32,
    attached: bool,
    /// Device the panel is attached to (0 = headset) and where on it.
    parent: u32,
    mat: Mat34,
    /// The left trigger was held on the last check (a press counts once).
    trigger_down: bool,
    /// A dot on the panel under the left controller's tip, like a laser dot.
    cursor: u64,
    cursor_on: bool,
    /// What was last logged about the devices, to log again only when it changes.
    devices_logged: String,
}

/// Where the user put the panel: 12 numbers, the panel in the right
/// controller's space, then its width. No file = the default spot on the wrist.
fn place_file() -> std::path::PathBuf {
    markii_core::log::dir().join("painel-vr.txt")
}

fn load_place() -> Option<(Mat34, f32)> {
    let s = std::fs::read_to_string(place_file()).ok()?;
    let v: Vec<f32> = s.split_whitespace().filter_map(|x| x.parse().ok()).collect();
    if !(12..=13).contains(&v.len()) || v.iter().any(|x| !x.is_finite()) {
        return None;
    }
    let mut m = [[0f32; 4]; 3];
    for r in 0..3 {
        m[r].copy_from_slice(&v[r * 4..r * 4 + 4]);
    }
    Some((m, v.get(12).copied().unwrap_or(PANEL_W).clamp(SIZE_RANGE.0, SIZE_RANGE.1)))
}

fn save_place(m: &Mat34, width: f32) {
    let s: Vec<String> = m.iter().flatten().chain(std::iter::once(&width)).map(|x| format!("{x:.5}")).collect();
    if let Err(e) = std::fs::write(place_file(), s.join(" ")) {
        markii_core::log::warn(format!("painel VR: não deu para salvar a posição: {e}"));
    }
}

/// Inverse of a rigid transform.
fn invert(m: &Mat34) -> Mat34 {
    let mut o = [[0f32; 4]; 3];
    for r in 0..3 {
        for c in 0..3 {
            o[r][c] = m[c][r];
        }
        o[r][3] = -(0..3).map(|k| m[k][r] * m[k][3]).sum::<f32>();
    }
    o
}

const TILT_DEFAULT: f32 = 0.0;
const ARM_DEFAULT: f32 = 0.14;

/// Right wrist, watch style: the face sits on top of the wrist behind the
/// grip, text running along the forearm, turned `tilt` degrees toward the eyes.
fn wrist_mat(tilt: f32, arm: f32, spin: f32) -> Mat34 {
    let (ps, pc) = spin.to_radians().sin_cos();
    mul(&wrist_base(tilt, arm), &[[pc, -ps, 0.0, 0.0], [ps, pc, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]])
}

fn wrist_base(tilt: f32, arm: f32) -> Mat34 {
    let (s, c) = tilt.to_radians().sin_cos();
    // Like a watch on the back of the right wrist: the face looks out of the
    // back of the hand (controller +X), text runs toward the elbow (+Z) and
    // its top points away from the body when the arm is raised to look
    // (controller -Y). It sits a bit below the ring, on the wrist itself.
    let base: Mat34 = [[0.0, 0.0, 1.0, 0.04], [0.0, -1.0, 0.0, -0.03], [1.0, 0.0, 0.0, arm]];
    // tilt: turns around the forearm, toward or away from the eyes
    mul(&base, &[[1.0, 0.0, 0.0, 0.0], [0.0, c, -s, 0.0], [0.0, s, c, 0.0]])
}

/// Until the right controller shows up: a bit below the line of sight.
fn front_mat() -> Mat34 {
    [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, -0.35], [0.0, 0.0, 1.0, -1.1]]
}

/// a x b for rigid 3x4 transforms.
fn mul(a: &Mat34, b: &Mat34) -> Mat34 {
    let mut m = [[0f32; 4]; 3];
    for r in 0..3 {
        for c in 0..4 {
            m[r][c] = (0..3).map(|k| a[r][k] * b[k][c]).sum::<f32>() + if c == 3 { a[r][3] } else { 0.0 };
        }
    }
    m
}

fn apply(m: &Mat34, p: [f32; 3]) -> [f32; 3] {
    let mut o = [0f32; 3];
    for r in 0..3 {
        o[r] = m[r][0] * p[0] + m[r][1] * p[1] + m[r][2] * p[2] + m[r][3];
    }
    o
}

/// World point -> panel space (x right, y up, z out of the face), in meters.
fn to_panel(panel: &Mat34, p: [f32; 3]) -> [f32; 3] {
    let d = [p[0] - panel[0][3], p[1] - panel[1][3], p[2] - panel[2][3]];
    let mut o = [0f32; 3];
    for c in 0..3 {
        o[c] = panel[0][c] * d[0] + panel[1][c] * d[1] + panel[2][c] * d[2];
    }
    o
}

/// Panel-space point -> pixel from the top left, when it is on the panel.
fn panel_pixel(l: [f32; 3], width: f32) -> Option<(f32, f32)> {
    let x = (l[0] / width + 0.5) * W as f32;
    let y = (0.5 - l[1] / (width * H as f32 / W as f32)) * H as f32;
    ((0.0..=W as f32).contains(&x) && (0.0..=H as f32).contains(&y)).then_some((x, y))
}

/// White dot with a dark ring, readable on any theme.
fn cursor_dot() -> Vec<u8> {
    let n = CURSOR_PX;
    let mut px = vec![0u8; n * n * 4];
    let c = (n as f32 - 1.0) / 2.0;
    for y in 0..n {
        for x in 0..n {
            let d = ((x as f32 - c).powi(2) + (y as f32 - c).powi(2)).sqrt();
            let r = n as f32 / 2.0 - 1.0;
            let a = (r - d + 0.5).clamp(0.0, 1.0);
            let v = if d < r - 4.0 { 255 } else { 30 };
            let i = (y * n + x) * 4;
            px[i..i + 4].copy_from_slice(&[v, v, v, (a * 255.0) as u8]);
        }
    }
    px
}

fn hash(px: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    px.hash(&mut h);
    h.finish()
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
            let mut s = Self {
                lib,
                sys,
                ov,
                handles: [0; 2],
                front: 0,
                shown: 0,
                hide_at: None,
                order: 0,
                custom: false,
                grab: None,
                grip_down: false,
                width: PANEL_W,
                attached: false,
                parent: 0,
                mat: front_mat(),
                trigger_down: false,
                cursor: 0,
                cursor_on: false,
                devices_logged: String::new(),
            };
            if sys.is_null() || ov.is_null() {
                s.shutdown();
                return Err("interface do SteamVR indisponível".into());
            }
            let create: extern "C" fn(*const i8, *const i8, *mut u64) -> i32 = std::mem::transmute(*s.ov.add(OV_CREATE));
            let width: extern "C" fn(u64, f32) -> i32 = std::mem::transmute(*s.ov.add(OV_SET_WIDTH));
            let alpha: extern "C" fn(u64, f32) -> i32 = std::mem::transmute(*s.ov.add(OV_SET_ALPHA));
            let order: extern "C" fn(u64, u32) -> i32 = std::mem::transmute(*s.ov.add(OV_SET_SORT_ORDER));
            // No input method and no "interactive" flag: SteamVR keeps the
            // controllers on the game. The buttons work by touch (see touch()).
            let keys: [&[u8]; 2] = [b"com.akaineko.markii.wrist\0", b"com.akaineko.markii.wrist2\0"];
            for (i, key) in keys.iter().enumerate() {
                let mut h = 0u64;
                let e = create(key.as_ptr() as _, b"Mark II\0".as_ptr() as _, &mut h);
                if e != 0 {
                    s.shutdown();
                    return Err(format!("CreateOverlay {e}"));
                }
                s.handles[i] = h;
                width(h, PANEL_W);
                alpha(h, 0.94);
                order(h, i as u32);
            }
            let mut h = 0u64;
            if create(b"com.akaineko.markii.cursor\0".as_ptr() as _, b"Mark II cursor\0".as_ptr() as _, &mut h) == 0 {
                s.cursor = h;
                width(h, 0.012);
                order(h, u32::MAX); // always over the panel
                let raw: extern "C" fn(u64, *mut u8, u32, u32, u32) -> i32 = std::mem::transmute(*s.ov.add(OV_SET_RAW));
                let mut dot = cursor_dot();
                raw(h, dot.as_mut_ptr(), CURSOR_PX as u32, CURSOR_PX as u32, 4);
            }
            s.apply_place();
            Ok(s)
        }
    }

    /// Attach both overlays to the right controller; until it shows up, in
    /// front of the headset.
    fn place(&mut self) {
        if self.attached {
            return;
        }
        unsafe {
            let role: extern "C" fn(i32) -> u32 = std::mem::transmute(*self.sys.add(SYS_CONTROLLER_FOR_ROLE));
            let right = role(ROLE_RIGHT_HAND);
            if right != INVALID_DEVICE {
                self.parent = right;
                let saved = load_place();
                self.custom = saved.is_some();
                self.mat = saved.map_or_else(|| wrist_mat(TILT_DEFAULT, ARM_DEFAULT, 0.0), |p| p.0);
                self.set_width(saved.map_or(PANEL_W, |p| p.1));
                self.attached = true;
            } else {
                self.parent = 0;
                self.mat = front_mat();
            }
        }
        if !self.apply_place() {
            self.attached = false;
        }
    }

    /// "Voltar o painel para o pulso" in Mark II deletes the saved spot.
    fn check_reset(&mut self) {
        if self.custom && self.grab.is_none() && !place_file().exists() {
            self.custom = false;
            self.mat = wrist_mat(TILT_DEFAULT, ARM_DEFAULT, 0.0);
            self.apply_place();
            self.set_width(PANEL_W);
            markii_core::log::info("painel VR: de volta ao pulso");
        }
    }

    fn set_width(&mut self, w: f32) {
        self.width = w;
        unsafe {
            let width: extern "C" fn(u64, f32) -> i32 = std::mem::transmute(*self.ov.add(OV_SET_WIDTH));
            for &h in &self.handles {
                width(h, w);
            }
        }
    }

    fn apply_place(&self) -> bool {
        unsafe {
            let rel: extern "C" fn(u64, u32, *const Mat34) -> i32 = std::mem::transmute(*self.ov.add(OV_SET_DEVICE_RELATIVE));
            self.handles.iter().all(|&h| rel(h, self.parent, &self.mat) == 0)
        }
    }

    /// Send a picture, only when it changed: on the hidden overlay first,
    /// then show it and hide the old one.
    fn upload(&mut self, rgba: &mut [u8]) {
        let h = hash(rgba);
        if h == self.shown {
            return;
        }
        unsafe {
            let raw: extern "C" fn(u64, *mut u8, u32, u32, u32) -> i32 = std::mem::transmute(*self.ov.add(OV_SET_RAW));
            let show: extern "C" fn(u64) -> i32 = std::mem::transmute(*self.ov.add(OV_SHOW));
            let hide: extern "C" fn(u64) -> i32 = std::mem::transmute(*self.ov.add(OV_HIDE));
            let order: extern "C" fn(u64, u32) -> i32 = std::mem::transmute(*self.ov.add(OV_SET_SORT_ORDER));
            if let Some((old, _)) = self.hide_at.take() {
                hide(self.handles[old]); // the previous swap is still pending
            }
            let back = 1 - self.front;
            if raw(self.handles[back], rgba.as_mut_ptr(), W, H, 4) != 0 {
                return;
            }
            // on top of the old picture, which stays visible underneath for a moment
            self.order = self.order.wrapping_add(1);
            order(self.handles[back], self.order);
            show(self.handles[back]);
            if self.shown != 0 {
                self.hide_at = Some((self.front, Instant::now() + Duration::from_millis(200)));
            }
            self.front = back;
        }
        self.shown = h;
    }

    /// Hides the old picture once the new one has surely reached the headset.
    fn finish_swap(&mut self) {
        if let Some((old, at)) = self.hide_at {
            if Instant::now() >= at {
                unsafe {
                    let hide: extern "C" fn(u64) -> i32 = std::mem::transmute(*self.ov.add(OV_HIDE));
                    hide(self.handles[old]);
                }
                self.hide_at = None;
            }
        }
    }

    /// Where the left controller's tip is over the panel (pixels from the top
    /// left, while it is close), and a click when the left trigger is pressed
    /// there. Also moves the cursor dot.
    /// Holding the left grip with the dot on the panel drags it; letting go
    /// leaves it there (on the right controller) and saves the spot.
    fn probe(&mut self) -> (Option<(f32, f32)>, Option<(f32, f32)>) {
        let Some((left, lp, pp)) = self.hands() else {
            self.move_cursor(None);
            self.grab = None;
            return (None, None);
        };
        let (trigger, grip, stick) = self.buttons(left);
        let (pressed, grabbed) = (trigger && !self.trigger_down, grip && !self.grip_down);
        (self.trigger_down, self.grip_down) = (trigger, grip);
        if let Some(off) = self.grab {
            if grip {
                self.mat = mul(&invert(&pp), &mul(&lp, &off));
                self.apply_place();
                if stick.abs() > 0.3 {
                    // ~2x bigger / smaller per second with the stick all the way
                    let w = (self.width * (1.0 + 0.025 * stick)).clamp(SIZE_RANGE.0, SIZE_RANGE.1);
                    self.set_width(w);
                }
                return (None, None);
            }
            self.grab = None;
            if self.attached {
                save_place(&self.mat, self.width);
                self.custom = true;
                markii_core::log::info("painel VR: posição nova salva");
            }
        }
        let panel = mul(&pp, &self.mat);
        let l = to_panel(&panel, apply(&lp, TIP));
        let over = (l[2] > -BEHIND && l[2] < CURSOR_NEAR).then(|| panel_pixel(l, self.width)).flatten();
        if grabbed && over.is_some() && self.attached {
            self.grab = Some(mul(&invert(&lp), &panel));
            self.move_cursor(None);
            return (None, None);
        }
        self.move_cursor(over.map(|_| l));
        (over, over.filter(|_| pressed))
    }

    /// Left trigger and grip, read like fpsVR / OVR Toolkit do (legacy state,
    /// so nothing is taken away from the game; the game sees the presses too).
    fn buttons(&self, left: u32) -> (bool, bool, f32) {
        let (size, pressed_at, axis_at) = STATE;
        let mut st = [0u8; 64];
        let ok = unsafe {
            let get: extern "C" fn(u32, *mut u8, u32) -> bool = std::mem::transmute(*self.sys.add(SYS_CONTROLLER_STATE));
            get(left, st.as_mut_ptr(), size as u32)
        };
        if !ok {
            return (false, false, 0.0);
        }
        let pressed = u64::from_le_bytes(st[pressed_at..pressed_at + 8].try_into().unwrap());
        let axis = |i: usize| f32::from_le_bytes(st[axis_at + i * 8..axis_at + i * 8 + 4].try_into().unwrap());
        // half-pulled counts while already held, so a shaky finger is one press
        let held = |was: bool, v: f32| v > if was { 0.4 } else { 0.75 };
        (
            pressed & BUTTON_TRIGGER != 0 || held(self.trigger_down, axis(0)),
            pressed & BUTTON_GRIP != 0 || held(self.grip_down, axis(1)),
            // rAxis[0].y: the stick, up = +1
            f32::from_le_bytes(st[axis_at - 4..axis_at].try_into().unwrap()),
        )
    }

    /// The left controller, its pose and the pose of the panel's parent.
    fn hands(&self) -> Option<(u32, Mat34, Mat34)> {
        let mut raw = vec![0u8; POSE_SIZE * MAX_DEVICES as usize];
        let left = unsafe {
            let role: extern "C" fn(i32) -> u32 = std::mem::transmute(*self.sys.add(SYS_CONTROLLER_FOR_ROLE));
            let poses: extern "C" fn(i32, f32, *mut u8, u32) = std::mem::transmute(*self.sys.add(SYS_POSES));
            poses(ORIGIN_STANDING, 0.0, raw.as_mut_ptr(), MAX_DEVICES);
            role(ROLE_LEFT_HAND)
        };
        if left == INVALID_DEVICE || left == self.parent {
            return None;
        }
        let pose = |i: u32| -> Option<Mat34> {
            let b = raw.get(i as usize * POSE_SIZE..(i as usize + 1) * POSE_SIZE)?;
            if b[76] == 0 {
                return None; // bPoseIsValid
            }
            let mut m = [[0f32; 4]; 3];
            for r in 0..3 {
                for c in 0..4 {
                    let o = (r * 4 + c) * 4;
                    m[r][c] = f32::from_le_bytes(b[o..o + 4].try_into().ok()?);
                }
            }
            Some(m)
        };
        Some((left, pose(left)?, pose(self.parent)?))
    }

    /// Puts the dot on the panel's face right under the tip, or hides it.
    fn move_cursor(&mut self, l: Option<[f32; 3]>) {
        if self.cursor == 0 {
            return;
        }
        unsafe {
            match l {
                Some(l) => {
                    let rel: extern "C" fn(u64, u32, *const Mat34) -> i32 = std::mem::transmute(*self.ov.add(OV_SET_DEVICE_RELATIVE));
                    let m = mul(&self.mat, &[[1.0, 0.0, 0.0, l[0]], [0.0, 1.0, 0.0, l[1]], [0.0, 0.0, 1.0, 0.004]]);
                    rel(self.cursor, self.parent, &m);
                    // fainter while far, solid when about to touch
                    let alpha: extern "C" fn(u64, f32) -> i32 = std::mem::transmute(*self.ov.add(OV_SET_ALPHA));
                    alpha(self.cursor, (1.0 - l[2].max(0.0) / CURSOR_NEAR * 0.6).clamp(0.4, 1.0));
                    if !self.cursor_on {
                        let show: extern "C" fn(u64) -> i32 = std::mem::transmute(*self.ov.add(OV_SHOW));
                        show(self.cursor);
                        self.cursor_on = true;
                    }
                }
                None if self.cursor_on => {
                    let hide: extern "C" fn(u64) -> i32 = std::mem::transmute(*self.ov.add(OV_HIDE));
                    hide(self.cursor);
                    self.cursor_on = false;
                }
                None => {}
            }
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
            if !self.ov.is_null() {
                let destroy: extern "C" fn(u64) -> i32 = std::mem::transmute(*self.ov.add(OV_DESTROY));
                for h in self.handles.iter_mut().chain(std::iter::once(&mut self.cursor)).filter(|h| **h != 0) {
                    destroy(*h);
                    *h = 0;
                }
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
    let cfg: Arc<Mutex<Cfg>> = Arc::default();
    let hist: Arc<Mutex<History>> = Arc::default();
    let alive = Arc::new(AtomicBool::new(true));
    {
        let (latest, cfg, hist, alive) = (latest.clone(), cfg.clone(), hist.clone(), alive.clone());
        std::thread::spawn(move || {
            for line in std::io::stdin().lock().lines() {
                let Ok(line) = line else { break };
                if let Some(c) = line.strip_prefix(CFG_MARK) {
                    *cfg.lock().unwrap() = Cfg::parse(c);
                } else if let Ok(s) = serde_json::from_str::<Snapshot>(&line) {
                    hist.lock().unwrap().push(s.cpu_temp, s.gpu_temp.map(|v| v as f64));
                    *latest.lock().unwrap() = Some(s);
                }
            }
            alive.store(false, Ordering::SeqCst); // Mark II is gone
        });
    }
    let mut ui = Ui::default();
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
                markii_core::log::info(if s.attached { "painel VR: preso ao pulso direito" } else { "painel VR: controle direito não encontrado (painel na frente do óculos)" });
            }
            if last_bat.elapsed() >= Duration::from_secs(10) {
                let b = s.batteries();
                println!("{BATTERY_MARK}{}", serde_json::to_string(&b).unwrap_or_else(|_| "[]".into()));
                ui.est.push(b.iter().find(|b| b.name == "Óculos"));
                ui.bats = b;
                last_bat = Instant::now();
            }
            let c = cfg.lock().unwrap().clone();
            s.check_reset();
            s.finish_swap();
            if ui.cfg_mode != c.mode {
                // set (or changed) in Mark II's settings
                ui.cfg_mode = c.mode.clone();
                ui.mode = c.mode.clone();
                last_draw -= Duration::from_secs(5);
            }
            let (over, tap) = s.probe();
            let hover = over.and_then(|(x, y)| ui.button_at(x, y));
            if hover != ui.hover {
                ui.hover = hover; // light up the button under the dot
                last_draw -= Duration::from_secs(5);
            }
            if let Some((x, y)) = tap {
                if let Some(m) = ui.click(x, y) {
                    println!("{MODE_MARK}{m}");
                    markii_core::log::info(format!("painel VR: modo {m}"));
                }
                last_draw -= Duration::from_secs(5); // show it right away
            }
            if ui.mode() == "hud" && ui.bar_until.is_some_and(|t| t <= Instant::now()) {
                ui.bar_until = None; // buttons hide again
                last_draw -= Duration::from_secs(5);
            }
            if last_draw.elapsed() >= Duration::from_millis(1000) {
                let snap = latest.lock().unwrap().clone();
                let h = hist.lock().unwrap().clone();
                let mut px = render(snap.as_ref(), font.as_ref(), &ui, &c, &h);
                s.upload(&mut px);
                last_draw = Instant::now();
            }
            // ~30 checks a second, so a quick touch is not missed
            std::thread::sleep(Duration::from_millis(33));
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

#[derive(Clone, Copy, PartialEq)]
struct Rgb(u8, u8, u8);

impl Rgb {
    fn hex(v: u32) -> Self {
        Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
    }
    /// `self` over `under` at `k` (0..1).
    fn over(self, under: Rgb, k: f32) -> Rgb {
        let m = |a: u8, b: u8| (a as f32 * k + b as f32 * (1.0 - k)).round() as u8;
        Rgb(m(self.0, under.0), m(self.1, under.1), m(self.2, under.2))
    }
}

/// The panel's colors, taken from the Mark II theme the user picked.
struct Pal {
    bg: Rgb,
    panel: Rgb,
    line: Rgb,
    dim: Rgb,
    muted: Rgb,
    text: Rgb,
    accent: Rgb,
    indigo: Rgb,
    green: Rgb,
    cyan: Rgb,
    red: Rgb,
}

impl Pal {
    fn of(theme: &str) -> Self {
        let t = crate::theme::THEMES.iter().find(|t| t.key == theme).unwrap_or(&crate::theme::THEMES[0]);
        let (g, a) = (t.grey, t.accent);
        Pal {
            bg: Rgb::hex(g[0]),
            panel: Rgb::hex(g[1]),
            line: Rgb::hex(g[3]),
            dim: Rgb::hex(g[5]),
            muted: Rgb::hex(g[6]),
            text: Rgb::hex(g[7]),
            accent: Rgb::hex(a[0]),
            indigo: Rgb::hex(a[1]),
            green: Rgb::hex(a[2]),
            cyan: Rgb::hex(a[3]),
            red: Rgb::hex(a[4]),
        }
    }

    fn temp(&self, t: Option<f64>, limit: f64) -> Rgb {
        match t {
            Some(v) if v >= limit => self.red,
            Some(v) if v >= limit - 12.0 => self.accent,
            Some(_) => self.green,
            None => self.muted,
        }
    }

    fn battery(&self, pct: f32, charging: bool) -> Rgb {
        if charging {
            self.cyan
        } else if pct <= 15.0 {
            self.red
        } else if pct <= 30.0 {
            self.accent
        } else {
            self.green
        }
    }
}

/// Settings from Mark II ("@cfg ..." lines).
#[derive(Clone)]
struct Cfg {
    mode: String,
    theme: String,
    cpu_limit: f64,
    gpu_limit: f64,
}

impl Default for Cfg {
    fn default() -> Self {
        Cfg { mode: "relogio".into(), theme: "grafite".into(), cpu_limit: 85.0, gpu_limit: 83.0 }
    }
}

impl Cfg {
    fn parse(s: &str) -> Self {
        let mut it = s.split_whitespace();
        let d = Cfg::default();
        let mode = it.next().filter(|m| MODES.iter().any(|x| x.0 == *m)).map(String::from).unwrap_or(d.mode);
        let theme = it.next().map(String::from).unwrap_or(d.theme);
        let cpu_limit = it.next().and_then(|v| v.parse().ok()).unwrap_or(d.cpu_limit);
        let gpu_limit = it.next().and_then(|v| v.parse().ok()).unwrap_or(d.gpu_limit);
        Cfg { mode, theme, cpu_limit, gpu_limit }
    }
}

/// CPU and GPU temperature over the last 5 minutes, one point per second at most.
#[derive(Clone, Default)]
struct History {
    pts: std::collections::VecDeque<(Instant, Option<f64>, Option<f64>)>,
}

const HIST_SECS: f32 = 300.0;

impl History {
    fn push(&mut self, cpu: Option<f64>, gpu: Option<f64>) {
        let now = Instant::now();
        if self.pts.back().is_some_and(|p| now.duration_since(p.0) < Duration::from_millis(900)) {
            return;
        }
        self.pts.push_back((now, cpu, gpu));
        while self.pts.front().is_some_and(|p| now.duration_since(p.0).as_secs_f32() > HIST_SECS) {
            self.pts.pop_front();
        }
    }
}

/// Headset battery over time, to guess how long it lasts.
#[derive(Default)]
struct BatteryEst {
    pts: std::collections::VecDeque<(Instant, f32)>,
}

impl BatteryEst {
    fn push(&mut self, b: Option<&VrBattery>) {
        let Some(b) = b else { return self.pts.clear() };
        if b.charging {
            return self.pts.clear();
        }
        let now = Instant::now();
        self.pts.push_back((now, b.pct));
        while self.pts.front().is_some_and(|p| now.duration_since(p.0) > Duration::from_secs(30 * 60)) {
            self.pts.pop_front();
        }
    }

    /// Minutes left, once it has dropped at least 2% over 5+ minutes.
    fn minutes_left(&self) -> Option<u32> {
        let (a, b) = (self.pts.front()?, self.pts.back()?);
        let mins = b.0.duration_since(a.0).as_secs_f32() / 60.0;
        let drop = a.1 - b.1;
        (mins >= 5.0 && drop >= 2.0).then(|| (b.1 / (drop / mins)).round() as u32)
    }
}

/// What the panel shows and where the buttons are.
#[derive(Default)]
struct Ui {
    /// Mode on screen ("" until Mark II says which).
    mode: String,
    /// Last mode received from Mark II's settings.
    cfg_mode: String,
    /// In HUD the buttons hide; a tap brings them back for a few seconds.
    bar_until: Option<Instant>,
    /// Button under the cursor dot.
    hover: Option<usize>,
    bats: Vec<VrBattery>,
    est: BatteryEst,
}

const BAR_Y: f32 = H as f32 - 52.0;
const BAR_H: f32 = 42.0;

fn bar_button(i: usize) -> (f32, f32, f32) {
    let w = (W as f32 - 24.0 - 3.0 * 8.0) / 4.0;
    (12.0 + i as f32 * (w + 8.0), BAR_Y, w)
}

impl Ui {
    fn mode(&self) -> &str {
        if self.mode.is_empty() { "relogio" } else { &self.mode }
    }

    fn bar_visible(&self) -> bool {
        self.mode() != "hud" || self.bar_until.is_some_and(|t| t > Instant::now())
    }

    fn button_at(&self, x: f32, y: f32) -> Option<usize> {
        if !self.bar_visible() || !(BAR_Y..=BAR_Y + BAR_H).contains(&y) {
            return None;
        }
        (0..4).find(|&i| {
            let (bx, _, bw) = bar_button(i);
            (bx..=bx + bw).contains(&x)
        })
    }

    /// A touch at (x, y). Returns the new mode when one was picked.
    fn click(&mut self, x: f32, y: f32) -> Option<&'static str> {
        if !self.bar_visible() {
            self.bar_until = Some(Instant::now() + Duration::from_secs(8));
            return None;
        }
        let i = self.button_at(x, y)?;
        let m = MODES[i].0;
        self.mode = m.to_string();
        self.bar_until = None;
        Some(m)
    }
}

struct Canvas {
    pm: tiny_skia::Pixmap,
}

impl Canvas {
    fn paint(c: Rgb, a: u8) -> tiny_skia::Paint<'static> {
        let mut p = tiny_skia::Paint::default();
        p.set_color_rgba8(c.0, c.1, c.2, a);
        p.anti_alias = true;
        p
    }

    fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, c: Rgb) {
        if let Some(r) = tiny_skia::Rect::from_xywh(x, y, w.max(0.5), h.max(0.5)) {
            self.pm.fill_rect(r, &Self::paint(c, 255), tiny_skia::Transform::identity(), None);
        }
    }

    fn rrect_path(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<tiny_skia::Path> {
        let r = r.min(w / 2.0).min(h / 2.0);
        let mut pb = tiny_skia::PathBuilder::new();
        pb.move_to(x + r, y);
        pb.line_to(x + w - r, y);
        pb.quad_to(x + w, y, x + w, y + r);
        pb.line_to(x + w, y + h - r);
        pb.quad_to(x + w, y + h, x + w - r, y + h);
        pb.line_to(x + r, y + h);
        pb.quad_to(x, y + h, x, y + h - r);
        pb.line_to(x, y + r);
        pb.quad_to(x, y, x + r, y);
        pb.close();
        pb.finish()
    }

    fn rrect(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, c: Rgb, a: u8) {
        if let Some(p) = Self::rrect_path(x, y, w, h, r) {
            self.pm.fill_path(&p, &Self::paint(c, a), tiny_skia::FillRule::Winding, tiny_skia::Transform::identity(), None);
        }
    }

    fn rrect_line(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, c: Rgb, width: f32) {
        if let Some(p) = Self::rrect_path(x, y, w, h, r) {
            let st = tiny_skia::Stroke { width, ..Default::default() };
            self.pm.stroke_path(&p, &Self::paint(c, 255), &st, tiny_skia::Transform::identity(), None);
        }
    }

    fn polyline(&mut self, pts: &[(f32, f32)], c: Rgb, width: f32, dash: bool) {
        if pts.len() < 2 {
            return;
        }
        let mut pb = tiny_skia::PathBuilder::new();
        pb.move_to(pts[0].0, pts[0].1);
        for p in &pts[1..] {
            pb.line_to(p.0, p.1);
        }
        let Some(path) = pb.finish() else { return };
        let mut st = tiny_skia::Stroke { width, line_cap: tiny_skia::LineCap::Round, line_join: tiny_skia::LineJoin::Round, ..Default::default() };
        if dash {
            st.dash = tiny_skia::StrokeDash::new(vec![7.0, 6.0], 0.0);
        }
        self.pm.stroke_path(&path, &Self::paint(c, 255), &st, tiny_skia::Transform::identity(), None);
    }

    /// A ring from 12 o'clock, clockwise, `frac` of the way round.
    fn arc(&mut self, cx: f32, cy: f32, r: f32, frac: f32, c: Rgb, width: f32) {
        let n = ((frac.clamp(0.0, 1.0) * 96.0).ceil() as usize).max(1);
        let pts: Vec<(f32, f32)> = (0..=n)
            .map(|i| {
                let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * frac.clamp(0.0, 1.0) * i as f32 / n as f32;
                (cx + r * a.cos(), cy + r * a.sin())
            })
            .collect();
        if frac > 0.004 {
            self.polyline(&pts, c, width, false);
        }
    }

    fn circle(&mut self, cx: f32, cy: f32, r: f32, c: Rgb) {
        if let Some(p) = tiny_skia::PathBuilder::from_circle(cx, cy, r) {
            self.pm.fill_path(&p, &Self::paint(c, 255), tiny_skia::FillRule::Winding, tiny_skia::Transform::identity(), None);
        }
    }

    fn measure(font: Option<&ab_glyph::FontVec>, s: &str, size: f32) -> f32 {
        use ab_glyph::{Font, ScaleFont};
        let Some(font) = font else { return 0.0 };
        let sf = font.as_scaled(size);
        let mut w = 0.0;
        let mut prev = None;
        for ch in s.chars() {
            let id = sf.glyph_id(ch);
            if let Some(p) = prev {
                w += sf.kern(p, id);
            }
            w += sf.h_advance(id);
            prev = Some(id);
        }
        w
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
                    data[i + 3] = data[i + 3].max((a * 255.0) as u8);
                });
            }
            pen += sf.h_advance(id);
            prev = Some(id);
        }
        pen - x
    }

    fn text_center(&mut self, font: Option<&ab_glyph::FontVec>, s: &str, cx: f32, y: f32, size: f32, c: Rgb) {
        let w = Self::measure(font, s, size);
        self.text(font, s, cx - w / 2.0, y, size, c);
    }

    fn text_right(&mut self, font: Option<&ab_glyph::FontVec>, s: &str, right: f32, y: f32, size: f32, c: Rgb) {
        let w = Self::measure(font, s, size);
        self.text(font, s, right - w, y, size, c);
    }

    /// Pieces of text in a row, each with its color; returns the width.
    fn spans(&mut self, font: Option<&ab_glyph::FontVec>, parts: &[(String, Rgb)], x: f32, y: f32, size: f32) -> f32 {
        let mut pen = x;
        for (s, c) in parts {
            pen += self.text(font, s, pen, y, size, *c);
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

/// ("21:47", "sex · 20 nov") from the local clock.
fn clock() -> (String, String) {
    let now = markii_core::log::now(); // "2026-11-20 21:47:05"
    let num = |a: usize, b: usize| now.get(a..b).and_then(|v| v.parse::<i32>().ok()).unwrap_or(0);
    let (y, m, d) = (num(0, 4), num(5, 7), num(8, 10));
    // Sakamoto: 0 = Sunday
    let t = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let yy = if m < 3 { y - 1 } else { y };
    let wd = (yy + yy / 4 - yy / 100 + yy / 400 + t[((m - 1).clamp(0, 11)) as usize] + d).rem_euclid(7);
    let days = ["dom", "seg", "ter", "qua", "qui", "sex", "sáb"];
    let months = ["jan", "fev", "mar", "abr", "mai", "jun", "jul", "ago", "set", "out", "nov", "dez"];
    (now.get(11..16).unwrap_or("--:--").to_string(), format!("{} · {d} {}", days[wd as usize], months[((m - 1).clamp(0, 11)) as usize]))
}

fn short_name(n: &str) -> &str {
    match n {
        "Controle esquerdo" => "E",
        "Controle direito" => "D",
        "Controle" => "C",
        "Rastreador" => "R",
        n => n,
    }
}

const STRIP_Y: f32 = 288.0; // alert / info line above the buttons

/// The wrist panel as RGBA bytes (W x H).
fn render(s: Option<&Snapshot>, font: Option<&ab_glyph::FontVec>, ui: &Ui, cfg: &Cfg, hist: &History) -> Vec<u8> {
    let p = Pal::of(&cfg.theme);
    let mut cv = Canvas { pm: tiny_skia::Pixmap::new(W, H).unwrap() };
    let hud = ui.mode() == "hud";
    if !hud {
        cv.pm.fill(tiny_skia::Color::from_rgba8(p.bg.0, p.bg.1, p.bg.2, 255));
        cv.rect(0.0, 0.0, W as f32, 4.0, p.accent);
        // logo, top center
        let (a, b) = (Canvas::measure(font, "MARK ", 24.0), Canvas::measure(font, "II", 24.0));
        let x = (W as f32 - a - b) / 2.0;
        cv.text(font, "MARK ", x, 36.0, 24.0, p.text);
        cv.text(font, "II", x + a, 36.0, 24.0, p.accent);
    }
    match s {
        None if !hud => cv.text_center(font, "aguardando dados do Mark II…", W as f32 / 2.0, 170.0, 22.0, p.muted),
        None => {}
        Some(s) => match ui.mode() {
            "grafico" => draw_graph(&mut cv, font, &p, s, cfg, hist, ui),
            "baterias" => draw_batteries(&mut cv, font, &p, s, cfg, ui),
            "hud" => draw_hud(&mut cv, font, &p, s, cfg, ui),
            _ => draw_clock(&mut cv, font, &p, s, cfg, ui),
        },
    }
    if !hud {
        if let Some(a) = s.and_then(|s| s.alerts.first()) {
            cv.rrect(14.0, STRIP_Y, W as f32 - 28.0, 28.0, 6.0, p.red.over(p.bg, 0.2), 255);
            cv.text(font, &format!("⚠ {a}"), 26.0, STRIP_Y + 20.0, 17.0, p.red);
        }
    }
    if ui.bar_visible() {
        if hud {
            cv.rrect(6.0, BAR_Y - 6.0, W as f32 - 12.0, BAR_H + 12.0, 12.0, p.bg, 235);
        }
        for (i, (key, label)) in MODES.iter().enumerate() {
            let (x, y, w) = bar_button(i);
            let on = ui.mode() == *key;
            cv.rrect(x, y, w, BAR_H, 9.0, if on { p.accent.over(p.panel, 0.18) } else { p.panel }, 255);
            if on {
                cv.rrect_line(x + 1.0, y + 1.0, w - 2.0, BAR_H - 2.0, 9.0, p.accent, 2.0);
            }
            if ui.hover == Some(i) {
                cv.rrect(x, y, w, BAR_H, 9.0, p.text, 40);
                cv.rrect_line(x + 1.0, y + 1.0, w - 2.0, BAR_H - 2.0, 9.0, p.text, 2.0);
            }
            cv.text_center(font, label, x + w / 2.0, y + 28.0, 18.0, if on { p.accent } else { p.muted });
        }
    }
    cv.pm.take()
}

fn draw_clock(cv: &mut Canvas, font: Option<&ab_glyph::FontVec>, p: &Pal, s: &Snapshot, cfg: &Cfg, ui: &Ui) {
    let (cx, cy) = (138.0, 168.0);
    cv.circle(cx, cy, 116.0, p.panel);
    cv.arc(cx, cy, 104.0, 1.0, p.line, 11.0);
    cv.arc(cx, cy, 104.0, s.cpu_load / 100.0, p.indigo, 11.0);
    cv.arc(cx, cy, 86.0, 1.0, p.line, 11.0);
    cv.arc(cx, cy, 86.0, s.gpu_load.unwrap_or(0) as f32 / 100.0, p.green, 11.0);
    let (time, date) = clock();
    cv.text_center(font, &time, cx, cy + 14.0, 44.0, p.text);
    cv.text_center(font, &date, cx, cy + 40.0, 16.0, p.muted);

    let (x0, x1) = (284.0, W as f32 - 22.0);
    let rows: [(&str, Rgb, String, Rgb); 3] = [
        ("CPU", p.indigo, format!("{:.0}% · {}", s.cpu_load, opt(s.cpu_temp.map(|v| format!("{v:.0}")), "°C")), p.temp(s.cpu_temp, cfg.cpu_limit)),
        ("GPU", p.green, format!("{} · {}", opt(s.gpu_load, "%"), opt(s.gpu_temp, "°C")), p.temp(s.gpu_temp.map(|v| v as f64), cfg.gpu_limit)),
        ("RAM", p.accent, format!("{} / {} GB", gb(s.ram_used), gb(s.ram_total)), p.text),
    ];
    let mut y = 96.0;
    for (name, col, v, vcol) in rows {
        cv.text(font, name, x0, y, 20.0, col);
        cv.text_right(font, &v, x1, y, 22.0, vcol);
        y += 40.0;
    }
    cv.rect(x0, 200.0, x1 - x0, 1.5, p.line);
    let bats: Vec<(String, Rgb)> = ui
        .bats
        .iter()
        .take(4)
        .flat_map(|b| {
            [
                (format!("{} ", if b.name == "Óculos" { "Óculos" } else { short_name(&b.name) }), p.muted),
                (format!("{:.0}%{}   ", b.pct, if b.charging { "+" } else { "" }), p.battery(b.pct, b.charging)),
            ]
        })
        .collect();
    if bats.is_empty() {
        cv.text(font, "sem baterias no SteamVR", x0, 234.0, 17.0, p.dim);
    } else {
        cv.spans(font, &bats, x0, 234.0, 17.0);
    }
    if s.alerts.is_empty() {
        let fans = s.fans.iter().take(3).map(|(n, r)| format!("{} {:.0} rpm", n.split_whitespace().next().unwrap_or(n), r)).collect::<Vec<_>>().join("  ");
        cv.text(font, if fans.is_empty() { "sem alertas" } else { &fans }, x0, 264.0, 16.0, if fans.is_empty() { p.green } else { p.cyan });
    }
}

fn draw_graph(cv: &mut Canvas, font: Option<&ab_glyph::FontVec>, p: &Pal, s: &Snapshot, cfg: &Cfg, hist: &History, ui: &Ui) {
    let (x0, x1) = (22.0, W as f32 - 22.0);
    cv.spans(font, &[("CPU ".into(), p.indigo), (opt(s.cpu_temp.map(|v| format!("{v:.0}")), "°C"), p.temp(s.cpu_temp, cfg.cpu_limit))], x0, 78.0, 20.0);
    cv.spans(font, &[("GPU ".into(), p.green), (opt(s.gpu_temp, "°C"), p.temp(s.gpu_temp.map(|v| v as f64), cfg.gpu_limit))], x0 + 170.0, 78.0, 20.0);
    cv.text_right(font, "últimos 5 min", x1, 78.0, 16.0, p.muted);

    let (gy0, gy1) = (92.0, 262.0);
    cv.rrect(x0, gy0, x1 - x0, gy1 - gy0, 8.0, p.panel, 255);
    let (tmin, tmax) = (30.0f64, 100.0f64);
    let ty = |t: f64| gy1 - 8.0 - ((t.clamp(tmin, tmax) - tmin) / (tmax - tmin)) as f32 * (gy1 - gy0 - 16.0);
    for t in [40.0, 60.0, 80.0] {
        cv.rect(x0 + 8.0, ty(t), x1 - x0 - 16.0, 1.0, p.line);
        cv.text(font, &format!("{t:.0}°"), x0 + 10.0, ty(t) - 4.0, 13.0, p.dim);
    }
    let limit = cfg.cpu_limit.min(cfg.gpu_limit);
    cv.polyline(&[(x0 + 8.0, ty(limit)), (x1 - 8.0, ty(limit))], p.red, 1.5, true);
    cv.text_right(font, &format!("limite {limit:.0}°C"), x1 - 10.0, ty(limit) - 6.0, 13.0, p.red);
    let now = Instant::now();
    let tx = |t: Instant| x1 - 8.0 - (now.duration_since(t).as_secs_f32() / HIST_SECS).min(1.0) * (x1 - x0 - 16.0);
    let line = |f: &dyn Fn(&(Instant, Option<f64>, Option<f64>)) -> Option<f64>| -> Vec<(f32, f32)> { hist.pts.iter().filter_map(|pt| f(pt).map(|v| (tx(pt.0), ty(v)))).collect() };
    cv.polyline(&line(&|pt| pt.2), p.green, 3.0, false);
    cv.polyline(&line(&|pt| pt.1), p.indigo, 3.0, false);
    if hist.pts.len() < 3 {
        cv.text_center(font, "juntando dados…", (x0 + x1) / 2.0, (gy0 + gy1) / 2.0, 17.0, p.dim);
    }
    if s.alerts.is_empty() {
        let vram = match (s.vram_used, s.vram_total) {
            (Some(u), Some(t)) if t > 0 => format!("VRAM {}/{} GB", gb(u), gb(t)),
            _ => String::new(),
        };
        let hmd = ui.bats.iter().find(|b| b.name == "Óculos").map(|b| format!("Óculos {:.0}%", b.pct)).unwrap_or_default();
        let parts = [format!("RAM {:.0}%", s.ram_pct()), vram, hmd].into_iter().filter(|v| !v.is_empty()).collect::<Vec<_>>().join("     ");
        cv.text(font, &parts, x0, STRIP_Y + 20.0, 16.0, p.muted);
    }
}

fn draw_batteries(cv: &mut Canvas, font: Option<&ab_glyph::FontVec>, p: &Pal, s: &Snapshot, cfg: &Cfg, ui: &Ui) {
    let bats: Vec<&VrBattery> = ui.bats.iter().take(4).collect();
    if bats.is_empty() {
        cv.text_center(font, "nenhum aparelho com bateria no SteamVR", W as f32 / 2.0, 150.0, 19.0, p.muted);
    }
    let n = bats.len().max(1) as f32;
    let colw = (W as f32 - 32.0) / n;
    for (i, b) in bats.iter().enumerate() {
        let cx = 16.0 + colw * (i as f32 + 0.5);
        let col = p.battery(b.pct, b.charging);
        let name = match b.name.as_str() {
            "Controle esquerdo" => "Esquerdo",
            "Controle direito" => "Direito",
            n => n,
        };
        cv.text_center(font, name, cx, 84.0, 17.0, p.muted);
        // battery icon
        let (bw, bh) = (84.0, 36.0);
        let (bx, by) = (cx - bw / 2.0 - 3.0, 98.0);
        cv.rrect_line(bx, by, bw, bh, 6.0, p.dim, 2.5);
        cv.rrect(bx + bw + 1.0, by + 11.0, 6.0, 14.0, 2.0, p.dim, 255);
        cv.rrect(bx + 5.0, by + 5.0, (bw - 10.0) * (b.pct / 100.0).clamp(0.03, 1.0), bh - 10.0, 3.0, col, 255);
        cv.text_center(font, &format!("{:.0}%", b.pct), cx, 184.0, 36.0, col);
        let sub = if b.charging {
            "carregando".to_string()
        } else if b.name == "Óculos" {
            ui.est.minutes_left().map(|m| format!("~{}h{:02} restantes", m / 60, m % 60)).unwrap_or_default()
        } else {
            String::new()
        };
        cv.text_center(font, &sub, cx, 208.0, 15.0, p.muted);
    }
    // the PC in one line
    cv.rrect(14.0, 228.0, W as f32 - 28.0, 46.0, 8.0, p.panel, 255);
    let items = [
        ("CPU", opt(s.cpu_temp.map(|v| format!("{v:.0}")), "°C"), p.temp(s.cpu_temp, cfg.cpu_limit)),
        ("GPU", opt(s.gpu_temp, "°C"), p.temp(s.gpu_temp.map(|v| v as f64), cfg.gpu_limit)),
        ("RAM", format!("{:.0}%", s.ram_pct()), p.text),
    ];
    for (i, (k, v, c)) in items.iter().enumerate() {
        let cx = 14.0 + (W as f32 - 28.0) * (i as f32 * 2.0 + 1.0) / 6.0;
        let w = Canvas::measure(font, &format!("{k} {v}"), 20.0);
        cv.spans(font, &[(format!("{k} "), p.muted), (v.clone(), *c)], cx - w / 2.0, 258.0, 20.0);
    }
}

fn draw_hud(cv: &mut Canvas, font: Option<&ab_glyph::FontVec>, p: &Pal, s: &Snapshot, cfg: &Cfg, ui: &Ui) {
    // only the pill; the rest stays see-through
    let (x, y, w, h) = (14.0, 120.0, W as f32 - 28.0, 72.0);
    let alert = !s.alerts.is_empty();
    cv.rrect(x, y, w, h, h / 2.0, p.bg, 230);
    cv.rrect_line(x + 1.0, y + 1.0, w - 2.0, h - 2.0, h / 2.0, if alert { p.red } else { p.line }, 2.0);
    let hmd = ui.bats.iter().find(|b| b.name == "Óculos");
    let mut items: Vec<(String, Rgb)> = vec![
        (if alert { "⚠".into() } else { "II".into() }, if alert { p.red } else { p.accent }),
        (format!("CPU {}", opt(s.cpu_temp.map(|v| format!("{v:.0}")), "°")), p.temp(s.cpu_temp, cfg.cpu_limit)),
        (format!("GPU {}", opt(s.gpu_temp, "°")), p.temp(s.gpu_temp.map(|v| v as f64), cfg.gpu_limit)),
        (format!("RAM {:.0}%", s.ram_pct()), p.text),
    ];
    if let Some(b) = hmd {
        items.push((format!("{:.0}%", b.pct), p.battery(b.pct, b.charging)));
    }
    let n = items.len() as f32;
    for (i, (t, c)) in items.iter().enumerate() {
        let cx = x + 30.0 + (w - 60.0) * (i as f32 + 0.5) / n;
        cv.text_center(font, t, cx, y + h / 2.0 + 9.0, 25.0, *c);
    }
}

/// Writes each mode of the panel to a PNG (for checking the drawing without a
/// headset): `out.png` becomes `out-relogio.png`, `out-grafico.png`...
pub fn preview_png(path: &std::path::Path, s: &Snapshot) -> Result<(), String> {
    let font = load_font();
    let mut hist = History::default();
    let t0 = Instant::now();
    for i in 0..300 {
        let k = i as f64 / 299.0;
        hist.pts.push_back((t0 - Duration::from_secs(300 - i), Some(52.0 + 12.0 * k + 3.0 * (k * 19.0).sin()), Some(45.0 + 33.0 * k.sqrt() + 2.0 * (k * 23.0).cos())));
    }
    let mut ui = Ui { bats: s.vr_batteries.clone(), ..Default::default() };
    for (k, m) in [(0.0, 70.0), (20.0, 66.0)] {
        ui.est.pts.push_back((t0 - Duration::from_secs_f64(1200.0 - k * 60.0), m));
    }
    let theme = std::env::var("MARKII_VR_THEME").unwrap_or_else(|_| "grafite".into());
    for (mode, _) in MODES {
        ui.mode = mode.into();
        let cfg = Cfg { mode: mode.into(), theme: theme.clone(), ..Default::default() };
        let px = render(Some(s), font.as_ref(), &ui, &cfg, &hist);
        let stem = path.file_stem().map(|v| v.to_string_lossy().to_string()).unwrap_or_else(|| "vr-preview".into());
        let out = path.with_file_name(format!("{stem}-{mode}.png"));
        let f = std::fs::File::create(&out).map_err(|e| e.to_string())?;
        let mut enc = png::Encoder::new(std::io::BufWriter::new(f), W, H);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().and_then(|mut w| w.write_image_data(&px)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touch_maps_to_pixels() {
        // right controller somewhere in the room, turned 90 degrees around Y
        let ctrl: Mat34 = [[0.0, 0.0, 1.0, 0.3], [0.0, 1.0, 0.0, 1.1], [-1.0, 0.0, 0.0, -0.4]];
        let panel = mul(&ctrl, &wrist_mat(TILT_DEFAULT, ARM_DEFAULT, 35.0));
        // center of the panel, 1 cm above its face
        let p = apply(&panel, [0.0, 0.0, 0.01]);
        let l = to_panel(&panel, p);
        assert!(l[2] > 0.0099 && l[2] < 0.0101);
        let (x, y) = panel_pixel(l, PANEL_W).unwrap();
        assert!((x - W as f32 / 2.0).abs() < 0.5 && (y - H as f32 / 2.0).abs() < 0.5);
        // near the bottom left corner of the picture -> near (0, H)
        let (x, y) = panel_pixel(to_panel(&panel, apply(&panel, [-PANEL_W * 0.49, -PANEL_H * 0.49, 0.0])), PANEL_W).unwrap();
        assert!((x - W as f32 * 0.01).abs() < 0.5 && (y - H as f32 * 0.99).abs() < 0.5);
        // off the panel
        assert!(panel_pixel(to_panel(&panel, apply(&panel, [PANEL_W, 0.0, 0.0])), PANEL_W).is_none());
    }
}
