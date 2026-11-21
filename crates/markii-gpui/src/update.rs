//! Updates from GitHub releases. Mark II asks once when it opens; if there is
//! a newer version it shows a bar, and "Atualizar" downloads the zip, checks
//! its SHA-256 against what GitHub publishes, unpacks it and hands over to a
//! short script that waits for Mark II to close, copies the files and opens
//! the new version. Nothing keeps running afterwards.

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};

const API: &str = "https://api.github.com/repos/AkaiNeko228/MarkII/releases/latest";
#[cfg(windows)]
const ASSET_SUFFIX: &str = "-windows-x64.zip";
#[cfg(not(windows))]
const ASSET_SUFFIX: &str = "-linux-x64.tar.gz";

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Idle,
    /// A newer version: (version, release page).
    Available(String, String),
    Working(String),
    Failed(String),
    /// The script took over; Mark II must close now.
    Restarting,
}

struct Release {
    version: String,
    page: String,
    asset: Option<(String, String, Option<String>)>, // name, url, sha256
}

fn cell() -> &'static Mutex<State> {
    static S: OnceLock<Mutex<State>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(State::Idle))
}

pub fn state() -> State {
    cell().lock().unwrap().clone()
}

fn set(s: State) {
    *cell().lock().unwrap() = s;
}

fn curl() -> Command {
    let mut c = Command::new("curl");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    c.args(["-fsSL", "--max-time", "600", "-H", "User-Agent: MarkII-updater"]);
    c
}

fn parse_version(v: &str) -> Option<(u32, u32, u32)> {
    let mut it = v.trim().trim_start_matches('v').split(['.', '-']).map(|x| x.parse::<u32>().ok());
    Some((it.next()??, it.next()??, it.next().flatten().unwrap_or(0)))
}

fn newer(latest: &str, current: &str) -> bool {
    matches!((parse_version(latest), parse_version(current)), (Some(a), Some(b)) if a > b)
}

fn fetch() -> Result<Release, String> {
    let out = curl().arg(API).output().map_err(|e| format!("curl: {e}"))?;
    if !out.status.success() {
        return Err("sem resposta do GitHub".into());
    }
    let j: serde_json::Value = serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())?;
    let version = j["tag_name"].as_str().ok_or("release sem versão")?.trim_start_matches('v').to_string();
    let page = j["html_url"].as_str().unwrap_or("https://github.com/AkaiNeko228/MarkII/releases").to_string();
    let asset = j["assets"].as_array().and_then(|a| {
        a.iter().find(|x| x["name"].as_str().is_some_and(|n| n.ends_with(ASSET_SUFFIX))).map(|x| {
            (
                x["name"].as_str().unwrap_or_default().to_string(),
                x["browser_download_url"].as_str().unwrap_or_default().to_string(),
                x["digest"].as_str().and_then(|d| d.strip_prefix("sha256:")).map(|d| d.to_ascii_lowercase()),
            )
        })
    });
    Ok(Release { version, page, asset })
}

/// Asks GitHub once, in the background, when Mark II opens.
pub fn check() {
    std::thread::spawn(|| match fetch() {
        // MARKII_UPDATE_TEST_FROM=0.1.0 pretends to be an older version (to see the bar)
        Ok(r) if newer(&r.version, &std::env::var("MARKII_UPDATE_TEST_FROM").unwrap_or_else(|_| env!("CARGO_PKG_VERSION").into())) => {
            markii_core::log::info(format!("atualização: v{} disponível", r.version));
            set(State::Available(r.version, r.page));
        }
        Ok(_) => {}
        Err(e) => markii_core::log::info(format!("atualização: não deu para conferir ({e})")),
    });
}

/// Whether this copy can update itself (release builds on Windows; test
/// builds and Linux open the release page instead).
pub fn can_install() -> bool {
    cfg!(windows) && !crate::view::ALPHA
}

pub fn open_page(url: &str) {
    #[cfg(windows)]
    let _ = Command::new("explorer").arg(url).spawn();
    #[cfg(not(windows))]
    let _ = Command::new("xdg-open").arg(url).spawn();
}

fn sha256(p: &Path) -> Result<String, String> {
    let mut f = std::fs::File::open(p).map_err(|e| e.to_string())?;
    let mut h = Sha256::new();
    std::io::copy(&mut f, &mut h).map_err(|e| e.to_string())?;
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Downloads and unpacks the new version, then starts the script that
/// swaps the files once Mark II has closed. On success the state becomes
/// Restarting and the caller quits Mark II.
pub fn install() {
    if !matches!(state(), State::Available(..) | State::Failed(_)) {
        return;
    }
    std::thread::spawn(|| {
        if let Err(e) = install_inner() {
            markii_core::log::error(format!("atualização: {e}"));
            set(State::Failed(e));
        }
    });
}

fn install_inner() -> Result<(), String> {
    set(State::Working("conferindo a versão…".into()));
    let r = fetch()?;
    let (name, url, digest) = r.asset.ok_or("a release não tem o arquivo para Windows")?;
    let digest = digest.ok_or("o GitHub não informou o SHA-256 do arquivo")?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = exe.parent().ok_or("pasta do Mark II")?.to_path_buf();
    let work = std::env::temp_dir().join("markii-update");
    let _ = std::fs::remove_dir_all(&work);
    let staging = work.join("novo");
    std::fs::create_dir_all(&staging).map_err(|e| e.to_string())?;
    let zip = work.join(&name);

    set(State::Working(format!("baixando v{}…", r.version)));
    let ok = curl().arg("-o").arg(&zip).arg(&url).status().map_err(|e| format!("curl: {e}"))?.success();
    if !ok {
        return Err("o download falhou".into());
    }
    set(State::Working("conferindo o arquivo…".into()));
    if sha256(&zip)? != digest {
        let _ = std::fs::remove_dir_all(&work);
        return Err("o arquivo baixado não confere (SHA-256); nada foi trocado".into());
    }
    set(State::Working("descompactando…".into()));
    let mut tar = Command::new("tar");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        tar.creation_flags(0x0800_0000);
    }
    if !tar.arg("-xf").arg(&zip).arg("-C").arg(&staging).status().map_err(|e| format!("tar: {e}"))?.success() {
        return Err("não deu para descompactar".into());
    }
    if !staging.join("MarkII.exe").exists() {
        return Err("o zip não tem o MarkII.exe".into());
    }
    let script = write_script(&work, &staging, &dir, std::process::id())?;
    let mut cmd = Command::new("cmd");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    cmd.arg("/C").arg(&script).spawn().map_err(|e| format!("cmd: {e}"))?;
    markii_core::log::info(format!("atualização: v{} pronta, reabrindo", r.version));
    set(State::Restarting);
    Ok(())
}

/// Waits for this Mark II to close, copies the new files over the old ones,
/// opens Mark II again and cleans up after itself.
fn write_script(work: &Path, staging: &Path, dir: &Path, pid: u32) -> Result<PathBuf, String> {
    let q = |p: &Path| p.display().to_string().replace('"', "");
    let body = format!(
        "@echo off\r\n\
         :wait\r\n\
         tasklist /FI \"PID eq {pid}\" /NH | find \"{pid}\" >nul && (timeout /t 1 /nobreak >nul & goto wait)\r\n\
         timeout /t 2 /nobreak >nul\r\n\
         robocopy \"{src}\" \"{dst}\" /E /R:10 /W:1 /NFL /NDL /NJH /NJS /NP >nul\r\n\
         start \"\" \"{dst}\\MarkII.exe\"\r\n\
         cd /d \"%TEMP%\"\r\n\
         rmdir /s /q \"{work}\"\r\n",
        src = q(staging),
        dst = q(dir),
        work = q(work),
    );
    let p = std::env::temp_dir().join("markii-update.cmd");
    std::fs::write(&p, body).map_err(|e| e.to_string())?;
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert!(newer("v0.8.1", "0.8.0"));
        assert!(newer("0.10.0", "0.9.9"));
        assert!(!newer("v0.8.0", "0.8.0"));
        assert!(!newer("v0.7.4", "0.8.0"));
        assert!(!newer("lixo", "0.8.0"));
    }
}
