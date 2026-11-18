"use strict";
const $ = (id) => document.getElementById(id);
const HIST = 180;
const hist = { cpu: [], gpu: [], mem: [], net: [], cput: [], gput: [] };
const push = (k, v) => { const a = hist[k]; a.push(v); if (a.length > HIST) a.shift(); };

const css = (n) => getComputedStyle(document.documentElement).getPropertyValue(n).trim();
const f0 = (v) => (v == null || !isFinite(v) ? "—" : Math.round(v).toString());
const f1 = (v) => (v == null || !isFinite(v) ? "—" : v.toFixed(1));
const gb = (b) => (b / 1073741824).toFixed(b >= 10 * 1073741824 ? 0 : 1);
function rate(bps) {
  const u = ["B/s", "KB/s", "MB/s", "GB/s"]; let i = 0;
  while (bps >= 1000 && i < u.length - 1) { bps /= 1024; i++; }
  return [bps >= 100 || i === 0 ? Math.round(bps).toString() : bps.toFixed(1), u[i]];
}
function uptime(s) { const d = Math.floor(s / 86400), h = Math.floor(s % 86400 / 3600), m = Math.floor(s % 3600 / 60); return (d ? d + "d " : "") + h + "h " + m + "m"; }

// temperature → color (cool cyan → green → yellow → orange → red)
function tcol(t) {
  if (t == null) return css("--dim");
  if (t < 45) return css("--cyan");
  if (t < 65) return css("--green");
  if (t < 78) return css("--yellow");
  if (t < 88) return css("--orange");
  return css("--red");
}
function lcol(p) {
  if (p == null) return css("--dim");
  if (p < 40) return css("--cyan");
  if (p < 70) return css("--green");
  if (p < 90) return css("--yellow");
  return css("--orange");
}

function spark(id, data, color, opts = {}) {
  const c = $(id); const dpr = window.devicePixelRatio || 1;
  const w = c.clientWidth, h = c.clientHeight;
  if (c.width !== w * dpr || c.height !== h * dpr) { c.width = w * dpr; c.height = h * dpr; }
  const g = c.getContext("2d"); g.setTransform(dpr, 0, 0, dpr, 0, 0); g.clearRect(0, 0, w, h);
  const pts = data.filter((v) => v != null && isFinite(v));
  if (pts.length < 2) return;
  let lo = opts.min ?? Math.min(...pts), hi = opts.max ?? Math.max(...pts);
  if (opts.min == null || opts.max == null) { const pad = Math.max((hi - lo) * 0.15, opts.span ?? 1); lo -= pad; hi += pad; }
  const x = (i) => (i / (HIST - 1)) * w, off = HIST - data.length;
  const y = (v) => h - 2 - ((v - lo) / (hi - lo || 1)) * (h - 4);
  g.beginPath(); let started = false;
  data.forEach((v, i) => { if (v == null) return; const X = x(i + off), Y = y(v); started ? g.lineTo(X, Y) : g.moveTo(X, Y); started = true; });
  g.strokeStyle = color; g.lineWidth = 1.6; g.lineJoin = "round"; g.stroke();
  g.lineTo(x(HIST - 1), h); g.lineTo(x(off), h); g.closePath();
  const gr = g.createLinearGradient(0, 0, 0, h); gr.addColorStop(0, color + "55"); gr.addColorStop(1, color + "00");
  g.fillStyle = gr; g.fill();
}

// ---------- CPU sprite ----------
function coreTile(c) {
  const hasT = c.temp != null;
  const col = hasT ? tcol(c.temp) : lcol(c.load);
  return `<div class="core" style="--c:${col}"><span class="cn">C${c.idx}</span><span class="cl">${f0(c.load)} %</span>
    <div class="ct">${hasT ? f0(c.temp) + "°" : f0(c.load) + "%"}</div>
    <div class="cf">${(c.mhz / 1000).toFixed(2)} GHz</div><div class="cb"><i style="width:${Math.min(100, c.load)}%"></i></div></div>`;
}
function renderCpu(cpu, mem) {
  $("cpu-name").textContent = "CPU · " + cpu.name.replace(/\(R\)|\(TM\)|CPU|@.*$/g, "").replace(/\s+/g, " ").trim();
  $("die-lbl").textContent = `${cpu.cores.length} NÚCLEOS · ${(cpu.mhz_avg / 1000).toFixed(2)} GHz`;
  const L = [], R = [];
  cpu.cores.forEach((c, i) => (i % 2 ? R : L).push(coreTile(c)));
  $("col-l").innerHTML = L.join(""); $("col-r").innerHTML = R.join("");
  $("ring-lbl").textContent = `Ring / L3 · ${(cpu.mhz_avg / 1000).toFixed(2)} GHz`;
  const s = (l, v, col, sub) => `<div><div class="sl">${l}</div><div class="sv" ${col ? `style="color:${col}"` : ""}>${v}</div>${sub ? `<div class="ss">${sub}</div>` : ""}</div>`;
  $("cpu-side").innerHTML = [
    s("CARGA", f0(cpu.load) + " %", lcol(cpu.load)),
    s("PACOTE", cpu.package_temp != null ? f0(cpu.package_temp) + " °C" : "—", tcol(cpu.package_temp), cpu.package_temp == null ? "precisa do HWiNFO" : ""),
    s("CORE MAX", cpu.core_max != null ? f0(cpu.core_max) + " °C" : "—", tcol(cpu.core_max)),
    s("POTÊNCIA", cpu.power != null ? f0(cpu.power) + " W" : "—"),
    s("VCORE", cpu.vcore != null ? cpu.vcore.toFixed(3) + " V" : "—"),
    s("RAM", `${gb(mem.used)} / ${gb(mem.total)} GB`, null, `${f0(mem.used / mem.total * 100)} % em uso`),
  ].join("");
}

// ---------- GPU sprite ----------
const MEMCHIPS = { top: ["M0", "M1"], l: ["M2", "M3"], r: ["M4", "M5"] };
function renderGpu(g) {
  const short = (g.name || "GPU").replace(/NVIDIA\s*/i, "");
  $("gpu-name").textContent = "GPU · " + short;
  $("gdie-name").textContent = /1660|1650/.test(short) ? "TU116" : "GPU";
  $("gdie").style.setProperty("--c", tcol(g.temp));
  $("gdie-t").textContent = g.temp != null ? g.temp + "°" : "—";
  $("gdie-s").textContent = g.clock != null ? (g.clock / 1000).toFixed(2) + " GHz" : "—";
  $("gdie-s2").textContent = `${f0(g.load)} % carga`;
  // VRAM chips light up in order as VRAM fills
  const used = g.vram_total ? g.vram_used / g.vram_total : 0;
  const all = [...MEMCHIPS.top, ...MEMCHIPS.l, ...MEMCHIPS.r];
  const per = g.vram_total ? g.vram_total / all.length : 0;
  const chip = (n) => {
    const i = all.indexOf(n), fill = Math.max(0, Math.min(1, used * all.length - i));
    const col = fill > 0 ? lcol(g.mem_load) : css("--dim");
    return `<div class="chip" style="--c:${col};opacity:${0.45 + fill * 0.55}"><b>${n}</b><span>${f0(fill * 100)}%</span></div>`;
  };
  $("mem-top").innerHTML = MEMCHIPS.top.map(chip).join("");
  $("mem-l").innerHTML = MEMCHIPS.l.map(chip).join("");
  $("mem-r").innerHTML = MEMCHIPS.r.map(chip).join("");
  $("mem-cap").textContent = `GDDR6 · ${g.vram_used != null ? gb(g.vram_used) : "—"} / ${g.vram_total != null ? gb(g.vram_total) : "—"} GB · ${g.mem_clock != null ? g.mem_clock + " MHz" : "—"} · ${f0(g.mem_load)} % carga`;
  // VRM phases glow with power draw
  const pf = g.power_w && g.power_limit_w ? g.power_w / g.power_limit_w : 0;
  const lit = Math.round(pf * 6);
  let ch = "";
  for (let i = 0; i < 6; i++) {
    const on = i < lit, c = on ? lcol(pf * 100) : "";
    ch += `<span class="ph" style="${on ? `background:${c}` : ""}"></span><span class="ch2" style="${on ? `background:${c}55` : ""}"></span><span class="ph" style="${on ? `background:${c}` : ""}"></span>`;
  }
  $("chokes").innerHTML = ch;
  $("vrm-w").textContent = g.power_w != null ? f0(g.power_w) + " W" : "—";
  $("pc-w").textContent = g.power_w != null ? f0(g.power_w) + " W" : "—";
  $("pc-s").textContent = g.power_limit_w != null ? `limite ${f0(g.power_limit_w)} W` : "";
  $("fan-lbl").textContent = g.fan != null ? `fan ${g.fan} %` : "fan —";
  $("fanblades").style.setProperty("--spd", g.fan ? (3.2 - g.fan / 100 * 2.9).toFixed(2) + "s" : "0s");
  $("board-cap").textContent = g.error ? g.error
    : `Placa ${f0(g.power_w)} W de ${f0(g.power_limit_w)} W · ${f0(pf * 100)} % do limite`;
  $("gpu-tag2").textContent = g.vram_total ? `${gb(g.vram_total)} GB` : "";
}

function rows(el, items, empty) {
  $(el).innerHTML = items.length ? items.join("") : `<div class="empty">${empty}</div>`;
}

let rateMs = 1000;
function render(t) {
  const { cpu, gpu, mem, net } = t;
  const memPct = mem.total ? (mem.used / mem.total) * 100 : null;
  const vramPct = gpu.vram_total ? (gpu.vram_used / gpu.vram_total) * 100 : null;
  push("cpu", cpu.load); push("gpu", gpu.load); push("mem", memPct); push("net", net.rx_bps);
  push("cput", cpu.package_temp); push("gput", gpu.temp);

  $("host").textContent = t.host || "PC";
  $("summary").textContent = `CPU ${f0(cpu.load)} % · GPU ${f0(gpu.load)} % · RAM ${f0(memPct)} % · VRAM ${f0(vramPct)} % · ligado há ${uptime(t.uptime)}`;
  $("status").innerHTML = t.hwinfo ? `<b>HWiNFO</b> offline · temperaturas da CPU indisponíveis` : `<b>HWiNFO</b> ok · NVML ${gpu.error ? "offline" : "ok"}`;

  // row 1
  $("cpu-load").textContent = f0(cpu.load);
  $("cpu-sub").textContent = `${(cpu.mhz_avg / 1000).toFixed(2)} GHz · ${cpu.cores.length} núcleos` + (cpu.package_temp != null ? ` · ${f0(cpu.package_temp)} °C` : "");
  $("gpu-load").textContent = f0(gpu.load);
  $("gpu-sub").textContent = `${gpu.temp ?? "—"} °C · ${f0(gpu.power_w)} W · ${gpu.clock ?? "—"} MHz`;
  $("mem-pct").textContent = f0(memPct);
  $("mem-sub").textContent = `${gb(mem.used)} de ${gb(mem.total)} GB · swap ${gb(mem.swap_used)} GB`;
  const [rx, rxu] = rate(net.rx_bps), [tx, txu] = rate(net.tx_bps);
  $("net-rx").textContent = rx; $("net-rx-u").textContent = " " + rxu;
  $("net-sub").textContent = `upload ${tx} ${txu}`;
  spark("sp-cpu", hist.cpu, css("--indigo"), { min: 0, max: 100 });
  spark("sp-gpu", hist.gpu, css("--green"), { min: 0, max: 100 });
  spark("sp-mem", hist.mem, css("--orange"), { min: 0, max: 100 });
  spark("sp-net", hist.net, css("--cyan"), { min: 0, span: 1024 });

  // row 2
  $("cpu-temp").innerHTML = cpu.package_temp != null ? `${f1(cpu.package_temp)}<small>°</small>` : "—";
  $("cpu-clk").innerHTML = `${(cpu.mhz_avg / 1000).toFixed(2)}<small>GHz</small>`;
  $("cpu-pwr").innerHTML = cpu.power != null ? `${f0(cpu.power)}<small>W</small>` : "—";
  $("cpu-src").textContent = t.hwinfo ? "HWiNFO offline" : "HWiNFO";
  $("cpu-foot").textContent = t.hwinfo ? "Abra o HWiNFO com “Shared Memory Support” para ver temperatura, potência e fans." : `core max ${f0(cpu.core_max)} °C · vcore ${cpu.vcore != null ? cpu.vcore.toFixed(3) + " V" : "—"}`;
  $("gpu-temp").innerHTML = gpu.temp != null ? `${gpu.temp}<small>°</small>` : "—";
  $("gpu-fan").innerHTML = gpu.fan != null ? `${gpu.fan}<small>%</small>` : "—";
  $("gpu-pwr").innerHTML = gpu.power_w != null ? `${f0(gpu.power_w)}<small>W</small>` : "—";
  $("gpu-foot").textContent = gpu.error || `core ${gpu.clock ?? "—"} MHz · mem ${gpu.mem_clock ?? "—"} MHz · VRAM ${gpu.vram_used != null ? gb(gpu.vram_used) : "—"} / ${gpu.vram_total != null ? gb(gpu.vram_total) : "—"} GB`;

  // row 3
  $("cpu-t2").textContent = cpu.package_temp != null ? f1(cpu.package_temp) : "—";
  $("cpu-t2-sub").textContent = cpu.package_temp != null ? `máx. ${f0(Math.max(...hist.cput.filter((v) => v != null)))} °C nos últimos ${Math.round(HIST * rateMs / 60000)} min` : "sem HWiNFO";
  $("gpu-t2").textContent = gpu.temp ?? "—";
  const gt = hist.gput.filter((v) => v != null);
  $("gpu-t2-sub").textContent = gt.length ? `mín. ${Math.min(...gt)} · máx. ${Math.max(...gt)} °C` : "—";
  spark("sp-cput", hist.cput, css("--indigo"), { span: 2 });
  spark("sp-gput", hist.gput, css("--green"), { span: 2 });
  const dTot = t.disks.reduce((a, d) => a + d.total, 0), dUsed = t.disks.reduce((a, d) => a + d.used, 0);
  $("disk-tag").textContent = `${gb(dUsed)} / ${gb(dTot)} GB`;
  rows("disks", t.disks.map((d) => {
    const p = d.total ? d.used / d.total * 100 : 0;
    const col = p > 90 ? css("--red") : p > 75 ? css("--orange") : css("--yellow");
    return `<div class="row"><div class="stack"><div style="display:flex;gap:8px"><span class="k">${d.mount} · ${d.name || d.kind}</span><span class="v">${gb(d.used)} / ${gb(d.total)} GB</span></div><div class="bar" style="--c:${col}"><i style="width:${p}%"></i></div></div></div>`;
  }), "nenhum disco");

  // row 4
  renderCpu(cpu, mem);
  renderGpu(gpu);

  // row 5
  rows("procs", t.procs.map((p) => `<div class="row"><span class="k">${p.name.replace(/[<>&]/g, "")}</span><span class="v">${f1(p.cpu)} % · ${(p.mem / 1048576).toFixed(0)} MB</span></div>`), "—");
  $("fans-tag").textContent = t.hwinfo ? "HWiNFO offline" : "HWiNFO";
  const fanRows = t.fans.map((f) => `<div class="row"><span class="k">${f.label}</span><span class="v">${f0(f.rpm)} rpm</span></div>`);
  if (gpu.fan != null) fanRows.unshift(`<div class="row"><span class="k">GPU fan</span><span class="v">${gpu.fan} %</span></div>`);
  rows("fans", fanRows, "Abra o HWiNFO para ver as ventoinhas da placa-mãe.");
}

// refresh-rate switch
document.querySelectorAll("#rate button").forEach((b) => b.addEventListener("click", () => {
  document.querySelectorAll("#rate button").forEach((x) => x.classList.toggle("on", x === b));
  rateMs = +b.dataset.ms;
  if (window.__TAURI__) window.__TAURI__.core.invoke("set_interval", { ms: rateMs });
}));

if (window.__TAURI__) {
  window.__TAURI__.event.listen("telemetry", (e) => render(e.payload));
} else {
  // Browser preview: fake data so the layout can be checked without the app.
  let k = 0;
  setInterval(() => {
    k++;
    const w = (a, p) => a * (0.5 + 0.5 * Math.sin(k / p));
    render({
      host: "DEMO-PC", uptime: 12345 + k, hwinfo: k % 40 < 20 ? null : "offline",
      cpu: { name: "Intel(R) Core(TM) i7-9700 CPU @ 3.00GHz", load: 10 + w(60, 7), mhz_avg: 4300, package_temp: 40 + w(30, 9), core_max: 45 + w(30, 9), power: 20 + w(60, 9), vcore: 1.2,
        cores: [...Array(8)].map((_, i) => ({ idx: i, load: 5 + w(90, 3 + i), mhz: 4500 - i * 30, temp: 38 + w(40, 4 + i) })) },
      gpu: { name: "NVIDIA GeForce GTX 1660 SUPER", temp: Math.round(45 + w(30, 11)), load: Math.round(w(100, 6)), mem_load: Math.round(w(60, 6)), vram_used: 2.5e9 + w(3e9, 13), vram_total: 6.4e9,
        power_w: 15 + w(110, 6), power_limit_w: 125, clock: 1530 + Math.round(w(400, 6)), mem_clock: 7000, fan: Math.round(30 + w(50, 11)) },
      mem: { used: 8.6e9 + w(2e9, 10), total: 17.1e9, swap_used: 1e9, swap_total: 4e9 },
      disks: [{ name: "SSD 240GB", mount: "C:", kind: "SSD", used: 2.1e11, total: 2.4e11 }, { name: "ST2000LM007", mount: "D:", kind: "HDD", used: 1.2e12, total: 2e12 }],
      net: { rx_bps: w(5e6, 4), tx_bps: w(4e5, 5) },
      procs: [{ name: "VRChat", cpu: 22.4, mem: 3.2e9 }, { name: "chrome", cpu: 6.1, mem: 1.1e9 }, { name: "Discord", cpu: 1.2, mem: 4e8 }],
      fans: [{ label: "CPU", rpm: 1100 }, { label: "Chassis1", rpm: 870 }],
    });
  }, 1000);
}
