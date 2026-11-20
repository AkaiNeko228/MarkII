//! Recognises the brand of the CPU, GPU, motherboard and memory modules from the
//! names Windows reports, and provides a logo for each (Simple Icons, CC0) or,
//! when no free logo exists, a text badge the front end draws itself.

use crate::pcinfo::{PcInfo, RamModule};
use regex::Regex;
use serde::Serialize;
use std::sync::OnceLock;

#[derive(Serialize, Clone, Debug, Default)]
pub struct Brand {
    /// Stable id ("intel", "amd", "nvidia", "asus"...); "" when unknown.
    pub key: String,
    /// Display name ("Intel", "AMD Radeon", "Intel Arc"...).
    pub name: String,
    /// Product line ("Core i7 · 9ª geração", "GeForce GTX", "Ryzen 7"...).
    pub line: String,
    /// Brand colour, already adjusted to read on the dark theme ("#RRGGBB").
    pub color: String,
    /// Monochrome SVG logo filled with `color`, if a free logo exists.
    pub svg: Option<String>,
    /// Extra word drawn after the logo ("RADEON", "ARC") when the product line
    /// has no logo of its own.
    pub tag: String,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct Brands {
    pub cpu: Brand,
    pub gpu: Brand,
    pub board: Brand,
    /// One per installed module, same order as `PcInfo::ram`.
    pub ram: Vec<Brand>,
}

const TEXT: &str = "#E8EDF3";

fn logo(key: &str) -> Option<&'static str> {
    Some(match key {
        "intel" => include_str!("../assets/logos/intel.svg"),
        "amd" => include_str!("../assets/logos/amd.svg"),
        "nvidia" => include_str!("../assets/logos/nvidia.svg"),
        "asus" => include_str!("../assets/logos/asus.svg"),
        "rog" => include_str!("../assets/logos/republicofgamers.svg"),
        "msi" => include_str!("../assets/logos/msi.svg"),
        "corsair" => include_str!("../assets/logos/corsair.svg"),
        "kingston" => include_str!("../assets/logos/kingstontechnology.svg"),
        "hyperx" => include_str!("../assets/logos/hyperx.svg"),
        "samsung" => include_str!("../assets/logos/samsung.svg"),
        "hp" => include_str!("../assets/logos/hp.svg"),
        "dell" => include_str!("../assets/logos/dell.svg"),
        "lenovo" => include_str!("../assets/logos/lenovo.svg"),
        "acer" => include_str!("../assets/logos/acer.svg"),
        "supermicro" => include_str!("../assets/logos/supermicro.svg"),
        "nzxt" => include_str!("../assets/logos/nzxt.svg"),
        _ => return None,
    })
}

/// Brand colours (official hex from Simple Icons where available).
fn color(key: &str) -> &'static str {
    match key {
        "intel" => "#0071C5",
        "amd" => "#ED1C24",
        "nvidia" => "#76B900",
        "rog" => "#FF0029",
        "msi" => "#FF0000",
        "hyperx" => "#E21836",
        "samsung" => "#1428A0",
        "hp" => "#0096D6",
        "dell" => "#007DB8",
        "lenovo" => "#E2231A",
        "acer" => "#83B81A",
        "supermicro" => "#151F6D",
        _ => TEXT,
    }
}

/// Lifts colours that would disappear on the dark background.
fn readable(hex: &str) -> String {
    let v = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0xE8EDF3);
    let (r, g, b) = (((v >> 16) & 255) as f64, ((v >> 8) & 255) as f64, (v & 255) as f64);
    let lum = (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255.0;
    if lum >= 0.30 {
        return format!("#{v:06X}");
    }
    if lum < 0.08 {
        return TEXT.into(); // black logos (ASUS, Corsair...) become light
    }
    // dark blues/reds: mix halfway to white
    let m = |c: f64| (c + (255.0 - c) * 0.45).round() as u32;
    format!("#{:02X}{:02X}{:02X}", m(r), m(g), m(b))
}

fn make(key: &str, name: &str, line: String) -> Brand {
    let col = readable(color(key));
    Brand {
        key: key.into(),
        name: name.into(),
        line,
        svg: logo(key).map(|s| s.replacen("<svg ", &format!("<svg fill=\"{col}\" "), 1)),
        color: col,
        tag: String::new(),
    }
}

fn re(cell: &'static OnceLock<Regex>, pat: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pat).unwrap())
}

// ---------------------------------------------------------------- CPU

pub fn cpu(name: &str) -> Brand {
    let n = name.replace("(R)", "").replace("(TM)", "").replace("(tm)", "");
    let low = n.to_lowercase();
    // AMD first: "AMD FX-8350 Eight-Core" and "Athlon II X2 Dual-Core" say "core" too
    if low.contains("amd") || low.contains("ryzen") || low.contains("athlon") || low.contains("epyc") || low.contains("phenom") || low.contains("threadripper") {
        static RY: OnceLock<Regex> = OnceLock::new();
        let line = if let Some(c) = re(&RY, r"(?i)Ryzen\s+(Threadripper|AI\s+\d+|\d)\s*(?:PRO\s+)?(\d)?").captures(&n) {
            match c.get(2) {
                Some(s) if !c[1].contains("AI") && !c[1].eq_ignore_ascii_case("threadripper") => format!("Ryzen {} · série {}000", &c[1], s.as_str()),
                _ => format!("Ryzen {}", &c[1]),
            }
        } else {
            ["EPYC", "Athlon", "Phenom", "FX"].iter().find(|w| low.contains(&w.to_lowercase())).map_or(String::new(), |w| w.to_string())
        };
        return make("amd", "AMD", line);
    }
    if low.contains("intel") || low.contains("core") || low.contains("xeon") || low.contains("celeron") || low.contains("pentium") {
        static ULTRA: OnceLock<Regex> = OnceLock::new();
        static CORE: OnceLock<Regex> = OnceLock::new();
        let line = if let Some(c) = re(&ULTRA, r"(?i)Core\s+Ultra\s+(\d)\s+(\d)").captures(&n) {
            format!("Core Ultra {} · série {}", &c[1], &c[2])
        } else if let Some(c) = re(&CORE, r"(?i)Core\s+(i\d)[- ]?(\d{4,5})").captures(&n) {
            let num = &c[2];
            // 4 digits = gen 2-9 (i7-9700), 5 digits = gen 10-14 (i7-12700)
            let generation = if num.len() == 5 { &num[..2] } else { &num[..1] };
            format!("Core {} · {}ª geração", &c[1], generation)
        } else {
            ["Xeon", "Pentium", "Celeron", "Atom", "Core"].iter().find(|w| low.contains(&w.to_lowercase())).map_or(String::new(), |w| w.to_string())
        };
        return make("intel", "Intel", line);
    }
    if low.contains("snapdragon") || low.contains("qualcomm") {
        return Brand { key: "qualcomm".into(), name: "Qualcomm".into(), line: "Snapdragon".into(), color: TEXT.into(), ..Default::default() };
    }
    Brand { name: n.split_whitespace().next().unwrap_or("").to_string(), color: TEXT.into(), ..Default::default() }
}

// ---------------------------------------------------------------- GPU

pub fn gpu(name: &str) -> Brand {
    let low = name.to_lowercase();
    if low.contains("nvidia") || low.contains("geforce") || low.contains("quadro") || low.contains("rtx") || low.contains("gtx") || low.contains("tesla") {
        let line = ["GeForce RTX", "GeForce GTX", "GeForce GT", "GeForce", "Quadro", "RTX", "Tesla"]
            .iter()
            .find(|w| low.contains(&w.to_lowercase()))
            .map_or(String::new(), |w| w.to_string());
        return make("nvidia", "NVIDIA", line);
    }
    if low.contains("radeon") || low.contains("amd") || low.contains("firepro") {
        let mut b = make("amd", "AMD Radeon", if low.contains("rx ") { "Radeon RX".into() } else { "Radeon".into() });
        b.tag = "RADEON".into();
        return b;
    }
    let arc = word_arc(&low);
    if low.contains("intel") || arc || low.contains("iris") || low.contains("uhd") {
        let mut b = make(
            "intel",
            if arc { "Intel Arc" } else { "Intel" },
            if arc { "Arc".into() } else if low.contains("iris") { "Iris Xe (integrada)".into() } else { "Graphics (integrada)".into() },
        );
        if arc {
            b.tag = "ARC".into();
        }
        return b;
    }
    Brand { name: name.split_whitespace().next().unwrap_or("").to_string(), color: TEXT.into(), ..Default::default() }
}

fn word_arc(low: &str) -> bool {
    static R: OnceLock<Regex> = OnceLock::new();
    re(&R, r"\barc\b").is_match(low)
}

/// Virtual / remote display adapters (Virtual Desktop, Parsec, RDP, Basic Display,
/// IddSampleDriver...): never the GPU to monitor.
pub fn is_virtual_gpu(name: &str) -> bool {
    let l = name.to_lowercase();
    ["virtual", "parsec", "remote display", "basic display", "basic render", "idd", "spacedesk", "duet", "citrix", "vmware", "hyper-v", "displaylink", "meta quest", "oculus"]
        .iter()
        .any(|w| l.contains(w))
}

/// The GPU the dashboard monitors: the first discrete card, else the first real one.
pub fn main_gpu(names: &[String]) -> String {
    let real: Vec<&String> = names.iter().filter(|n| !is_virtual_gpu(n)).collect();
    real.iter()
        .find(|n| {
            let l = n.to_lowercase();
            l.contains("nvidia") || l.contains("radeon") || word_arc(&l)
        })
        .or(real.first())
        .map(|n| n.to_string())
        .or_else(|| names.first().cloned())
        .unwrap_or_default()
}

// ---------------------------------------------------------------- motherboard

pub fn board(maker: &str, product: &str) -> Brand {
    let m = maker.to_lowercase();
    let p = product.to_lowercase();
    let (key, name) = if p.starts_with("rog") || p.contains("republic of gamers") {
        ("rog", "ASUS ROG")
    } else if m.contains("asus") {
        ("asus", "ASUS")
    } else if m.contains("micro-star") || m.starts_with("msi") {
        ("msi", "MSI")
    } else if m.contains("gigabyte") {
        ("gigabyte", "GIGABYTE")
    } else if m.contains("asrock") {
        ("asrock", "ASRock")
    } else if m.contains("biostar") {
        ("biostar", "BIOSTAR")
    } else if m.contains("evga") {
        ("evga", "EVGA")
    } else if m.contains("nzxt") {
        ("nzxt", "NZXT")
    } else if m.contains("supermicro") {
        ("supermicro", "Supermicro")
    } else if m.contains("dell") {
        ("dell", "Dell")
    } else if m.contains("hewlett") || m == "hp" || m.starts_with("hp ") {
        ("hp", "HP")
    } else if m.contains("lenovo") {
        ("lenovo", "Lenovo")
    } else if m.contains("acer") {
        ("acer", "Acer")
    } else if m.contains("intel") {
        ("intel", "Intel")
    } else {
        ("", "")
    };
    if key.is_empty() {
        let n = maker.split([',', ' ']).next().unwrap_or("").trim();
        return Brand { name: n.to_string(), line: product.to_string(), color: TEXT.into(), ..Default::default() };
    }
    make(key, name, product.to_string())
}

// ---------------------------------------------------------------- memory

/// JEDEC manufacturer id as Windows reports it (bank+code in various byte orders).
fn jedec(code: &str) -> Option<(&'static str, &'static str)> {
    Some(match code.to_uppercase().as_str() {
        "80CE" | "CE00" | "CE80" => ("samsung", "Samsung"),
        "802C" | "2C00" | "2C80" => ("micron", "Micron"),
        "80AD" | "AD00" | "AD80" => ("skhynix", "SK hynix"),
        "859B" | "9B05" => ("crucial", "Crucial"),
        "04CD" | "CD04" => ("gskill", "G.Skill"),
        "029E" | "9E02" => ("corsair", "Corsair"),
        "0198" | "9801" => ("kingston", "Kingston"),
        "04EF" | "EF04" => ("teamgroup", "TeamGroup"),
        "04CB" | "CB04" => ("adata", "ADATA"),
        "8502" | "0502" => ("patriot", "Patriot"),
        "01F1" | "F101" => ("pny", "PNY"),
        _ => return None,
    })
}

/// Module brand from its part number first (more specific), then the JEDEC id.
pub fn ram(m: &RamModule) -> Brand {
    let part = m.part.trim().to_uppercase();
    let by_part: Option<(&str, &str, &str)> = [
        ("KF", "kingston", "Kingston", "FURY"),
        ("KHX", "hyperx", "HyperX", ""),
        ("HX4", "hyperx", "HyperX", ""),
        ("HX3", "hyperx", "HyperX", ""),
        ("KVR", "kingston", "Kingston", "ValueRAM"),
        ("CM", "corsair", "Corsair", ""),
        ("F3-", "gskill", "G.Skill", ""),
        ("F4-", "gskill", "G.Skill", ""),
        ("F5-", "gskill", "G.Skill", ""),
        ("BL", "crucial", "Crucial", "Ballistix"),
        ("CT", "crucial", "Crucial", ""),
        ("M378", "samsung", "Samsung", ""),
        ("M471", "samsung", "Samsung", ""),
        ("M393", "samsung", "Samsung", ""),
        ("M425", "samsung", "Samsung", ""),
        ("HMA", "skhynix", "SK hynix", ""),
        ("HMT", "skhynix", "SK hynix", ""),
        ("HMC", "skhynix", "SK hynix", ""),
        ("MTA", "micron", "Micron", ""),
        ("MTC", "micron", "Micron", ""),
        ("TF", "teamgroup", "TeamGroup", "T-Force"),
        ("TED", "teamgroup", "TeamGroup", ""),
        ("AX4", "adata", "ADATA", "XPG"),
        ("AX5", "adata", "ADATA", "XPG"),
        ("AD4", "adata", "ADATA", ""),
        ("AD5", "adata", "ADATA", ""),
        ("PSD", "patriot", "Patriot", ""),
        ("PVS", "patriot", "Patriot", "Viper"),
    ]
    .iter()
    .find(|(pre, ..)| part.starts_with(pre))
    .map(|(_, k, n, l)| (*k, *n, *l));
    // HP-branded OEM modules: 5 alphanumerics + "AA" (e.g. 7EH65AA#).
    static HP: OnceLock<Regex> = OnceLock::new();
    let (key, name, line) = if let Some(b) = by_part {
        b
    } else if re(&HP, r"^[0-9A-Z]{5}AA#?$").is_match(&part) {
        ("hp", "HP", "")
    } else if let Some((k, n)) = jedec(&m.maker) {
        (k, n, "")
    } else {
        let n = m.maker.trim();
        let shown = if n.len() == 4 && n.chars().all(|c| c.is_ascii_hexdigit()) { "" } else { n };
        return Brand { name: shown.to_string(), line: m.part.clone(), color: TEXT.into(), ..Default::default() };
    };
    make(key, name, if line.is_empty() { m.part.trim().to_string() } else { format!("{line} · {}", m.part.trim()) })
}

pub fn detect(pc: &PcInfo) -> Brands {
    Brands {
        cpu: cpu(&pc.cpu),
        gpu: gpu(&main_gpu(&pc.gpus)),
        board: board(&pc.board_maker, &pc.board),
        ram: pc.ram.iter().map(ram).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpus() {
        let b = cpu("Intel(R) Core(TM) i7-9700 CPU @ 3.00GHz");
        assert_eq!((b.key.as_str(), b.line.as_str()), ("intel", "Core i7 · 9ª geração"));
        assert_eq!(cpu("12th Gen Intel(R) Core(TM) i5-12400F").line, "Core i5 · 12ª geração");
        assert_eq!(cpu("Intel(R) Core(TM) Ultra 7 155H").line, "Core Ultra 7 · série 1");
        assert_eq!(cpu("AMD Ryzen 7 5800X 8-Core Processor").line, "Ryzen 7 · série 5000");
        assert_eq!(cpu("AMD Ryzen Threadripper 3970X").line, "Ryzen Threadripper");
        assert!(cpu("AMD Ryzen 5 7600").svg.is_some());
    }

    #[test]
    fn gpus() {
        assert_eq!(gpu("NVIDIA GeForce GTX 1660 SUPER").line, "GeForce GTX");
        let r = gpu("AMD Radeon RX 7800 XT");
        assert_eq!((r.key.as_str(), r.tag.as_str()), ("amd", "RADEON"));
        let a = gpu("Intel(R) Arc(TM) A770 Graphics");
        assert_eq!((a.name.as_str(), a.tag.as_str()), ("Intel Arc", "ARC"));
        assert_eq!(gpu("Intel(R) UHD Graphics 630").tag, "");
        assert_eq!(main_gpu(&["Intel(R) UHD Graphics 630".into(), "NVIDIA GeForce RTX 3060".into()]), "NVIDIA GeForce RTX 3060");
    }

    #[test]
    fn boards_and_ram() {
        assert_eq!(board("ASUSTeK COMPUTER INC.", "ROG STRIX B550-F GAMING").key, "rog");
        assert_eq!(board("ASUSTeK COMPUTER INC.", "PRIME B450M-A").key, "asus");
        assert_eq!(board("Micro-Star International Co., Ltd.", "MAG B550").key, "msi");
        let g = board("Gigabyte Technology Co., Ltd.", "B550M AORUS");
        assert!(g.key == "gigabyte" && g.svg.is_none());
        let m = |maker: &str, part: &str| RamModule { maker: maker.into(), part: part.into(), ..Default::default() };
        assert_eq!(ram(&m("8054", "7EH65AA#")).key, "hp");
        assert_eq!(ram(&m("04CD", "F4-3600C18-16GVK")).key, "gskill");
        assert_eq!(ram(&m("0198", "KF432C16BB/16")).line, "FURY · KF432C16BB/16");
        assert_eq!(ram(&m("80CE", "M378A1K43EB2-CWE")).key, "samsung");
        // dark logos are lifted for the dark theme
        assert_eq!(board("ASUSTeK COMPUTER INC.", "PRIME").color, TEXT);
    }
}

#[cfg(test)]
mod detect_tests {
    use super::*;

    #[test]
    fn old_amd_cpus_are_amd() {
        assert_eq!(cpu("AMD FX(tm)-8350 Eight-Core Processor").key, "amd");
        assert_eq!(cpu("AMD Athlon(tm) II X2 250 Dual-Core Processor").key, "amd");
        assert_eq!(cpu("AMD Ryzen 5 5600 6-Core Processor").key, "amd");
        assert_eq!(cpu("Intel(R) Core(TM) i3-6006U CPU @ 2.00GHz").key, "intel");
    }

    #[test]
    fn virtual_adapters_are_not_the_main_gpu() {
        let l = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(main_gpu(&l(&["Virtual Desktop Monitor", "Parsec Virtual Display Adapter", "NVIDIA GeForce GTX 1660 SUPER"])), "NVIDIA GeForce GTX 1660 SUPER");
        assert_eq!(main_gpu(&l(&["Virtual Desktop Monitor", "Intel(R) HD Graphics 520"])), "Intel(R) HD Graphics 520");
        assert_eq!(gpu("Intel(R) Arc(TM) A770 Graphics").name, "Intel Arc");
    }
}
