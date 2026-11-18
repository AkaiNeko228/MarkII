use markii_core::*;
fn main() {
    let cfg = Config::default();
    let pc = pcinfo::collect();
    println!("slots {} max {} GB rgb {:?} ram {:?}", pc.ram_slots, pc.ram_max_gb, pc.rgb_devices, pc.ram);
    let mut c = Collector::new();
    for i in 0..30 {
        let (t, _) = c.sample(&cfg, Some(&pc));
        if !t.hw.board.chip.is_empty() || i == 29 {
            println!("[{i}] src={} lhm={} ({}) pawnio={}", t.source, t.lhm_state, t.lhm_msg, t.pawnio);
            println!("  cpu pkg={:?} max={:?} P={:?} cores={:?} vcore={:?} vid={:?} bus={:?} min/max {}..{} sess {}..{}",
                t.cpu.package_temp, t.cpu.core_max, t.cpu.power, t.cpu.cores_power, t.cpu.vcore, t.cpu.vid, t.cpu.bus_mhz, t.cpu.mhz_min, t.cpu.mhz_max, t.cpu.session_min, t.cpu.session_max);
            println!("  phys {:?}", t.cpu.phys.iter().map(|c| (c.kind.clone(), c.mhz, c.temp, c.volt)).collect::<Vec<_>>());
            println!("  gpu hot={:?} mem={:?} vrm={:?} v={:?} rpm={:?} bus={:?}", t.gpu.hotspot, t.gpu.mem_temp, t.gpu.vrm_temp, t.gpu.voltage, t.gpu.fan_rpm, t.gpu.bus_width);
            println!("  board {:?}", t.hw.board);
            println!("  fans {:?}", t.hw.fans);
            println!("  spd {:?}", t.hw.spd);
            println!("  drives {:?}", t.hw.drives);
            println!("  cooling {} {} {}", t.cooling.kind, t.cooling.confidence, t.cooling.reason);
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1000));
    }
}
