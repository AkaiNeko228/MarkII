//! Memory helpers shared by the front ends: SPD timings in clock cycles and the
//! motherboard slot map (which slots exist and which hold a module).

use crate::pcinfo::{PcInfo, RamModule};
use regex::Regex;
use std::sync::OnceLock;

/// Converts an SPD time (ns) to clock cycles at `mts` MT/s, JEDEC-style rounding.
pub fn cycles(t_ns: f64, mts: u32) -> Option<u32> {
    if mts == 0 || !t_ns.is_finite() || t_ns <= 0.0 {
        return None;
    }
    let tck = 2000.0 / mts as f64;
    Some((t_ns / tck - 0.025).ceil().max(1.0) as u32)
}

/// First-word latency in ns for a CAS latency in cycles.
pub fn cas_ns(cl: u32, mts: u32) -> Option<f64> {
    (mts > 0).then(|| cl as f64 * 2000.0 / mts as f64)
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct Slot {
    pub name: String,
    pub module: Option<RamModule>,
}

fn loc_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?i)channel\s*([A-H])\W*dimm\s*(\d)").unwrap())
}

/// How many memory slots the board has. Many BIOSes copy a 4-slot SMBIOS template,
/// so a board with an Intel chipset limited to one DIMM per channel (H310, H410,
/// H510, H610) is capped at 2, unless more modules are actually installed.
pub fn slot_count(pc: &PcInfo) -> usize {
    static R: OnceLock<Regex> = OnceLock::new();
    let one_dpc = R.get_or_init(|| Regex::new(r"(?i)\bH[3-6]10").unwrap());
    let mut n = pc.ram_slots as usize;
    if n > 2 && one_dpc.is_match(&pc.board) {
        n = 2;
    }
    n.max(pc.ram.len())
}

/// Builds the list of slots the board reports, marking the populated ones.
/// SMBIOS gives the slot count and each module's locator ("ChannelB-DIMM0");
/// empty slot names are inferred from that pattern when the board uses it.
pub fn slots(pc: &PcInfo) -> Vec<Slot> {
    let n = slot_count(pc);
    if n == 0 {
        return Vec::new();
    }
    let parsed: Vec<Option<(char, u32)>> = pc
        .ram
        .iter()
        .map(|m| loc_re().captures(&m.slot).map(|c| (c[1].to_ascii_uppercase().chars().next().unwrap_or('A'), c[2].parse().unwrap_or(0))))
        .collect();
    if !pc.ram.is_empty() && parsed.iter().all(|p| p.is_some()) {
        let channels = if n >= 2 && n % 2 == 0 { 2 } else { 1 };
        let per = n / channels;
        let base = parsed.iter().flatten().map(|p| p.1).min().unwrap_or(0).min(1);
        let mut out = Vec::new();
        for ch in 0..channels {
            let letter = (b'A' + ch as u8) as char;
            for d in 0..per {
                let dimm = base + d as u32;
                let module = pc.ram.iter().zip(&parsed).find(|(_, p)| **p == Some((letter, dimm))).map(|(m, _)| m.clone());
                out.push(Slot { name: format!("{letter}{dimm}"), module });
            }
        }
        // Modules whose locator fell outside the inferred grid still get shown.
        for (m, p) in pc.ram.iter().zip(&parsed) {
            if !out.iter().any(|s| s.module.as_ref().is_some_and(|x| x.slot == m.slot)) {
                let (l, d) = p.unwrap();
                out.push(Slot { name: format!("{l}{d}"), module: Some(m.clone()) });
            }
        }
        return out;
    }
    (0..n)
        .map(|i| Slot {
            name: pc.ram.get(i).map(|m| m.slot.clone()).unwrap_or_else(|| format!("Slot {}", i + 1)),
            module: pc.ram.get(i).cloned(),
        })
        .collect()
}

/// "Single", "Dual", "Quad"... from how many distinct channels hold a module.
pub fn channel_mode(pc: &PcInfo) -> &'static str {
    let mut ch: Vec<char> = pc.ram.iter().filter_map(|m| loc_re().captures(&m.slot)).map(|c| c[1].to_ascii_uppercase().chars().next().unwrap_or('A')).collect();
    ch.sort();
    ch.dedup();
    let n = if ch.is_empty() { pc.ram.len().min(2) } else { ch.len() };
    match n {
        0 => "—",
        1 => "Single channel",
        2 => "Dual channel",
        3 => "Triple channel",
        _ => "Quad channel",
    }
}

#[cfg(test)]
mod slot_tests {
    use super::*;

    #[test]
    fn h310_has_two_slots_even_if_smbios_says_four() {
        let m = RamModule { slot: "ChannelB-DIMM0".into(), size_gb: 16.0, ..Default::default() };
        let pc = PcInfo { board: "H310M H 2.0".into(), ram_slots: 4, ram: vec![m.clone()], ..Default::default() };
        let s = slots(&pc);
        assert_eq!(s.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), ["A0", "B0"]);
        assert!(s[1].module.is_some());
        let b450 = PcInfo { board: "B450M DS3H".into(), ram_slots: 4, ram: vec![m], ..Default::default() };
        assert_eq!(slot_count(&b450), 4);
    }
}
