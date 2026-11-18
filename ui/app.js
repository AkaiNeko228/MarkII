"use strict";
const $ = (id) => document.getElementById(id);
const TAURI = window.__TAURI__;
const invoke = (cmd, args) => (TAURI ? TAURI.core.invoke(cmd, args) : Promise.resolve(null));
const HIST = 120;
const hist = { cpu: [], gpu: [], mem: [], net: [], cput: [], gput: [] };
const push = (k, v) => { const a = hist[k]; a.push(v); if (a.length > HIST) a.shift(); };

const css = (n) => getComputedStyle(document.documentElement).getPropertyValue(n).trim();
const esc = (s) => String(s ?? "").replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));
const f0 = (v) => (v == null || !isFinite(v) ? "—" : Math.round(v).toString());
const f1 = (v) => (v == null || !isFinite(v) ? "—" : v.toFixed(1));
const gb = (b) => (b / 1073741824).toFixed(b >= 10 * 1073741824 ? 0 : 1);
function rate(bps) {
  const u = ["B/s", "KB/s", "MB/s", "GB/s"]; let i = 0;
  while (bps >= 1000 && i < u.length - 1) { bps /= 1024; i++; }
  return [bps >= 100 || i === 0 ? Math.round(bps).toString() : bps.toFixed(1), u[i]];
}
function uptime(s) { const d = Math.floor(s / 86400), h = Math.floor(s % 86400 / 3600), m = Math.floor(s % 3600 / 60); return (d ? d + "d " : "") + h + "h " + m + "m"; }
function tcol(t) {
  if (t == null) return css("--dim");
  return t < 45 ? css("--cyan") : t < 65 ? css("--green") : t < 78 ? css("--yellow") : t < 88 ? css("--orange") : css("--red");
}
function lcol(p) {
  if (p == null) return css("--dim");
  return p < 40 ? css("--cyan") : p < 70 ? css("--green") : p < 90 ? css("--yellow") : css("--orange");
}
const segbar = (p, n = 10) => { const on = Math.round(Math.max(0, Math.min(100, p || 0)) / 100 * n); return [...Array(n)].map((_, i) => `<i class="${i < on ? "on" : ""}"></i>`).join(""); };

function canvas(id) {
  const c = $(id), dpr = window.devicePixelRatio || 1, w = c.clientWidth, h = c.clientHeight;
  if (c.width !== w * dpr || c.height !== h * dpr) { c.width = w * dpr; c.height = h * dpr; }
  const g = c.getContext("2d"); g.setTransform(dpr, 0, 0, dpr, 0, 0); g.clearRect(0, 0, w, h);
  return [g, w, h];
}
// Column chart for the vitals strip (last 40 samples).
function bars(id, data, color, max) {
  const [g, w, h] = canvas(id);
  const n = 40, d = data.slice(-n), bw = w / n;
  const top = max ?? Math.max(1, ...d.filter((v) => v != null));
  d.forEach((v, i) => {
    if (v == null) return;
    const bh = Math.max(1.5, (v / top) * (h - 2));
    g.fillStyle = i === d.length - 1 ? color : color + "88";
    g.fillRect((n - d.length + i) * bw + 0.5, h - bh, Math.max(1, bw - 1.5), bh);
  });
}
// Multi-line chart with a light grid for thermals.
function lines(id, series) {
  const [g, w, h] = canvas(id);
  const all = series.flatMap((s) => s.data).filter((v) => v != null);
  g.strokeStyle = css("--line"); g.lineWidth = 1; g.font = "10px Bahnschrift, sans-serif"; g.fillStyle = css("--dim");
  if (!all.length) return;
  let lo = Math.floor((Math.min(...all) - 3) / 10) * 10, hi = Math.ceil((Math.max(...all) + 3) / 10) * 10;
  if (hi - lo < 20) hi = lo + 20;
  const y = (v) => h - 4 - ((v - lo) / (hi - lo)) * (h - 8);
  for (let v = lo; v <= hi; v += 10) { g.beginPath(); g.moveTo(26, y(v) + .5); g.lineTo(w, y(v) + .5); g.stroke(); g.fillText(v + "°", 0, y(v) + 3); }
  const x = (i, len) => 28 + ((i + HIST - len) / (HIST - 1)) * (w - 28);
  for (const s of series) {
    g.beginPath(); let st = false;
    s.data.forEach((v, i) => { if (v == null) return; st ? g.lineTo(x(i, s.data.length), y(v)) : g.moveTo(x(i, s.data.length), y(v)); st = true; });
    g.strokeStyle = s.color; g.lineWidth = 2; g.lineJoin = "round"; g.stroke();
  }
}

// ---------- shared bits ----------
const opt = (v, fmt) => (v == null || !isFinite(v) ? "—" : fmt(v));
const needs = (t) => (!t.source || t.lhm_state === "limited" ? "precisa dos sensores completos" : "");
function cell(k, v, col, note) {
  const miss = v === "—";
  return `<div class="cell"><div class="sl">${k}</div><b class="${miss ? "dim" : ""}" ${!miss && col ? `style="color:${col}"` : ""}>${v}</b>${note ? `<span>${esc(note)}</span>` : ""}</div>`;
}

// ---------- brands ----------
let brands = null, brandsGpu = null;
// logos are the app's own embedded SVG files (Simple Icons, CC0)
function mark(b, size) {
  if (!b) return "";
  if (b.svg) return `<span class="bmark" style="--s:${size}px">${b.svg}${b.tag ? `<b style="color:${b.color}">${esc(b.tag)}</b>` : ""}</span>`;
  return b.name ? `<span class="bbadge" style="--c:${b.color}">${esc(b.name)}</span>` : "";
}
function loadBrands(gpu) {
  brandsGpu = gpu;
  invoke("get_brands", { gpu }).then((b) => {
    if (!b) return;
    brands = b;
    $("cpu-logo").innerHTML = mark(b.cpu, 28);
    $("gpu-logo").innerHTML = mark(b.gpu, 28);
    $("board-logo").innerHTML = mark(b.board, 28);
    if (lastT) { renderCpu(lastT); renderRam(lastT); renderBoard(lastT); }
  });
}

// Windows numbers the two threads of an SMT core next to each other, SMT cores first.
function threadMap(cores, threads) {
  if (!cores || !threads) return [];
  cores = Math.min(cores, threads);
  const extra = Math.min(threads - cores, cores);
  let t = 0;
  return [...Array(cores)].map((_, c) => { const n = c < extra ? 2 : 1, v = [...Array(n)].map((_, k) => t + k); t += n; return v; });
}

// ---------- CPU ----------
let cpuView = "cores";
function threadBars(ths) {
  return `<div class="tbars">${ths.map((th) => `<div class="tbar" style="--c:${lcol(th.load)}"><i>T${th.idx}</i><div class="segs">${segbar(th.load, 8)}</div><em>${f0(th.load)}%</em></div>`).join("")}</div>`;
}
function coreTile(c, threads, hybrid, ths) {
  const hasT = c.temp != null && !threads, col = hasT ? tcol(c.temp) : lcol(c.load);
  const tag = threads ? `T${c.idx}` : hybrid ? `${c.kind}${c.idx}` : `C${c.idx}`;
  const sub = `${(c.mhz / 1000).toFixed(2)} GHz` + (!threads && c.volt != null ? ` · ${c.volt.toFixed(2)} V` : "");
  return `<div class="core" style="--c:${col}"><span class="cn">${tag}</span><span class="cl">${f0(c.load)}%</span>
    <div class="ct">${hasT ? f0(c.temp) + "°" : f0(c.load) + "%"}</div><div class="cf">${sub}</div>
    ${ths ? threadBars(ths) : `<div class="cb">${segbar(c.load)}</div>`}</div>`;
}
function renderCpu(t) {
  const cpu = t.cpu;
  const short = cpu.name.replace(/\(R\)|\(TM\)|CPU|@.*$/g, "").replace(/\s+/g, " ").trim();
  const gen = (brands?.cpu?.line || "").includes(" · ") ? brands.cpu.line.split(" · ").pop() : "";
  $("cpu-name").textContent = gen ? `${short} · ${gen}` : short;
  const physOk = cpu.phys && cpu.phys.length > 0;
  const both = cpuView === "both";
  const threads = cpuView === "threads" || (!physOk && !both);
  document.querySelectorAll("#cpuview button").forEach((b) => b.classList.toggle("on", b.dataset.v === (both ? "both" : threads ? "threads" : "cores")));
  const ncores = physOk ? cpu.phys.length : pcInfo?.cpu_cores || 0;
  const tmap = threadMap(ncores, cpu.cores.length);
  // without sensors, build each core from its threads
  const synth = () => tmap.map((ts, i) => { const th = ts.map((x) => cpu.cores[x]).filter(Boolean); return { idx: i, kind: "", load: th.reduce((a, x) => a + x.load, 0) / Math.max(1, th.length), mhz: Math.max(0, ...th.map((x) => x.mhz)), temp: null, volt: null }; });
  const list = threads ? cpu.cores : physOk ? cpu.phys : synth();
  const hybrid = physOk && cpu.phys.some((c) => c.kind === "E");
  $("die-lbl").textContent = `${ncores || cpu.cores.length} NÚCLEOS · ${cpu.threads || cpu.cores.length} THREADS`;
  $("cores").style.setProperty("--cols", list.length <= 8 ? 2 : list.length <= 24 ? 4 : 6);
  $("cores").innerHTML = list.map((c) => coreTile(c, threads, hybrid, both ? (tmap[c.idx] || []).map((x) => cpu.cores[x]).filter(Boolean) : null)).join("");
  $("cpu-note").hidden = physOk || cpuView !== "cores";
  $("cpu-note").textContent = "Os núcleos físicos aparecem quando os sensores completos estão ativos; mostrando threads.";
  const need = needs(t);
  const ghz = (m) => (m ? (m / 1000).toFixed(2) : "—");
  const s = (l, v, col, sub) => `<div><div class="sl">${l}</div><div class="sv ${v === "—" ? "dim" : ""}" ${col && v !== "—" ? `style="color:${col}"` : ""}>${v}</div>${sub ? `<div class="ss">${esc(sub)}</div>` : ""}</div>`;
  $("cpu-side").innerHTML = [
    s("CARGA", f0(cpu.load) + " %", lcol(cpu.load)),
    s("PACOTE", opt(cpu.package_temp, (v) => f0(v) + " °C"), tcol(cpu.package_temp), cpu.package_temp == null ? need : ""),
    s("CORE MAX", opt(cpu.core_max, (v) => f0(v) + " °C"), tcol(cpu.core_max)),
    s("POTÊNCIA", opt(cpu.power, (v) => v.toFixed(1) + " W"), null, cpu.cores_power != null ? `núcleos ${cpu.cores_power.toFixed(1)} W` : cpu.power == null ? need : ""),
    s("VCORE", opt(cpu.vcore, (v) => v.toFixed(3) + " V"), css("--yellow"), cpu.vcore != null && cpu.vid != null ? `VID pedido ${cpu.vid.toFixed(3)} V` : ""),
    s("CLOCK AGORA", `${ghz(cpu.mhz_min)}–${ghz(cpu.mhz_max)} GHz`, null, `sessão ${ghz(cpu.session_min)}–${ghz(cpu.session_max)} GHz`),
    cpu.bus_mhz != null ? s("BARRAMENTO", cpu.bus_mhz.toFixed(1) + " MHz") : "",
  ].join("");
}

// ---------- GPU (compact) ----------
function renderGpu(t) {
  const g = t.gpu;
  if (pcInfo && g.name && brandsGpu !== g.name) loadBrands(g.name);
  const short = (g.name || "GPU").replace(/NVIDIA\s*|AMD\s*/i, "");
  $("gpu-name").textContent = short;
  $("gpu-tag2").textContent = g.vram_total ? `${gb(g.vram_total)} GB` : "";
  $("gdie").style.setProperty("--c", tcol(g.temp));
  $("gdie-t").textContent = g.temp != null ? g.temp + "°" : "—";
  $("gdie-s").textContent = g.clock != null ? (g.clock / 1000).toFixed(2) + " GHz" : "—";
  const used = g.vram_total ? g.vram_used / g.vram_total : 0;
  // one memory chip per 32 bits of bus width
  const n = g.bus_width ? Math.max(1, Math.min(16, Math.round(g.bus_width / 32))) : 4, half = Math.ceil(n / 2);
  const chip = (i) => {
    const fill = Math.max(0, Math.min(1, used * n - i)), col = fill > 0 ? lcol(g.mem_load) : css("--dim");
    return `<i class="mchip" style="--c:${col};--f:${Math.round(12 + fill * 35)}%"></i>`;
  };
  $("mem-top").innerHTML = [...Array(half)].map((_, i) => chip(i)).join("");
  $("mem-bot").innerHTML = [...Array(n - half)].map((_, i) => chip(half + i)).join("");
  $("g-use").textContent = g.load != null ? g.load + " %" : "—";
  $("g-use").style.color = lcol(g.load);
  $("g-bar").innerHTML = segbar(g.load, 20); $("g-bar").style.setProperty("--c", css("--green"));
  $("g-vram").textContent = `VRAM ${g.vram_used != null ? gb(g.vram_used) : "—"} / ${g.vram_total != null ? gb(g.vram_total) : "—"} GB · ${n} chips${g.bus_width ? ` · ${g.bus_width}-bit` : ""}`;
  $("fan-lbl").textContent = g.fan != null && g.fan_rpm != null ? `${g.fan}% · ${f0(g.fan_rpm)} rpm` : g.fan != null ? `${g.fan}%` : g.fan_rpm != null ? `${f0(g.fan_rpm)} rpm` : "fan —";
  $("fanblades").style.setProperty("--spd", g.fan ? (3.2 - g.fan / 100 * 2.9).toFixed(2) + "s" : "0s");
  const no = "sem sensor nesta placa", T = (v) => opt(v, (x) => f0(x) + " °C");
  $("gpu-cells").innerHTML = [
    cell("Temperatura", T(g.temp), tcol(g.temp)),
    cell("Hotspot", T(g.hotspot), tcol(g.hotspot), g.hotspot == null ? needs(t) || no : ""),
    cell("Memória", T(g.mem_temp), tcol(g.mem_temp), g.mem_temp == null ? no : ""),
    cell("VRM", T(g.vrm_temp), tcol(g.vrm_temp), g.vrm_temp == null ? no : ""),
    cell("Consumo", opt(g.power_w, (v) => f0(v) + " W"), null, g.power_limit_w != null ? `limite ${f0(g.power_limit_w)} W` : ""),
    cell("Tensão", opt(g.voltage, (v) => v.toFixed(3) + " V"), css("--yellow"), g.voltage == null ? needs(t) : ""),
    cell("Clock memória", g.mem_clock != null ? g.mem_clock + " MHz" : "—", null, g.mem_load != null ? `controlador ${g.mem_load}%` : ""),
    cell("VRAM", g.vram_used != null ? gb(g.vram_used) + " GB" : "—", null, `${f0(used * 100)}% de ${g.vram_total != null ? gb(g.vram_total) : "—"} GB`),
  ].join("");
  $("gpu-err").hidden = !g.error; $("gpu-err").textContent = g.error || "";
}

// ---------- RAM ----------
const RAMTYPE = { 34: "DDR5", 26: "DDR4", 24: "DDR3", 30: "LPDDR4", 35: "LPDDR5" };
const cyc = (ns, mts) => (ns == null || !mts ? null : Math.max(1, Math.ceil(ns / (2000 / mts) - 0.025)));
let ramLayout = null;
function renderRam(t) {
  const mem = t.mem, p = mem.total ? mem.used / mem.total * 100 : 0;
  const mods = pcInfo?.ram || [], m0 = mods[0];
  const mts = m0 ? m0.configured || m0.speed : 0, kind = (m0 && RAMTYPE[m0.kind]) || "RAM";
  const spd = (t.hw.spd || [])[0];
  const cl = cyc(spd?.taa, mts), tm = [cl, cyc(spd?.trcd, mts), cyc(spd?.trp, mts), cyc(spd?.tras, mts)];
  const volt = m0 && m0.mv ? m0.mv / 1000 : t.hw.board?.dram_v;
  const temps = (t.hw.spd || []).map((s) => s.temp).filter((v) => v != null), temp = temps.length ? Math.max(...temps) : null;
  $("ram-tag").textContent = `${kind} · ${gb(mem.total)} GB`;
  $("ram-pct").textContent = f0(p) + "%"; $("ram-pct").style.color = lcol(p);
  $("ram-sub").textContent = `${gb(mem.used)} de ${gb(mem.total)} GB em uso · virtual ${gb(mem.swap_used)} / ${gb(mem.swap_total)} GB`;
  $("ram-bar").innerHTML = segbar(p, 30); $("ram-bar").style.setProperty("--c", css("--amber"));
  const slots = pcInfo?.ram_slots || 0;
  $("ram-cells").innerHTML = [
    cell("Frequência", mts ? `${mts} MT/s` : "—", null, mts ? `${Math.round(mts / 2)} MHz reais · ${kind}` : ""),
    cell("Latência", cl ? `CL${cl}` : "—", css("--cyan"), tm.every((x) => x) ? `${tm.join("-")} · ${(cl * 2000 / mts).toFixed(2)} ns` : needs(t)),
    cell("Tensão", opt(volt, (v) => v.toFixed(2) + " V"), css("--yellow")),
    cell("Temperatura", opt(temp, (v) => f0(v) + " °C"), tcol(temp), temp == null ? "o pente não tem sensor" : ""),
    cell("Consumo", opt(t.cpu.mem_power, (v) => v.toFixed(1) + " W"), null, t.cpu.mem_power != null ? "controlador de memória" : ""),
    cell("Canais", ramLayout ? ramLayout.mode.replace(" channel", "") : "—", null, `${mods.length} de ${Math.max(slots, mods.length)} slots`),
  ].join("");
  $("ram-mods").innerHTML = mods.map((m, i) => {
    const s = (t.hw.spd || [])[i], rfc = cyc(s?.trfc, mts);
    const sp = m.configured || m.speed;
    const b = brands?.ram?.[i];
    return `<div class="row">${mark(b, 18)}<span class="k">${esc(m.slot)} · ${f0(m.size_gb)} GB ${b?.name ? esc(b.name) + " " : ""}${esc(m.part)}</span><span class="v">${sp} MT/s${m.speed > m.configured && m.configured ? ` (suporta ${m.speed})` : ""}${rfc ? ` · tRFC ${rfc}` : ""}</span></div>`;
  }).join("");
}

// ---------- motherboard ----------
function renderBoard(t) {
  const b = t.hw.board || {}, fans = t.hw.fans || [];
  if (!pcInfo) return;
  $("board-name").textContent = `Placa-mãe · ${brands?.board?.name || pcInfo.board_maker} ${pcInfo.board}`;
  $("board-chip").textContent = b.chip || "chip de sensores não lido";
  $("slots").innerHTML = (ramLayout?.slots || []).map((s) => `<div class="slot ${s.module ? "on" : ""}"><i></i>${esc(s.name)}<small>${s.module ? f0(s.module.size_gb) + " GB" : "vazio"}</small></div>`).join("");
  const inUse = fans.filter((f) => f.rpm > 0).length;
  $("hdr-lbl").textContent = `CONECTORES DE FAN · ${inUse} DE ${b.headers || 0} EM USO`;
  $("hdrs").innerHTML = fans.length
    ? fans.map((f, i) => `<div class="hdr ${f.rpm > 0 ? "on" : ""}"><i><b></b><b></b><b></b><b></b></i>${/^Fan #/.test(f.label) ? "F" + (i + 1) : esc(f.label)}<span>${f.rpm > 0 ? f0(f.rpm) : "—"}</span></div>`).join("")
    : `<div class="empty">sem leitura do chip da placa</div>`;
  $("rgb").innerHTML = pcInfo.rgb_devices?.length
    ? `<div class="chips">${pcInfo.rgb_devices.map((d) => `<span class="rgbdev">${esc(d)}</span>`).join("")}</div>`
    : `<div class="empty" style="padding:0">nenhum controlador RGB por USB encontrado</div>`;
  const T = (v) => opt(v, (x) => f0(x) + " °C"), no = "a placa não tem esse sensor";
  const measured = (t.cpu.power || 0) + (t.gpu.power_w || 0);
  $("board-cells").innerHTML = [
    cell("VRM da CPU", T(b.vrm), tcol(b.vrm), b.vrm == null ? needs(t) || no : ""),
    cell("VRM da memória", T(b.vrm_mem), tcol(b.vrm_mem), b.vrm_mem == null ? needs(t) || no : ""),
    cell("Chipset", T(b.pch), tcol(b.pch), b.pch == null ? no : ""),
    cell("Consumo da placa", opt(b.power, (v) => f0(v) + " W"), null, b.power != null ? "sensor da placa" : measured ? `sem sensor · CPU+GPU medidos ${f0(measured)} W` : "sem sensor"),
  ].join("");
  const nt = (b.temps || []).filter((x) => !/^Temperature #/.test(x.label)), nv = (b.volts || []).filter((x) => !/^Voltage #/.test(x.label));
  $("board-chips").innerHTML = [...nt.map((x) => `<span><em>${esc(x.label)}</em>${f0(x.value)} °C</span>`), ...nv.map((x) => `<span><em>${esc(x.label)}</em>${x.value.toFixed(3)} V</span>`)].join("");
  const unnamed = (b.temps || []).length - nt.length + (b.volts || []).length - nv.length;
  $("board-note").textContent = unnamed > 0 ? `+${unnamed} leituras sem nome no chip (esta placa não identifica os canais)` : "";
}

// ---------- fans ----------
function renderFans(t) {
  const fans = t.hw.fans || [], g = t.gpu;
  const inUse = fans.filter((f) => f.rpm > 0).length, gpuFan = g.fan != null || g.fan_rpm != null, total = inUse + (gpuFan ? 1 : 0);
  $("fans-tag").textContent = fans.length ? `${total} girando · ${fans.length} conectores` : "sem leitura";
  $("fan-count").textContent = total;
  $("fan-count-l").textContent = total === 1 ? "ventoinha detectada" : "ventoinhas detectadas";
  const r = fans.map((f) => {
    const on = f.rpm > 0, pc = f.pct ?? 0, n = Math.round(Math.min(100, pc) / 10);
    return `<div class="row"><i class="fan-dot ${on ? "on" : ""}"></i><span class="k" style="${on ? "" : "color:var(--dim)"}">${esc(f.label)}</span>
      <span class="fanbar" style="--c:${on ? css("--cyan") : css("--dim")}">${[...Array(10)].map((_, i) => `<i class="${i < n ? "on" : ""}"></i>`).join("")}</span>
      <span class="v" style="width:120px;text-align:right">${on ? `${f0(f.rpm)} rpm · ${f.pct != null ? f0(f.pct) + "%" : "—"}` : "desconectado"}</span></div>`;
  });
  if (gpuFan) r.push(`<div class="row"><i class="fan-dot on"></i><span class="k">GPU</span><span class="v">${g.fan_rpm != null ? f0(g.fan_rpm) + " rpm · " : ""}${g.fan != null ? g.fan + "%" : ""}</span></div>`);
  $("fans").innerHTML = r.join("");
  $("fans-note").textContent = fans.length ? "Ventoinhas ligadas em hub/controladora aparecem como uma só no conector." : "Ative os sensores completos (PawnIO + administrador) para ler as ventoinhas da placa-mãe.";
}

// ---------- sensors banner ----------
let sensAction = null;
function renderSensors(t) {
  let msg = "", act = null;
  if ((t.lhm_state === "running" || t.lhm_state === "off") && t.pawnio) msg = "";
  else if (t.lhm_state === "starting" || t.lhm_state === "installing") msg = t.lhm_msg;
  else if (!t.pawnio) { msg = "Sensores limitados: falta o driver PawnIO para ler CPU (potência, Vcore), ventoinhas e placa-mãe."; act = ["Instalar PawnIO", "install"]; }
  else if (t.lhm_state === "limited") { msg = "Rodando sem administrador: GPU e discos funcionam, mas CPU, ventoinhas e placa-mãe precisam da permissão (UAC)."; act = ["Tentar com administrador", "admin"]; }
  else { msg = "Sensores: " + t.lhm_msg; act = ["Tentar de novo", "admin"]; }
  $("sensbar").hidden = !msg;
  $("sens-msg").textContent = msg;
  $("sens-act").hidden = !act;
  if (act) { $("sens-act").textContent = act[0]; sensAction = act[1]; }
  $("sens-state").textContent = `Estado: ${t.lhm_msg} · driver PawnIO ${t.pawnio ? "instalado" : "não instalado"}`;
}
$("sens-act").onclick = () => (sensAction === "install" ? invoke("sensors_install") : invoke("sensors_start", { admin: true }));
$("sens-restart").onclick = () => { invoke("sensors_start", { admin: $("c-admin").checked }); $("save-msg").textContent = "reiniciando sensores…"; };
document.querySelectorAll("#cpuview button").forEach((b) => b.addEventListener("click", () => {
  cpuView = b.dataset.v;
  if (cfg) { cfg.cpu_view = cpuView; invoke("save_config", { cfg }); }
  if (lastT) renderCpu(lastT);
}));

// ---------- PC identity ----------
const JEDEC = { "80CE": "Samsung", "CE00": "Samsung", "802C": "Micron", "2C00": "Micron", "80AD": "SK hynix", "AD00": "SK hynix", "859B": "Crucial", "04CD": "G.Skill", "029E": "Corsair", "0198": "Kingston", "9801": "Kingston", "04F1": "Teamgroup", "0783": "Transcend", "8551": "Qimonda", "017A": "ADATA", "04CB": "ADATA" };
function ramMaker(m) {
  if (/^[0-9A-F]{5}AA#?$/i.test(m.part)) return "HP";
  return JEDEC[(m.maker || "").toUpperCase()] || (m.maker && !/^[0-9A-F]{4}$/i.test(m.maker) ? m.maker : "");
}
let pcInfo = null;
function renderPc(pc) {
  pcInfo = pc;
  invoke("ram_layout").then((l) => { if (l) ramLayout = l; });
  loadBrands(lastT?.gpu?.name || "");
  if (!pc || pc.error) { $("pc-spec").innerHTML = `<div class="empty">${esc(pc?.error || "indisponível")}</div>`; $("pc-tag").textContent = "—"; return; }
  const generic = /^(to be filled|system manufacturer|default string|intel|)$/i.test(pc.maker.trim()) || pc.model === pc.board;
  const name = generic ? "PC montado" : `${pc.maker} ${pc.model}`;
  $("pc-tag").textContent = name;
  const ramTot = pc.ram.reduce((a, m) => a + m.size_gb, 0);
  const tips = [];
  if (pc.ram.length === 1) tips.push("RAM em <b>single channel</b>: um segundo pente igual ativa o dual channel e melhora o desempenho, principalmente em jogos.");
  const sysHdd = pc.disks.find((d) => d.media === "HDD" && /C:/.test(""));
  if (pc.disks.some((d) => d.bus === "USB")) tips.push("Há um disco conectado por <b>USB</b>; ele pode ser mais lento que os internos.");
  if (pc.cpu_max_mhz && pc.cpu_max_mhz < 3500) tips.push(`O clock de ${(pc.cpu_max_mhz / 1000).toFixed(1)} GHz é o <b>base</b>; o processador acelera sozinho (turbo) quando precisa.`);
  const row = (k, v) => `<dt>${k}</dt><dd>${v}</dd>`;
  $("pc-spec").innerHTML = [
    row("Sistema", `${esc(name)}${generic ? "" : ""} <small>· placa ${esc(pc.board_maker)} ${esc(pc.board)}</small>`),
    row("Windows", `${esc(pc.os.replace("Microsoft ", ""))} <small>· ${esc(pc.os_build)}</small>`),
    row("BIOS", esc(pc.bios)),
    row("CPU", `${esc(pc.cpu.replace(/\(R\)|\(TM\)/g, "").replace(/\s+/g, " "))}<br><small>${pc.cpu_cores} núcleos · ${pc.cpu_threads} threads · ${(pc.cpu_l3_kb / 1024).toFixed(0)} MB L3 · soquete ${esc(pc.cpu_socket)}</small>`),
    row("GPU", `${pc.gpus.map(esc).join(", ") || "—"}<br><small>driver ${esc(pc.gpu_driver)}</small>`),
    row("RAM", `${f0(ramTot)} GB · ${pc.ram.length} pente${pc.ram.length > 1 ? "s" : ""}<div class="chips">${pc.ram.map((m) => `<span>${esc(m.slot)} · ${f0(m.size_gb)} GB ${m.speed} MT/s ${esc(ramMaker(m))}</span>`).join("")}</div>`),
    row("Discos", `<div class="chips">${pc.disks.map((d) => `<span>${esc(d.model)} · ${d.size_gb >= 1000 ? (d.size_gb / 1024).toFixed(1) + " TB" : d.size_gb + " GB"} · ${esc(d.media === "Unspecified" ? d.bus : d.media)}</span>`).join("")}</div>`),
    tips.length ? row("Dicas", tips.map((t) => `<div>• ${t}</div>`).join("")) : "",
  ].join("");
  void sysHdd;
}

// ---------- cooling ----------
const FAN = (cx, cy, r, cls) => `<g class="${cls}" style="transform-origin:${cx}px ${cy}px">${[0, 72, 144, 216, 288].map((a) => `<path transform="rotate(${a} ${cx} ${cy})" d="M${cx} ${cy} C${cx} ${cy - r * .9} ${cx + r * .55} ${cy - r * .95} ${cx + r * .6} ${cy - r * .55} C${cx + r * .5} ${cy - r * .3} ${cx + r * .2} ${cy - r * .1} ${cx} ${cy}Z" fill="#3c4756"/>`).join("")}</g><circle cx="${cx}" cy="${cy}" r="${r * .22}" fill="#ffb020"/>`;
function coolArt(kind) {
  if (kind === "water") return `<svg viewBox="0 0 210 150">
    <rect x="6" y="8" width="198" height="52" rx="4" fill="#161c24" stroke="#3a4452"/>
    ${[...Array(30)].map((_, i) => `<line x1="${14 + i * 6.2}" y1="12" x2="${14 + i * 6.2}" y2="56" stroke="#262e39"/>`).join("")}
    <circle cx="56" cy="34" r="22" fill="#10151b" stroke="#2c3540"/>${FAN(56, 34, 20, "spin fan")}
    <circle cx="154" cy="34" r="22" fill="#10151b" stroke="#2c3540"/>${FAN(154, 34, 20, "spin fan")}
    <path class="flow" d="M80 60 C80 90 90 100 100 112" fill="none" stroke="#44c7e0" stroke-width="5" stroke-linecap="round"/>
    <path class="flow" d="M130 60 C130 90 120 100 112 112" fill="none" stroke="#ff8a3d" stroke-width="5" stroke-linecap="round"/>
    <rect x="80" y="106" width="52" height="38" rx="8" fill="#1b2129" stroke="#5a6472"/>
    <circle cx="106" cy="125" r="11" fill="none" stroke="#44c7e0" stroke-width="2" stroke-dasharray="10 4" class="spin pump" style="transform-origin:106px 125px"/><circle cx="106" cy="125" r="4" fill="#44c7e0"/>
  </svg>`;
  if (kind === "air") return `<svg viewBox="0 0 210 150">
    <rect x="40" y="128" width="130" height="14" rx="2" fill="#1a2e22" stroke="#3a4452"/>
    <rect x="88" y="118" width="34" height="12" rx="2" fill="#5a6472"/>
    ${[...Array(4)].map((_, i) => `<path d="M${94 + i * 7} 120 L${94 + i * 7} 14" stroke="#b07a3a" stroke-width="3"/>`).join("")}
    ${[...Array(22)].map((_, i) => `<rect x="70" y="${12 + i * 4.8}" width="96" height="2.4" fill="#4a5462"/>`).join("")}
    <rect x="16" y="20" width="56" height="96" rx="4" fill="#10151b" stroke="#3a4452"/>
    ${FAN(44, 68, 25, "spin fan")}
  </svg>`;
  return `<svg viewBox="0 0 210 150"><rect x="55" y="25" width="100" height="100" rx="8" fill="none" stroke="#3a4452" stroke-dasharray="6 5"/><text x="105" y="90" text-anchor="middle" font-size="48" fill="#556070" font-family="Bahnschrift">?</text></svg>`;
}
let lastCoolKind = "";
function renderCooling(c, cpu) {
  if (!c || !c.kind) return;
  if (c.kind !== lastCoolKind) { $("cool-art").innerHTML = coolArt(c.kind); lastCoolKind = c.kind; }
  const label = { water: "Water cooler", air: "Air cooler", unknown: "Não identificado" }[c.kind];
  $("cool-type").textContent = label;
  $("cool-type").style.color = c.kind === "water" ? css("--cyan") : c.kind === "air" ? css("--text") : css("--muted");
  $("cool-reason").textContent = c.reason;
  $("cool-conf").textContent = { manual: "definido manualmente", high: "certeza alta", medium: "provável", low: "palpite" }[c.confidence] || "";
  const art = $("cool-art");
  const spd = (rpm) => (rpm ? Math.max(0.15, 60 / rpm * 8).toFixed(2) + "s" : "0s");
  art.querySelectorAll(".fan").forEach((f) => f.style.setProperty("--spd", spd(c.cpu_fan_rpm ?? (c.kind !== "unknown" ? 900 : 0))));
  art.querySelectorAll(".pump").forEach((f) => f.style.setProperty("--spd", spd(c.pump_rpm ?? 1500)));
  art.querySelectorAll(".flow").forEach((f) => f.style.setProperty("--fspd", c.pump_rpm === 0 ? "0s" : "1.2s"));
  const st = [];
  st.push(`<div><b style="color:${tcol(cpu.package_temp)}">${cpu.package_temp != null ? f0(cpu.package_temp) + " °C" : "—"}</b><span>CPU agora</span></div>`);
  if (c.kind === "water") st.push(`<div><b>${c.pump_rpm != null ? f0(c.pump_rpm) : "—"}</b><span>bomba (rpm)</span></div>`);
  st.push(`<div><b>${c.cpu_fan_rpm != null ? f0(c.cpu_fan_rpm) : "—"}</b><span>${c.kind === "water" ? "fans do radiador" : "ventoinha da CPU"} (rpm)</span></div>`);
  if (c.kind !== "water") st.push(`<div><b>${f0(cpu.power)}${cpu.power != null ? " W" : ""}</b><span>calor da CPU</span></div>`);
  $("cool-stats").innerHTML = st.join("");
  $("cool-tip").innerHTML = c.kind === "water"
    ? "Water cooler: a bomba precisa estar sempre girando. Se a rotação cair a zero a CPU esquenta em segundos. Verifique o nível de ruído da bomba e limpe o radiador a cada 6 meses."
    : c.kind === "air"
      ? "Air cooler: limpe a poeira das aletas a cada 3–6 meses. CPU acima de 85 °C em carga costuma indicar pasta térmica velha ou ventoinha fraca."
      : "Ative os sensores completos (aviso no topo) para o app ler as ventoinhas e a bomba, ou escolha o tipo em Configurações.";
}

function rows(el, items, empty) { $(el).innerHTML = items.length ? items.join("") : `<div class="empty">${empty}</div>`; }

let rateMs = 1000, lastT = null;
function render(t) {
  lastT = t;
  t.hw = t.hw || { board: {}, fans: [], spd: [], drives: [] };
  const { cpu, gpu, mem, net } = t;
  const memPct = mem.total ? (mem.used / mem.total) * 100 : null;
  push("cpu", cpu.load); push("gpu", gpu.load); push("mem", memPct); push("net", net.rx_bps);
  push("cput", cpu.package_temp); push("gput", gpu.temp);

  $("host").textContent = t.host || "PC";
  $("uptime").textContent = uptime(t.uptime);
  const src = (n, ok, msg) => `<b>${n}</b> <span class="${ok ? "ok" : "off"}">${ok ? "●" : "○"}</span> ${msg}`;
  const sens = { running: "CPU · placa-mãe · ventoinhas · RAM · GPU", limited: "limitado (sem administrador)", starting: "iniciando…", installing: "instalando PawnIO…" }[t.lhm_state] || "desligado";
  $("status").innerHTML = [src("Windows", true, "carga · clocks · RAM · discos · rede"), src("NVML", !gpu.error, gpu.error ? "sem GPU NVIDIA" : "GPU"),
    src("Sensores", t.lhm_state === "running", sens), ...(t.hwinfo ? [] : [src("HWiNFO", true, t.source === "hwinfo" ? "em uso" : "disponível")])].join("  ·  ");
  renderSensors(t);

  // alerts
  const keys = new Set(t.alerts.map((a) => a.key));
  $("alertbar").hidden = !t.alerts.length;
  $("alertbar").innerHTML = t.alerts.map((a) => `<span class="ai"><b>${esc(a.title)}</b> ${f0(a.value)} ${a.unit} <span style="color:var(--muted)">(limite ${f0(a.limit)})</span></span>`).join("");
  $("v-cpu").classList.toggle("alert", keys.has("cpu_temp"));
  $("v-gpu").classList.toggle("alert", keys.has("gpu_temp"));
  $("v-ram").classList.toggle("alert", keys.has("ram"));
  $("p-cpu").classList.toggle("alert", keys.has("cpu_temp"));
  $("p-gpu").classList.toggle("alert", keys.has("gpu_temp"));
  $("p-therm").classList.toggle("alert", keys.has("cpu_temp") || keys.has("gpu_temp"));

  // vitals
  $("cpu-load").textContent = f0(cpu.load);
  $("cpu-sub").textContent = `${(cpu.mhz_avg / 1000).toFixed(2)} GHz · ${cpu.phys?.length || cpu.cores.length} núcleos` + (cpu.package_temp != null ? ` · ${f0(cpu.package_temp)} °C` : "");
  $("gpu-load").textContent = f0(gpu.load);
  $("gpu-sub").textContent = `${gpu.temp ?? "—"} °C · ${f0(gpu.power_w)} W · ${gpu.clock ?? "—"} MHz`;
  $("mem-pct").textContent = f0(memPct);
  $("mem-sub").textContent = `${gb(mem.used)} de ${gb(mem.total)} GB`;
  const [rx, rxu] = rate(net.rx_bps), [tx, txu] = rate(net.tx_bps);
  $("net-rx").textContent = rx; $("net-rx-u").textContent = rxu;
  $("net-sub").textContent = `↑ ${tx} ${txu}`;
  bars("sp-cpu", hist.cpu, css("--indigo"), 100);
  bars("sp-gpu", hist.gpu, css("--green"), 100);
  bars("sp-mem", hist.mem, css("--amber"), 100);
  bars("sp-net", hist.net, css("--cyan"));

  // sprites
  renderCpu(t);
  renderGpu(t);
  renderRam(t);
  renderBoard(t);
  renderFans(t);
  renderCooling(t.cooling, cpu);

  // thermals
  $("cpu-temp").innerHTML = cpu.package_temp != null ? `<span style="color:${tcol(cpu.package_temp)}">${f1(cpu.package_temp)}°</span>` : "—";
  $("gpu-temp").innerHTML = gpu.temp != null ? `<span style="color:${tcol(gpu.temp)}">${gpu.temp}°</span>` : "—";
  $("cpu-pwr").textContent = cpu.power != null ? f0(cpu.power) + " W" : "—";
  $("gpu-pwr").textContent = gpu.power_w != null ? f0(gpu.power_w) + " W" : "—";
  const mx = (a) => { const v = a.filter((x) => x != null); return v.length ? Math.max(...v) : null; };
  $("therm-sub").textContent = `máx. CPU ${f0(mx(hist.cput))}° · GPU ${f0(mx(hist.gput))}° · últimos ${Math.round(HIST * rateMs / 60000)} min`;
  lines("sp-therm", [{ data: hist.cput, color: css("--indigo") }, { data: hist.gput, color: css("--green") }]);

  // storage
  const dTot = t.disks.reduce((a, d) => a + d.total, 0), dUsed = t.disks.reduce((a, d) => a + d.used, 0);
  $("disk-tag").textContent = `${gb(dUsed)} / ${gb(dTot)} GB`;
  rows("disks", t.disks.map((d) => {
    const p = d.total ? d.used / d.total * 100 : 0;
    const col = keys.has("disk:" + d.mount) || p > 90 ? css("--red") : p > 75 ? css("--orange") : css("--amber");
    return `<div class="row"><div class="stack"><div style="display:flex;gap:8px"><span class="k"><b>${esc(d.mount)}</b> ${esc(d.name || d.kind)} <small style="color:var(--dim)">${esc(d.kind)}</small></span><span class="v">${gb(d.used)} / ${gb(d.total)} GB · ${f0(p)}%</span></div><div class="seg10" style="--c:${col}">${segbar(p, 20)}</div></div></div>`;
  }), "nenhum disco");
  const drv = t.hw.drives || [];
  $("drv-lbl").hidden = !drv.length;
  $("drives").innerHTML = drv.map((d) => `<div class="row"><span class="k">${esc(d.name)}</span><span class="v">${[d.temp != null ? f0(d.temp) + " °C" : "", d.life != null ? `vida ${f0(d.life)}%` : "", d.hours != null ? `${f0(d.hours)} h ligado` : ""].filter(Boolean).join(" · ")}</span></div>`).join("");

  // processes + fans
  rows("procs", t.procs.map((p) => `<div class="row"><span class="k">${esc(p.name)}</span><span class="v">${f1(p.cpu)} % · ${(p.mem / 1048576).toFixed(0)} MB</span></div>`), "—");
}

// ---------- controls ----------
document.querySelectorAll("#rate button").forEach((b) => b.addEventListener("click", () => setRate(+b.dataset.ms, true)));
function setRate(ms, send) {
  rateMs = ms;
  document.querySelectorAll("#rate button").forEach((x) => x.classList.toggle("on", +x.dataset.ms === ms));
  if (send) invoke("set_interval", { ms });
}

let cfg = null;
const F = { alerts_enabled: "c-alerts", notify: "c-notify", close_to_tray: "c-tray" };
const N = { cpu_temp: "c-cpu", gpu_temp: "c-gpu", ram_pct: "c-ram", disk_pct: "c-disk", hold_s: "c-hold", cooldown_min: "c-cool" };
function fillSettings() {
  if (!cfg) return;
  for (const [k, id] of Object.entries(F)) $(id).checked = !!cfg[k];
  for (const [k, id] of Object.entries(N)) $(id).value = cfg[k];
  $("c-cooling").value = cfg.cooling || "auto";
  $("c-admin").checked = cfg.sensors_admin !== false;
}
$("open-settings").onclick = () => { fillSettings(); $("save-msg").textContent = ""; $("settings").hidden = false; };
$("close-settings").onclick = () => ($("settings").hidden = true);
$("settings").onclick = (e) => { if (e.target.id === "settings") $("settings").hidden = true; };
document.addEventListener("keydown", (e) => { if (e.key === "Escape") $("settings").hidden = true; });
$("test-alert").onclick = () => invoke("test_alert").then(() => ($("save-msg").textContent = "notificação enviada"), (e) => ($("save-msg").textContent = String(e)));
$("save-settings").onclick = async () => {
  const next = { ...(cfg || {}) };
  for (const [k, id] of Object.entries(F)) next[k] = $(id).checked;
  for (const [k, id] of Object.entries(N)) next[k] = Math.max(0, Number($(id).value) || 0);
  next.cooling = $("c-cooling").value;
  next.sensors_admin = $("c-admin").checked;
  next.cpu_view = cpuView;
  next.interval_ms = rateMs;
  try { cfg = (await invoke("save_config", { cfg: next })) || next; $("save-msg").textContent = "salvo ✓"; setTimeout(() => ($("settings").hidden = true), 600); }
  catch (e) { $("save-msg").style.color = "var(--red)"; $("save-msg").textContent = String(e); }
};

if (TAURI) {
  invoke("get_config").then((c) => { cfg = c; cpuView = c.cpu_view || "cores"; setRate(c.interval_ms, false); });
  invoke("get_pcinfo").then((p) => p && renderPc(p));
  TAURI.event.listen("pcinfo", (e) => renderPc(e.payload));
  TAURI.event.listen("telemetry", (e) => render(e.payload));
} else {
  // Browser preview with fake data.
  cfg = { alerts_enabled: true, notify: true, cpu_temp: 85, gpu_temp: 83, ram_pct: 90, disk_pct: 95, hold_s: 5, cooldown_min: 5, close_to_tray: true, interval_ms: 1000, cooling: "auto" };
  ramLayout = { mode: "Dual channel", slots: [{ name: "A0", module: { size_gb: 16 } }, { name: "A1", module: null }, { name: "B0", module: { size_gb: 16 } }, { name: "B1", module: null }] };
  renderPc({ maker: "DEMO", model: "Desktop", board_maker: "DEMO", board: "B550M", bios: "American Megatrends Inc. 5.13", os: "Microsoft Windows 11 Pro", os_build: "10.0.26300 build 26300",
    cpu: "Intel(R) Core(TM) i7-9700 CPU @ 3.00GHz", cpu_cores: 8, cpu_threads: 8, cpu_max_mhz: 3000, cpu_l3_kb: 12288, cpu_socket: "U3E1", gpus: ["NVIDIA GeForce GTX 1660 SUPER"], gpu_driver: "32.0.16.2114",
    ram: [{ slot: "ChannelA-DIMM0", size_gb: 16, speed: 3600, configured: 3200, mv: 1350, kind: 26, maker: "04CD", part: "F4-3600C18-16GVK" }, { slot: "ChannelB-DIMM0", size_gb: 16, speed: 3600, configured: 3200, mv: 1350, kind: 26, maker: "04CD", part: "F4-3600C18-16GVK" }],
    ram_slots: 4, ram_max_gb: 64, rgb_devices: ["AURA LED Controller"], board_version: "Rev 1.xx",
    disks: [{ model: "ST2000LM007-1R8174", size_gb: 1863, media: "HDD", bus: "SATA" }, { model: "SSD 240GB", size_gb: 224, media: "SSD", bus: "SATA" }, { model: "ATA TOSHIBA MQ01ABD0", size_gb: 466, media: "Unspecified", bus: "USB" }],
    cooling_devices: [] });
  let k = 0;
  setInterval(() => {
    k++;
    const w = (a, p) => a * (0.5 + 0.5 * Math.sin(k / p));
    const water = Math.floor(k / 15) % 2 === 1, ct = 40 + w(50, 9);
    render({
      host: "DEMO-PC", uptime: 12345 + k, hwinfo: null,
      alerts: ct > 85 ? [{ key: "cpu_temp", title: "CPU quente", value: ct, limit: 85, unit: "°C" }] : [],
      cooling: water ? { kind: "water", confidence: "high", reason: "bomba detectada em “W_PUMP+” (2100 rpm)", pump_rpm: 2100, cpu_fan_rpm: 1100 }
        : { kind: "air", confidence: "medium", reason: "ventoinha da CPU em “CPU” (1100 rpm) e nenhuma bomba", pump_rpm: null, cpu_fan_rpm: 1100 },
      lhm_state: "running", lhm_msg: "ativo", pawnio: true, source: "lhm",
      cpu: { name: "Intel(R) Core(TM) i7-9700 CPU @ 3.00GHz", load: 10 + w(60, 7), mhz_avg: 4300, package_temp: ct, core_max: ct + 4, power: 20 + w(60, 9), cores_power: 15 + w(50, 9), vcore: 1.25, vid: 1.28, bus_mhz: 100,
        mhz_min: 4200, mhz_max: 4700, session_min: 800, session_max: 4700, threads: 8, mem_power: 1.4,
        phys: [...Array(8)].map((_, i) => ({ idx: i, kind: "P", load: 5 + w(90, 3 + i), mhz: 4500 - i * 30, temp: 38 + w(40, 4 + i), volt: 1.2 + i * 0.01 })),
        cores: [...Array(8)].map((_, i) => ({ idx: i, kind: "", load: 5 + w(90, 3 + i), mhz: 4500 - i * 30, temp: null, volt: null })) },
      gpu: { name: "NVIDIA GeForce GTX 1660 SUPER", temp: Math.round(45 + w(30, 11)), load: Math.round(w(100, 6)), mem_load: Math.round(w(60, 6)), vram_used: 2.5e9 + w(3e9, 13), vram_total: 6.4e9,
        power_w: 15 + w(110, 6), power_limit_w: 125, clock: 1530 + Math.round(w(400, 6)), mem_clock: 7000, fan: Math.round(30 + w(50, 11)), fan_rpm: 1400,
        hotspot: 55 + w(30, 11), mem_temp: null, vrm_temp: null, voltage: 0.9, bus_width: 192 },
      mem: { used: 8.6e9 + w(2e9, 10), total: 17.1e9, swap_used: 1e9, swap_total: 4e9 },
      disks: [{ name: "SSD 240GB", mount: "C:", kind: "SSD", used: 2.1e11, total: 2.4e11 }, { name: "ST2000LM007", mount: "D:", kind: "HDD", used: 1.2e12, total: 2e12 }],
      net: { rx_bps: w(5e6, 4), tx_bps: w(4e5, 5) },
      procs: [{ name: "VRChat", cpu: 22.4, mem: 3.2e9 }, { name: "chrome", cpu: 6.1, mem: 1.1e9 }, { name: "Discord", cpu: 1.2, mem: 4e8 }],
      fans: [{ label: "CPU", rpm: 1100 }, { label: "Chassis1", rpm: 870 }],
      hw: { board: { chip: "Nuvoton NCT6798D", headers: 6, vrm: 48 + w(10, 8), vrm_mem: null, pch: 44, dram_v: 1.35, power: null, temps: [{ label: "System", value: 32 }], volts: [{ label: "Vcore", value: 1.25 }, { label: "+12V", value: 12.1 }] },
        fans: [{ label: "CPU Fan", rpm: 1100, pct: 55 }, { label: "Pump", rpm: 0, pct: 100 }, { label: "System Fan #1", rpm: 870, pct: 40 }, { label: "System Fan #2", rpm: 0, pct: 40 }, { label: "System Fan #3", rpm: 0, pct: 40 }, { label: "AIO Pump", rpm: 0, pct: 100 }],
        spd: [{ taa: 11.25, trcd: 11.25, trp: 11.25, tras: 26.25, trfc: 350, temp: null }], drives: [{ name: "Samsung SSD 970", temp: 41, life: 98, hours: 1200 }] },
    });
  }, 1000);
}
