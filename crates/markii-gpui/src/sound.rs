//! Short UI sounds: an alert, an "all clear", a confirmation and the startup. Mark II ships
//! its own (crates/markii-gpui/sons/, made for it); the user can pick their own
//! .wav for each. Played asynchronously; nothing keeps running after the sound ends.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Sound {
    /// Something crossed a limit.
    Alert,
    /// Every alert is gone.
    Clear,
    /// An action worked (colors applied, log saved).
    Ok,
    /// The app opened.
    Start,
}

static ENABLED: AtomicBool = AtomicBool::new(true);

pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

/// The user's own .wav per sound (alert, clear, ok, start); empty = Mark II's own sound.
fn custom_cell() -> &'static Mutex<[String; 4]> {
    static C: OnceLock<Mutex<[String; 4]>> = OnceLock::new();
    C.get_or_init(Default::default)
}

fn slot(s: Sound) -> usize {
    match s {
        Sound::Alert => 0,
        Sound::Clear => 1,
        Sound::Ok => 2,
        Sound::Start => 3,
    }
}

pub fn set_custom(alert: &str, clear: &str, ok: &str, start: &str) {
    let mut c = custom_cell().lock().unwrap();
    if c[0] != alert || c[1] != clear || c[2] != ok || c[3] != start {
        *c = [alert.to_string(), clear.to_string(), ok.to_string(), start.to_string()];
    }
}

/// Name of the copy kept in the config folder for each sound.
pub fn file_name(s: Sound) -> &'static str {
    match s {
        Sound::Alert => "alert.wav",
        Sound::Clear => "clear.wav",
        Sound::Ok => "ok.wav",
        Sound::Start => "start.wav",
    }
}

const MAX_WAV: u64 = 5 * 1024 * 1024;

/// Checks that `src` is a WAV (RIFF/WAVE, up to 5 MB) and copies it to
/// `<config>/sons/`, so moving or deleting the original doesn't break it.
pub fn import_wav(src: &std::path::Path, s: Sound) -> Result<std::path::PathBuf, String> {
    let meta = std::fs::metadata(src).map_err(|e| e.to_string())?;
    if meta.len() > MAX_WAV {
        return Err("arquivo grande demais (máximo 5 MB)".into());
    }
    let bytes = std::fs::read(src).map_err(|e| e.to_string())?;
    if bytes.len() < 44 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("não é um arquivo .wav".into());
    }
    let dir = markii_core::log::dir().join("sons");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let dst = dir.join(file_name(s));
    std::fs::write(&dst, &bytes).map_err(|e| e.to_string())?;
    Ok(dst)
}

/// Bytes of a custom .wav, read once per version of the file.
fn custom_wav(path: &str) -> Option<&'static [u8]> {
    static CACHE: OnceLock<Mutex<HashMap<(String, u64, u128), &'static [u8]>>> = OnceLock::new();
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() > MAX_WAV {
        return None;
    }
    let when = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos());
    let key = (path.to_string(), meta.len(), when);
    let mut m = CACHE.get_or_init(Default::default).lock().unwrap();
    if let Some(b) = m.get(&key) {
        return Some(b);
    }
    let bytes = std::fs::read(path).ok()?;
    if bytes.len() < 44 || &bytes[..4] != b"RIFF" {
        return None;
    }
    let b: &'static [u8] = Box::leak(bytes.into_boxed_slice());
    m.insert(key, b);
    Some(b)
}

/// Plays the user's .wav for this sound (else Mark II's own), if sounds are on.
pub fn play(s: Sound) {
    if ENABLED.load(Ordering::Relaxed) {
        let custom = custom_cell().lock().unwrap()[slot(s)].clone();
        let mine = if custom.is_empty() { None } else { custom_wav(&custom) };
        if let Some(b) = mine.or(default_wav(s)) {
            play_bytes(b);
        }
    }
}

/// Mark II's own sound for `s`.
pub fn default_wav(s: Sound) -> Option<&'static [u8]> {
    Some(match s {
        Sound::Alert => include_bytes!("../sons/alerta.wav"),
        Sound::Clear => include_bytes!("../sons/resolvido.wav"),
        Sound::Ok => include_bytes!("../sons/confirmacao.wav"),
        Sound::Start => include_bytes!("../sons/abertura.wav"),
    })
}

/// Plays Mark II's own sound even when sounds are off (the ▶ buttons).
pub fn play_default(s: Sound) {
    if let Some(b) = default_wav(s) {
        play_bytes(b);
    }
}

/// Plays a .wav file even when sounds are off (the ▶ buttons). False if it can't be read.
pub fn play_file(path: &str) -> bool {
    match custom_wav(path) {
        Some(b) => {
            play_bytes(b);
            true
        }
        None => false,
    }
}

fn play_bytes(bytes: &'static [u8]) {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::Media::Audio::{PlaySoundA, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};
        PlaySoundA(bytes.as_ptr(), std::ptr::null_mut(), SND_MEMORY | SND_ASYNC | SND_NODEFAULT);
    }
    #[cfg(not(windows))]
    {
        use std::io::Write;
        use std::process::{Command, Stdio};
        // paplay (PulseAudio/PipeWire) or aplay (ALSA), whichever exists; ends with the sound
        std::thread::spawn(move || {
            for (cmd, args) in [("paplay", &["/dev/stdin"][..]), ("aplay", &["-q", "-"][..])] {
                if let Ok(mut ch) = Command::new(cmd).args(args).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
                    if let Some(mut i) = ch.stdin.take() {
                        let _ = i.write_all(bytes);
                    }
                    let _ = ch.wait();
                    return;
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_sounds_are_wavs() {
        for s in [Sound::Alert, Sound::Clear, Sound::Ok, Sound::Start] {
            let w = default_wav(s).unwrap();
            assert!(w.len() > 44 && &w[..4] == b"RIFF" && &w[8..12] == b"WAVE", "{s:?}");
        }
    }

    #[test]
    fn import_rejects_non_wav() {
        let dir = std::env::temp_dir().join("markii-sound-test");
        std::fs::create_dir_all(&dir).unwrap();
        let bad = dir.join("x.wav");
        std::fs::write(&bad, b"not a wav file at all, just some text here....").unwrap();
        assert!(import_wav(&bad, Sound::Ok).is_err());
        assert!(!play_file(bad.to_str().unwrap()));
    }
}
