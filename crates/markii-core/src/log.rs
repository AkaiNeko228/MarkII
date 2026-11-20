//! Mark II's event log, kept in memory only (last 1000 lines). Nothing is written
//! to disk until the user clicks "Criar log", which saves a report
//! (`markii-log-<data>.txt`) next to config.json. Never holds PINs or tokens.
//!
//! The SteamVR panel process (`--vr-overlay`) has no window: its lines go to its
//! stdout with a marker and Mark II adds them here (see vr.rs).
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

const MAX_LINES: usize = 1000;
/// Prefix of the log lines a child process prints on stdout.
pub const CHILD_MARK: &str = "@markii-log ";

struct State {
    role: &'static str,
    lines: VecDeque<String>,
}

fn state() -> &'static Mutex<State> {
    static S: OnceLock<Mutex<State>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(State { role: "app", lines: VecDeque::new() }))
}

/// Folder of config.json (where "Criar log" saves).
pub fn dir() -> PathBuf {
    crate::rgb::config_dir().parent().map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

/// Names this process in the log and records panics. Call once at startup.
pub fn init(process_role: &'static str) {
    state().lock().unwrap().role = process_role;
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        write("PANIC", &info.to_string());
        prev(info);
    }));
}

/// Local time "2026-11-20 12:34:56".
pub fn now() -> String {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::SystemInformation::GetLocalTime;
        let mut t = unsafe { std::mem::zeroed() };
        unsafe { GetLocalTime(&mut t) };
        format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond)
    }
    #[cfg(not(windows))]
    {
        let mut now: libc::time_t = 0;
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        unsafe {
            libc::time(&mut now);
            libc::localtime_r(&now, &mut tm);
        }
        format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday, tm.tm_hour, tm.tm_min, tm.tm_sec)
    }
}

fn write(level: &str, msg: &str) {
    let mut st = state().lock().unwrap();
    let line = format!("{} [{}] {level}: {}", now(), st.role, msg.replace(['\r', '\n'], " "));
    if st.role == "vr" {
        // no window in this process: hand the line to Mark II
        println!("{CHILD_MARK}{line}");
        return;
    }
    st.lines.push_back(line);
    while st.lines.len() > MAX_LINES {
        st.lines.pop_front();
    }
}

/// Adds a line printed by a child process (already formatted).
pub fn push_child(line: &str) {
    if let Some(l) = line.strip_prefix(CHILD_MARK) {
        let mut st = state().lock().unwrap();
        st.lines.push_back(l.chars().filter(|c| !c.is_control()).take(400).collect());
        while st.lines.len() > MAX_LINES {
            st.lines.pop_front();
        }
    }
}

pub fn info(msg: impl AsRef<str>) {
    write("info", msg.as_ref());
}

pub fn warn(msg: impl AsRef<str>) {
    write("aviso", msg.as_ref());
}

pub fn error(msg: impl AsRef<str>) {
    write("ERRO", msg.as_ref());
}

pub fn lines() -> Vec<String> {
    state().lock().unwrap().lines.iter().cloned().collect()
}

/// Saves `header` + the events + the RGB login log to `markii-log-<data>.txt`
/// and returns its path.
pub fn save_report(header: &str) -> Result<PathBuf, String> {
    let stamp = now().replace([':', ' '], "-");
    let p = dir().join(format!("markii-log-{stamp}.txt"));
    let mut out = String::new();
    out.push_str(header.trim_end());
    out.push_str("\n\n== Eventos desta sessão ==\n");
    let ls = lines();
    if ls.is_empty() {
        out.push_str("(nenhum)\n");
    }
    for l in ls {
        out.push_str(&l);
        out.push('\n');
    }
    let rgb = crate::rgb::saved_path().with_file_name("rgb-login.log");
    if let Ok(s) = std::fs::read_to_string(&rgb) {
        out.push_str("\n== Cores ao ligar o PC (rgb-login.log) ==\n");
        out.push_str(&s);
    }
    std::fs::create_dir_all(dir()).map_err(|e| e.to_string())?;
    std::fs::write(&p, out.replace('\n', if cfg!(windows) { "\r\n" } else { "\n" })).map_err(|e| e.to_string())?;
    Ok(p)
}

/// Shows the file in the file manager (selected, on Windows).
pub fn reveal(p: &std::path::Path) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // raw: explorer wants /select,"path" (quotes only around the path, for spaces)
        let _ = std::process::Command::new("explorer").raw_arg(format!("/select,\"{}\"", p.display())).spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = std::process::Command::new("xdg-open").arg(p.parent().unwrap_or(p)).spawn();
    }
}
