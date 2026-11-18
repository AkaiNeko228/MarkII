//! Reads HWiNFO's shared-memory sensor block (Global\HWiNFO_SENS_SM2).
//! HWiNFO must be running with "Shared Memory Support" enabled.

#[derive(Debug, Clone)]
pub struct Reading {
    pub kind: u32,
    pub sensor: String,
    pub label: String,
    pub label_user: String,
    #[allow(dead_code)]
    pub unit: String,
    pub value: f64,
}

const SIG_OK: u32 = 0x5369_5748; // "HWiS"
const SIG_DEAD: u32 = 0x4441_4544; // "DEAD"

fn cstr(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    // HWiNFO's legacy strings are ANSI (CP1252); latin-1 covers what we need (°).
    b[..end].iter().map(|&c| c as char).collect::<String>().trim().to_string()
}

pub fn parse(b: &[u8]) -> Result<Vec<Reading>, String> {
    let u32at = |o: usize| -> Option<u32> {
        b.get(o..o + 4).map(|s| u32::from_le_bytes(s.try_into().unwrap()))
    };
    let f64at = |o: usize| -> Option<f64> {
        b.get(o..o + 8).map(|s| f64::from_le_bytes(s.try_into().unwrap()))
    };
    let sig = u32at(0).ok_or("HWiNFO shared memory too small")?;
    if sig == SIG_DEAD {
        return Err("HWiNFO is shutting down".into());
    }
    if sig != SIG_OK {
        return Err(format!("unexpected HWiNFO signature 0x{sig:08x}"));
    }
    let s_off = u32at(20).unwrap_or(0) as usize;
    let s_size = u32at(24).unwrap_or(0) as usize;
    let s_num = u32at(28).unwrap_or(0) as usize;
    let r_off = u32at(32).unwrap_or(0) as usize;
    let r_size = u32at(36).unwrap_or(0) as usize;
    let r_num = u32at(40).unwrap_or(0) as usize;
    if s_size < 264 || r_size < 292 {
        return Err("unsupported HWiNFO shared memory layout".into());
    }

    let mut sensors = Vec::with_capacity(s_num);
    for i in 0..s_num {
        let o = s_off + i * s_size;
        let Some(e) = b.get(o..o + 264) else { break };
        let user = cstr(&e[136..264]);
        sensors.push(if user.is_empty() { cstr(&e[8..136]) } else { user });
    }

    let mut out = Vec::with_capacity(r_num);
    for i in 0..r_num {
        let o = r_off + i * r_size;
        let Some(e) = b.get(o..o + 292) else { break };
        let kind = u32::from_le_bytes(e[0..4].try_into().unwrap());
        let si = u32::from_le_bytes(e[4..8].try_into().unwrap()) as usize;
        out.push(Reading {
            kind,
            sensor: sensors.get(si).cloned().unwrap_or_default(),
            label: cstr(&e[12..140]),
            label_user: cstr(&e[140..268]),
            unit: cstr(&e[268..284]),
            value: f64at(o + 284).unwrap_or(f64::NAN),
        });
    }
    Ok(out)
}

#[cfg(windows)]
pub fn read() -> Result<Vec<Reading>, String> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Memory::{
        MapViewOfFile, OpenFileMappingW, UnmapViewOfFile, VirtualQuery, FILE_MAP_READ,
        MEMORY_BASIC_INFORMATION,
    };
    let name: Vec<u16> = "Global\\HWiNFO_SENS_SM2\0".encode_utf16().collect();
    unsafe {
        let h = OpenFileMappingW(FILE_MAP_READ, 0, name.as_ptr());
        if h.is_null() {
            return Err("HWiNFO not running (or Shared Memory Support is off)".into());
        }
        let view = MapViewOfFile(h, FILE_MAP_READ, 0, 0, 0);
        if view.Value.is_null() {
            CloseHandle(h);
            return Err("could not map HWiNFO shared memory".into());
        }
        let base = view.Value as *const u8;
        let mut mbi: MEMORY_BASIC_INFORMATION = std::mem::zeroed();
        let size = if VirtualQuery(
            base as _,
            &mut mbi,
            std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
        ) != 0
        {
            mbi.RegionSize
        } else {
            0
        };
        let res = parse(std::slice::from_raw_parts(base, size));
        UnmapViewOfFile(view);
        CloseHandle(h);
        res
    }
}

#[cfg(not(windows))]
pub fn read() -> Result<Vec<Reading>, String> {
    Err("HWiNFO is Windows-only".into())
}
