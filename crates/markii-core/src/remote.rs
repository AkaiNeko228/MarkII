//! Read-only access for the Mark II Android app over the local network.
//!
//! Off by default, and it only runs while the Mark II window is open (it lives in
//! the app process). A phone or headset pairs once with the 6-digit PIN shown in
//! Mark II and gets a random token; every read needs that token. The server only
//! answers three requests and never accepts commands:
//!
//!   GET  /v1/hello      -> {"app":"markii","v":1,"host":...}      (no token)
//!   POST /v1/pair       {"pin":"123456","device":"Quest 3"} -> {"token":...}
//!   GET  /v1/snapshot   Authorization: Bearer <token> -> Snapshot JSON
//!
//! Wrong PINs are limited: after 5 the PIN changes and pairing pauses for 30 s.

use crate::snapshot::Snapshot;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

const MAX_REQUEST: usize = 8 * 1024;
const MAX_TOKENS: usize = 8;

type OnTokens = Box<dyn Fn(Vec<String>) + Send + Sync>;

#[derive(Default)]
struct Inner {
    pin: String,
    tokens: Vec<String>,
    snapshot: Option<Snapshot>,
    fails: u32,
    locked_until: Option<Instant>,
    port: u16,
    address: String,
    error: Option<String>,
    last_client: Option<(String, Instant)>,
    /// What each paired device reported about itself: (token, data, when).
    devices: Vec<(String, Device, Instant)>,
}

/// What the phone / headset app reports about itself (sent with each refresh).
/// Every field is optional: the app sends what its platform lets it read.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Device {
    pub name: String,
    /// Battery, 0-100.
    pub battery: Option<f64>,
    pub charging: bool,
    pub battery_temp: Option<f64>,
    pub ram_used: Option<u64>,
    pub ram_total: Option<u64>,
    pub storage_free: Option<u64>,
    pub storage_total: Option<u64>,
    /// Wi-Fi signal in dBm and link speed in Mbit/s.
    pub wifi_rssi: Option<i32>,
    pub wifi_mbps: Option<i32>,
    /// Android thermal status: 0 none, 1 light, 2 moderate, 3 severe, 4 critical, 5 emergency, 6 shutdown.
    pub thermal: Option<i32>,
    pub cpu_temp: Option<f64>,
    pub uptime: Option<u64>,
}

impl Device {
    /// Keeps a report from the network sane before showing it.
    fn clean(mut self, fallback: &str) -> Self {
        self.name = self.name.chars().filter(|c| !c.is_control()).take(40).collect();
        if self.name.trim().is_empty() {
            self.name = fallback.to_string();
        }
        let fin = |v: Option<f64>, lo: f64, hi: f64| v.filter(|x| x.is_finite() && *x >= lo && *x <= hi);
        self.battery = fin(self.battery, 0.0, 100.0);
        self.battery_temp = fin(self.battery_temp, -40.0, 150.0);
        self.cpu_temp = fin(self.cpu_temp, -40.0, 150.0);
        self.thermal = self.thermal.filter(|t| (0..=6).contains(t));
        self.wifi_rssi = self.wifi_rssi.filter(|r| (-127..=0).contains(r));
        self
    }
}

/// How long a device report stays on screen after the app stops sending.
const DEVICE_TTL: Duration = Duration::from_secs(8);

pub struct Remote {
    inner: Arc<Mutex<Inner>>,
    stop: Mutex<Option<Arc<AtomicBool>>>,
    on_tokens: Mutex<Option<Arc<OnTokens>>>,
}

pub fn global() -> &'static Remote {
    static R: OnceLock<Remote> = OnceLock::new();
    R.get_or_init(|| Remote { inner: Arc::default(), stop: Mutex::new(None), on_tokens: Mutex::new(None) })
}

fn random_bytes<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    getrandom::getrandom(&mut b).expect("system RNG");
    b
}

fn new_pin() -> String {
    let n = u32::from_le_bytes(random_bytes::<4>()) % 1_000_000;
    format!("{n:06}")
}

fn new_token() -> String {
    random_bytes::<24>().iter().map(|b| format!("{b:02x}")).collect()
}

/// Constant-time compare, so a token cannot be guessed byte by byte from timing.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// This PC's address on the local network (no packet is sent).
pub fn local_ip() -> Option<String> {
    let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("8.8.8.8:80").ok()?;
    Some(s.local_addr().ok()?.ip().to_string())
}

#[derive(Clone, Debug, Default)]
pub struct Info {
    pub running: bool,
    pub pin: String,
    pub address: String,
    pub paired: usize,
    pub error: Option<String>,
    /// Device that read data in the last few seconds.
    pub connected: Option<String>,
}

impl Remote {
    /// Starts listening (no-op when already running). `tokens` are the devices
    /// paired before; `on_tokens` is called to persist the list when it changes.
    pub fn start(&self, port: u16, tokens: Vec<String>, on_tokens: OnTokens) {
        if self.running() {
            return;
        }
        {
            let mut i = self.inner.lock().unwrap();
            i.pin = new_pin();
            i.tokens = tokens;
            i.port = port;
            i.address = format!("{}:{port}", local_ip().unwrap_or_else(|| "?".into()));
            i.fails = 0;
            i.error = None;
        }
        *self.on_tokens.lock().unwrap() = Some(Arc::new(on_tokens));
        // a listener that was just stopped may hold the port for a moment
        let mut bound = TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], port)));
        for _ in 0..10 {
            if bound.is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
            bound = TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], port)));
        }
        let listener = match bound {
            Ok(l) => l,
            Err(e) => {
                crate::log::error(format!("rede local: não consegui abrir a porta {port}: {e}"));
                self.inner.lock().unwrap().error = Some(format!("porta {port}: {e}"));
                return;
            }
        };
        let _ = listener.set_nonblocking(true);
        crate::log::info(format!("rede local ligada em {}", self.inner.lock().unwrap().address));
        let stop = Arc::new(AtomicBool::new(false));
        *self.stop.lock().unwrap() = Some(stop.clone());
        let inner = self.inner.clone();
        let on = self.on_tokens.lock().unwrap().clone();
        std::thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((s, addr)) => {
                        let (inner, on) = (inner.clone(), on.clone());
                        std::thread::spawn(move || {
                            let _ = handle(s, addr, &inner, on.as_deref());
                        });
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(80)),
                }
            }
        });
    }

    pub fn stop(&self) {
        if let Some(s) = self.stop.lock().unwrap().take() {
            s.store(true, Ordering::SeqCst);
            crate::log::info("rede local desligada");
        }
        let mut i = self.inner.lock().unwrap();
        i.snapshot = None;
        i.pin.clear();
        i.devices.clear();
    }

    pub fn running(&self) -> bool {
        self.stop.lock().unwrap().is_some()
    }

    pub fn publish(&self, s: Snapshot) {
        if self.running() {
            self.inner.lock().unwrap().snapshot = Some(s);
        }
    }

    /// Forgets every paired device.
    pub fn unpair_all(&self) {
        {
            let mut i = self.inner.lock().unwrap();
            i.tokens.clear();
            i.devices.clear();
        }
        crate::log::info("rede local: todos os aparelhos foram desconectados");
        if let Some(f) = self.on_tokens.lock().unwrap().as_ref() {
            f(Vec::new());
        }
    }

    /// Devices that reported in the last few seconds (phones, headsets).
    pub fn devices(&self) -> Vec<Device> {
        let i = self.inner.lock().unwrap();
        i.devices.iter().filter(|(_, _, t)| t.elapsed() < DEVICE_TTL).map(|(_, d, _)| d.clone()).collect()
    }

    pub fn info(&self) -> Info {
        let i = self.inner.lock().unwrap();
        Info {
            running: self.running(),
            pin: i.pin.clone(),
            address: i.address.clone(),
            paired: i.tokens.len(),
            error: i.error.clone(),
            connected: i.last_client.as_ref().filter(|(_, t)| t.elapsed() < Duration::from_secs(5)).map(|(n, _)| n.clone()),
        }
    }
}

#[derive(Deserialize)]
struct PairReq {
    pin: String,
    #[serde(default)]
    device: String,
}

fn respond(s: &mut TcpStream, code: u16, body: &str) -> std::io::Result<()> {
    let reason = match code {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        429 => "Too Many Requests",
        _ => "Error",
    };
    let msg = format!(
        "HTTP/1.1 {code} {reason}\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    s.write_all(msg.as_bytes())
}

fn handle(mut s: TcpStream, addr: SocketAddr, inner: &Mutex<Inner>, on_tokens: Option<&OnTokens>) -> std::io::Result<()> {
    // local network only
    let local = match addr.ip() {
        std::net::IpAddr::V4(ip) => ip.is_private() || ip.is_loopback() || ip.is_link_local(),
        std::net::IpAddr::V6(ip) => ip.is_loopback(),
    };
    if !local {
        crate::log::warn(format!("rede local: recusado {} (fora da rede local)", addr.ip()));
        return respond(&mut s, 403, r#"{"error":"somente rede local"}"#);
    }
    s.set_read_timeout(Some(Duration::from_secs(3)))?;
    s.set_write_timeout(Some(Duration::from_secs(3)))?;
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    // read headers (+ a small body)
    let (head_end, body_len) = loop {
        let n = s.read(&mut chunk)?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.len() > MAX_REQUEST {
            return respond(&mut s, 400, r#"{"error":"grande demais"}"#);
        }
        if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..p]).to_string();
            let len = head
                .lines()
                .find_map(|l| l.split_once(':').filter(|(k, _)| k.trim().eq_ignore_ascii_case("content-length")).map(|(_, v)| v.trim().parse::<usize>().unwrap_or(0)))
                .unwrap_or(0);
            if len > MAX_REQUEST {
                return respond(&mut s, 400, r#"{"error":"grande demais"}"#);
            }
            // some clients wait for this before sending the body
            if len > 0 && head.to_ascii_lowercase().contains("expect: 100-continue") {
                s.write_all(b"HTTP/1.1 100 Continue\r\n\r\n")?;
            }
            break (p + 4, len);
        }
    };
    while buf.len() < head_end + body_len {
        let n = s.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let body = String::from_utf8_lossy(&buf[head_end..(head_end + body_len).min(buf.len())]).to_string();
    let mut first = head.lines().next().unwrap_or("").split_whitespace();
    let (method, path) = (first.next().unwrap_or(""), first.next().unwrap_or(""));
    let bearer = head.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        k.trim().eq_ignore_ascii_case("authorization").then(|| v.trim().strip_prefix("Bearer ").unwrap_or("").trim().to_string())
    });

    match (method, path) {
        ("GET", "/v1/hello") => {
            let host = inner.lock().unwrap().snapshot.as_ref().map(|x| x.host.clone()).unwrap_or_default();
            respond(&mut s, 200, &serde_json::json!({"app": "markii", "v": 1, "host": host}).to_string())
        }
        ("POST", "/v1/pair") => {
            let Ok(req) = serde_json::from_str::<PairReq>(&body) else {
                return respond(&mut s, 400, r#"{"error":"pedido inválido"}"#);
            };
            let mut i = inner.lock().unwrap();
            if i.locked_until.is_some_and(|t| Instant::now() < t) {
                return respond(&mut s, 429, r#"{"error":"muitas tentativas, aguarde 30 s"}"#);
            }
            if i.pin.is_empty() || !same(req.pin.trim(), &i.pin) {
                i.fails += 1;
                if i.fails >= 5 {
                    i.fails = 0;
                    i.pin = new_pin();
                    i.locked_until = Some(Instant::now() + Duration::from_secs(30));
                    crate::log::warn(format!("rede local: 5 PINs errados de {}; PIN trocado e pausa de 30 s", addr.ip()));
                } else {
                    crate::log::warn(format!("rede local: PIN errado de {}", addr.ip()));
                }
                return respond(&mut s, 401, r#"{"error":"PIN incorreto"}"#);
            }
            let token = new_token();
            i.tokens.push(token.clone());
            while i.tokens.len() > MAX_TOKENS {
                i.tokens.remove(0);
            }
            i.fails = 0;
            i.pin = new_pin(); // one PIN, one device
            let device: String = req.device.chars().filter(|c| !c.is_control()).take(40).collect();
            i.last_client = Some((if device.is_empty() { addr.ip().to_string() } else { device }, Instant::now()));
            let list = i.tokens.clone();
            crate::log::info(format!("rede local: pareado {} ({})", i.last_client.as_ref().map(|c| c.0.as_str()).unwrap_or("?"), addr.ip()));
            drop(i);
            if let Some(f) = on_tokens {
                f(list);
            }
            respond(&mut s, 200, &serde_json::json!({"token": token}).to_string())
        }
        // GET: read the PC. POST: same, and the body tells the PC about the device itself.
        ("GET", "/v1/snapshot") | ("POST", "/v1/snapshot") => {
            let mut i = inner.lock().unwrap();
            let token = bearer.as_deref().and_then(|b| (!b.is_empty()).then(|| i.tokens.iter().find(|t| same(t, b)).cloned()).flatten());
            let Some(token) = token else {
                crate::log::warn(format!("rede local: pedido sem pareamento válido de {}", addr.ip()));
                return respond(&mut s, 401, r#"{"error":"não pareado"}"#);
            };
            let mut name = i.last_client.as_ref().map(|(n, _)| n.clone()).unwrap_or_else(|| addr.ip().to_string());
            if method == "POST" {
                if let Ok(d) = serde_json::from_str::<Device>(&body) {
                    let d = d.clean(&addr.ip().to_string());
                    name = d.name.clone();
                    let known = i.devices.iter().any(|(t, _, at)| t == &token && at.elapsed() < DEVICE_TTL);
                    if !known {
                        crate::log::info(format!(
                            "rede local: {} conectado ({}) · bateria {}",
                            d.name,
                            addr.ip(),
                            d.battery.map_or("?".into(), |b| format!("{b:.0}%"))
                        ));
                    }
                    i.devices.retain(|(t, _, at)| t != &token && at.elapsed() < DEVICE_TTL);
                    i.devices.push((token, d, Instant::now()));
                }
            }
            i.last_client = Some((name, Instant::now()));
            let body = i.snapshot.as_ref().map(|x| serde_json::to_string(x).unwrap_or_default()).unwrap_or_else(|| "{}".into());
            drop(i);
            respond(&mut s, 200, &body)
        }
        _ => respond(&mut s, 404, r#"{"error":"não encontrado"}"#),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn http(port: u16, req: &str) -> String {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.write_all(req.as_bytes()).unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        out
    }

    #[test]
    fn pair_and_read() {
        let r = Remote { inner: Arc::default(), stop: Mutex::new(None), on_tokens: Mutex::new(None) };
        let saved: Arc<Mutex<Vec<String>>> = Arc::default();
        let s2 = saved.clone();
        let port = 47899;
        r.start(port, vec![], Box::new(move |t| *s2.lock().unwrap() = t));
        r.publish(Snapshot { host: "PC".into(), cpu_load: 12.0, ..Default::default() });
        std::thread::sleep(Duration::from_millis(200));
        assert!(http(port, "GET /v1/hello HTTP/1.1\r\n\r\n").contains("\"markii\""));
        assert!(http(port, "GET /v1/snapshot HTTP/1.1\r\n\r\n").starts_with("HTTP/1.1 401"));
        assert!(http(port, "GET /v1/snapshot HTTP/1.1\r\nAuthorization: Bearer nope\r\n\r\n").starts_with("HTTP/1.1 401"));
        let bad = r#"{"pin":"xxxxxx"}"#;
        assert!(http(port, &format!("POST /v1/pair HTTP/1.1\r\nContent-Length: {}\r\n\r\n{bad}", bad.len())).starts_with("HTTP/1.1 401"));
        let pin = r.info().pin;
        let good = format!(r#"{{"pin":"{pin}","device":"Quest 3"}}"#);
        let resp = http(port, &format!("POST /v1/pair HTTP/1.1\r\nContent-Length: {}\r\n\r\n{good}", good.len()));
        assert!(resp.starts_with("HTTP/1.1 200"), "{resp}");
        let token = resp.split("\"token\":\"").nth(1).unwrap().split('"').next().unwrap().to_string();
        assert_eq!(saved.lock().unwrap().as_slice(), [token.clone()]);
        assert_ne!(r.info().pin, pin, "PIN changes after pairing");
        let snap = http(port, &format!("GET /v1/snapshot HTTP/1.1\r\nAuthorization: Bearer {token}\r\n\r\n"));
        assert!(snap.starts_with("HTTP/1.1 200") && snap.contains("\"cpu_load\":12.0"), "{snap}");
        let dev = r#"{"name":"Quest 2","battery":81.0,"charging":true,"thermal":1,"wifi_rssi":-52,"battery_temp":999}"#;
        let resp = http(port, &format!("POST /v1/snapshot HTTP/1.1\r\nAuthorization: Bearer {token}\r\nContent-Length: {}\r\n\r\n{dev}", dev.len()));
        assert!(resp.starts_with("HTTP/1.1 200") && resp.contains("\"cpu_load\":12.0"), "{resp}");
        let d = r.devices();
        assert_eq!(d.len(), 1);
        assert_eq!((d[0].name.as_str(), d[0].battery, d[0].charging, d[0].battery_temp), ("Quest 2", Some(81.0), true, None));
        assert!(http(port, "DELETE /v1/snapshot HTTP/1.1\r\n\r\n").starts_with("HTTP/1.1 404"));
        r.stop();
    }
}

/// Name of the inbound rule Mark II creates for the LAN server (Windows).
#[cfg(windows)]
const FW_RULE: &str = "Mark II (app na rede local)";

/// Whether the Windows firewall has Mark II's own allow rule (no admin needed to read).
#[cfg(windows)]
pub fn firewall_ok() -> bool {
    use std::os::windows::process::CommandExt;
    std::process::Command::new("netsh")
        .args(["advfirewall", "firewall", "show", "rule"])
        .arg(format!("name={FW_RULE}"))
        .creation_flags(0x0800_0000)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(not(windows))]
pub fn firewall_ok() -> bool {
    true
}

/// Replaces every firewall rule for this exe (including stale "block" ones Windows creates
/// when its prompt is dismissed) with one TCP allow rule for `port`, reachable only from the
/// local subnet. Asks UAC once. Err("cancelado") when UAC is declined.
#[cfg(windows)]
pub fn allow_firewall(port: u16) -> Result<(), String> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::GetExitCodeProcess;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe = exe.to_string_lossy();
    let sys = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    let cmd = std::path::PathBuf::from(&sys).join(r"System32\cmd.exe");
    let args = format!(
        "/c netsh advfirewall firewall delete rule name=all dir=in program=\"{exe}\" & \
         netsh advfirewall firewall delete rule name=\"{FW_RULE}\" & \
         netsh advfirewall firewall add rule name=\"{FW_RULE}\" dir=in action=allow enable=yes \
         program=\"{exe}\" protocol=TCP localport={port} remoteip=localsubnet profile=any"
    );
    let h = match crate::lhm::elevate(&cmd, &args, true) {
        Ok(h) => h,
        Err(1223) => {
            crate::log::warn("firewall: UAC recusado");
            return Err("cancelado".into());
        }
        Err(e) => {
            crate::log::error(format!("firewall: não consegui pedir o UAC (erro {e})"));
            return Err(format!("erro {e}"));
        }
    };
    let mut code = 1u32;
    if !h.is_null() {
        unsafe {
            GetExitCodeProcess(h, &mut code);
            CloseHandle(h);
        }
    }
    if code == 0 && firewall_ok() {
        crate::log::info(format!("firewall: Mark II liberado na porta {port} (só rede local)"));
        Ok(())
    } else {
        crate::log::error(format!("firewall: o netsh falhou (código {code})"));
        Err(format!("o netsh falhou (código {code})"))
    }
}

#[cfg(not(windows))]
pub fn allow_firewall(_port: u16) -> Result<(), String> {
    Ok(())
}
