package com.akaineko.markii

import android.app.Activity
import android.app.AlertDialog
import android.content.res.ColorStateList
import android.content.Context
import android.graphics.Typeface
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.text.InputType
import android.view.Gravity
import android.view.ViewGroup.LayoutParams.MATCH_PARENT
import android.view.ViewGroup.LayoutParams.WRAP_CONTENT
import android.widget.Button
import android.widget.EditText
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import org.json.JSONObject
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean

/**
 * Pair once with the address and PIN shown in Mark II (Configurações > Celular e óculos),
 * then show the live panel. Read-only: the app never sends commands to the PC.
 */
class MainActivity : Activity() {
    private val prefs by lazy { getSharedPreferences("markii", Context.MODE_PRIVATE) }
    private val io = Executors.newSingleThreadExecutor()
    private val ui = Handler(Looper.getMainLooper())
    private val polling = AtomicBoolean(false)
    private var panel: PanelView? = null
    private var pal: Palette = Themes.fixed[0]

    private val address get() = prefs.getString("address", "") ?: ""
    private val token get() = prefs.getString("token", "") ?: ""
    private val shareDevice get() = prefs.getBoolean("share_device", true)

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        pal = Themes.current(this)
        paintWindow()
        if (token.isEmpty()) showConnect(null) else showPanel()
    }

    override fun onResume() {
        super.onResume()
        // Material You follows the wallpaper: pick up a change made while we were away
        val fresh = Themes.current(this)
        if (fresh.bg != pal.bg || fresh.amber != pal.amber) applyTheme(fresh)
        if (panel != null) startPolling()
    }

    override fun onPause() {
        super.onPause()
        polling.set(false)
    }

    private fun dp(v: Int) = (v * resources.displayMetrics.density).toInt()

    @Suppress("DEPRECATION")
    private fun paintWindow() {
        window.statusBarColor = pal.bg
        window.navigationBarColor = pal.bg
        window.decorView.setBackgroundColor(pal.bg)
    }

    private fun applyTheme(p: Palette) {
        pal = p
        paintWindow()
        val pv = panel
        if (pv != null) showPanel(pv.snap) else showConnect(null)
    }

    /** Accent button with readable text on any theme. */
    private fun accentButton(t: String, primary: Boolean) = Button(this).apply {
        text = t
        isAllCaps = false
        backgroundTintList = ColorStateList.valueOf(if (primary) pal.amber else pal.panel2)
        setTextColor(if (primary) (if (pal.light) 0xFFFFFFFF.toInt() else pal.bg) else pal.text)
    }

    private fun pickTheme() {
        val choices = Themes.choices()
        val current = Themes.selectedKey(this)
        val idx = choices.indexOfFirst { it.first == current }.coerceAtLeast(0)
        AlertDialog.Builder(this, if (pal.light) android.R.style.Theme_Material_Light_Dialog_Alert else android.R.style.Theme_Material_Dialog_Alert)
            .setTitle("Tema")
            .setSingleChoiceItems(choices.map { it.second }.toTypedArray(), idx) { d, which ->
                Themes.select(this, choices[which].first)
                d.dismiss()
                applyTheme(Themes.current(this))
            }
            .setNegativeButton("Fechar", null)
            .show()
    }

    private fun showConnect(message: String?) {
        polling.set(false)
        panel = null
        val col = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            gravity = Gravity.CENTER_HORIZONTAL
            setPadding(dp(32), dp(28), dp(32), dp(28))
            setBackgroundColor(pal.bg)
        }
        fun label(t: String, size: Float, color: Int, bold: Boolean = false) = TextView(this).apply {
            text = t
            textSize = size
            setTextColor(color)
            if (bold) typeface = Typeface.DEFAULT_BOLD
        }
        col.addView(label("MARK II", 28f, pal.amber, true))
        col.addView(label("No PC, abra o Mark II > Configurações > Celular e óculos, ligue a opção e use o endereço e o PIN mostrados.", 15f, pal.muted).apply {
            setPadding(0, dp(8), 0, dp(16))
            gravity = Gravity.CENTER
        })
        val addr = EditText(this).apply {
            hint = "endereço (ex.: 192.168.0.10:47820)"
            setText(address)
            inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_URI
            setTextColor(pal.text)
            setHintTextColor(pal.muted)
            isSingleLine = true
        }
        val pin = EditText(this).apply {
            hint = "PIN (6 dígitos)"
            inputType = InputType.TYPE_CLASS_NUMBER
            setTextColor(pal.text)
            setHintTextColor(pal.muted)
            isSingleLine = true
        }
        val status = label(message ?: "", 15f, if (message == null) pal.muted else pal.red).apply {
            setPadding(0, dp(12), 0, 0)
            gravity = Gravity.CENTER
        }
        for (e in listOf(addr, pin)) e.backgroundTintList = ColorStateList.valueOf(pal.amber)
        val go = accentButton("Conectar", true).apply {
            setOnClickListener {
                val a = addr.text.toString().trim()
                val p = pin.text.toString().trim()
                if (a.isEmpty() || p.length != 6) {
                    status.setTextColor(pal.red)
                    status.text = "Digite o endereço e os 6 dígitos do PIN."
                    return@setOnClickListener
                }
                isEnabled = false
                status.setTextColor(pal.muted)
                status.text = "conectando…"
                io.execute {
                    val r = runCatching { Api.pair(a, p, DeviceInfo.name()) }
                    ui.post {
                        isEnabled = true
                        r.onSuccess { t ->
                            prefs.edit().putString("address", a).putString("token", t).apply()
                            showPanel()
                        }.onFailure { e ->
                            status.setTextColor(pal.red)
                            status.text = when (e) {
                                is Api.HttpError -> e.message
                                else -> "Não achei o Mark II nesse endereço. Ele está aberto, com a opção ligada, na mesma rede?"
                            }
                        }
                    }
                }
            }
        }
        col.addView(addr, LinearLayout.LayoutParams(dp(420), WRAP_CONTENT))
        col.addView(pin, LinearLayout.LayoutParams(dp(420), WRAP_CONTENT))
        col.addView(go, LinearLayout.LayoutParams(dp(200), WRAP_CONTENT).apply { topMargin = dp(12) })
        col.addView(status)
        col.addView(android.widget.CheckBox(this).apply {
            text = "Mostrar a bateria e o status deste aparelho no Mark II do PC"
            setTextColor(pal.muted)
            buttonTintList = ColorStateList.valueOf(pal.amber)
            isChecked = shareDevice
            setOnCheckedChangeListener { _, v -> prefs.edit().putBoolean("share_device", v).apply() }
        }, LinearLayout.LayoutParams(WRAP_CONTENT, WRAP_CONTENT).apply { topMargin = dp(16) })
        col.addView(accentButton("Tema", false).apply { setOnClickListener { pickTheme() } },
            LinearLayout.LayoutParams(WRAP_CONTENT, WRAP_CONTENT).apply { topMargin = dp(20) })
        setContentView(col)
    }


    // ---------------------------------------------------------------- tabs
    private var tab = 0
    private var last: Snap? = null
    private val hist = ArrayDeque<Double>()
    private var content: FrameLayout? = null
    private var listBox: LinearLayout? = null
    private var navItems = listOf<TextView>()
    private var deviceLine = ""
    private var liveStatus = "conectando…"
    private var lastDevice: JSONObject? = null

    private fun showPanel(keep: Snap? = null) {
        if (keep != null) last = keep
        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setBackgroundColor(pal.bg)
        }
        val body = FrameLayout(this)
        root.addView(body, LinearLayout.LayoutParams(MATCH_PARENT, 0, 1f))
        val nav = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            setBackgroundColor(pal.panel)
            setPadding(0, dp(6), 0, dp(8))
        }
        val names = listOf("◉" to "Painel", "▤" to "Detalhes", "▯" to "Este aparelho", "⚙" to "Ajustes")
        navItems = names.mapIndexed { i, (icon, name) ->
            TextView(this).apply {
                text = "$icon\n$name"
                gravity = Gravity.CENTER
                textSize = 12f
                setPadding(0, dp(4), 0, dp(4))
                setOnClickListener { selectTab(i) }
            }.also { nav.addView(it, LinearLayout.LayoutParams(0, WRAP_CONTENT, 1f)) }
        }
        root.addView(nav, LinearLayout.LayoutParams(MATCH_PARENT, WRAP_CONTENT))
        content = body
        panel = PanelView(this)
        setContentView(root)
        selectTab(tab)
        startPolling()
    }

    private fun selectTab(i: Int) {
        tab = i
        navItems.forEachIndexed { j, t -> t.setTextColor(if (j == i) pal.amber else pal.muted) }
        val body = content ?: return
        body.removeAllViews()
        listBox = null
        if (i == 0) {
            val pv = panel ?: PanelView(this).also { panel = it }
            pv.pal = pal
            pv.cpuHist = hist.toList()
            pv.device = deviceLine
            pv.status = liveStatus
            pv.snap = last
            body.addView(pv, FrameLayout.LayoutParams(MATCH_PARENT, MATCH_PARENT))
        } else {
            val sv = ScrollView(this)
            val box = LinearLayout(this).apply {
                orientation = LinearLayout.VERTICAL
                setPadding(dp(18), dp(14), dp(18), dp(18))
            }
            sv.addView(box)
            body.addView(sv, FrameLayout.LayoutParams(MATCH_PARENT, MATCH_PARENT))
            listBox = box
            fillList()
        }
    }

    // ---- list building blocks
    private fun section(box: LinearLayout, title: String) {
        box.addView(TextView(this).apply {
            text = title.uppercase()
            textSize = 12f
            letterSpacing = 0.08f
            setTextColor(pal.amber)
            typeface = Typeface.DEFAULT_BOLD
            setPadding(0, dp(18), 0, dp(6))
        })
    }

    private fun kv(box: LinearLayout, k: String, v: String, color: Int = pal.text) {
        val row = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            setPadding(0, dp(5), 0, dp(5))
        }
        row.addView(TextView(this).apply { text = k; textSize = 15f; setTextColor(pal.muted) }, LinearLayout.LayoutParams(0, WRAP_CONTENT, 1f))
        row.addView(TextView(this).apply { text = v; textSize = 15f; setTextColor(color); gravity = Gravity.END })
        box.addView(row)
    }

    private fun barRow(box: LinearLayout, k: String, pct: Double, color: Int, right: String) {
        kv(box, k, right)
        box.addView(BarView(this, pct, color, pal.line), LinearLayout.LayoutParams(MATCH_PARENT, dp(8)).apply { bottomMargin = dp(4) })
    }

    private fun note(box: LinearLayout, t: String, color: Int = pal.muted) {
        box.addView(TextView(this).apply { text = t; textSize = 14f; setTextColor(color); setPadding(0, dp(6), 0, dp(6)) })
    }

    private fun fillList() {
        val box = listBox ?: return
        box.removeAllViews()
        when (tab) {
            1 -> details(box)
            2 -> thisDevice(box)
            3 -> settings(box)
        }
    }

    private fun details(box: LinearLayout) {
        val s = last
        if (s == null) {
            note(box, liveStatus)
            return
        }
        fun o(v: Double?, unit: String) = PanelView.opt(v, unit)
        section(box, "Processador")
        note(box, s.cpuName.trim(), pal.text)
        barRow(box, "Uso", s.cpuLoad, pal.indigo, "%.0f%%".format(s.cpuLoad))
        kv(box, "Temperatura", o(s.cpuTemp, " °C"), pal.tempCol(s.cpuTemp))
        kv(box, "Consumo", o(s.cpuPower, " W"))
        kv(box, "Clock médio", "%.2f GHz".format(s.cpuMhz / 1000.0))
        if (s.cores.isNotEmpty()) {
            section(box, "Threads (${s.cores.size})")
            s.cores.chunked(2).forEachIndexed { r, pair ->
                val row = LinearLayout(this).apply { orientation = LinearLayout.HORIZONTAL }
                pair.forEachIndexed { k, load ->
                    val cell = LinearLayout(this).apply {
                        orientation = LinearLayout.VERTICAL
                        setPadding(if (k == 0) 0 else dp(8), 0, if (k == 0) dp(8) else 0, 0)
                    }
                    barRow(cell, "T${r * 2 + k}", load, pal.indigo, "%.0f%%".format(load))
                    row.addView(cell, LinearLayout.LayoutParams(0, WRAP_CONTENT, 1f))
                }
                if (pair.size == 1) row.addView(android.view.View(this), LinearLayout.LayoutParams(0, 0, 1f))
                box.addView(row)
            }
        }
        section(box, "Placa de vídeo")
        note(box, s.gpuName.trim().ifBlank { "GPU" }, pal.text)
        barRow(box, "Uso", s.gpuLoad ?: 0.0, pal.green, o(s.gpuLoad, "%"))
        kv(box, "Temperatura", o(s.gpuTemp, " °C"), pal.tempCol(s.gpuTemp))
        if (s.gpuHotspot != null) kv(box, "Hotspot", o(s.gpuHotspot, " °C"), pal.tempCol(s.gpuHotspot))
        kv(box, "Consumo", o(s.gpuPower, " W"))
        s.gpuClock?.let { kv(box, "Clock", "$it MHz") }
        s.gpuFan?.let { kv(box, "Ventoinha", "$it%") }
        if (s.vramUsed != null && s.vramTotal != null && s.vramTotal > 0) {
            barRow(box, "VRAM", s.vramUsed * 100.0 / s.vramTotal, pal.green, "${PanelView.gb(s.vramUsed)} / ${PanelView.gb(s.vramTotal)} GB")
        }
        section(box, "Memória")
        barRow(box, "RAM", s.ramPct, pal.amber, "${PanelView.gb(s.ramUsed)} / ${PanelView.gb(s.ramTotal)} GB")
        if (s.disks.isNotEmpty()) {
            section(box, "Discos")
            for ((name, used, total) in s.disks) {
                val pct = if (total > 0) used * 100.0 / total else 0.0
                barRow(box, name, pct, if (pct >= 90) pal.red else pal.cyan, "${PanelView.gb(used)} / ${PanelView.gb(total)} GB")
            }
        }
        if (s.fans.isNotEmpty()) {
            section(box, "Ventoinhas")
            for ((name, rpm) in s.fans) kv(box, name, "%.0f rpm".format(rpm))
        }
        section(box, "Rede e sistema")
        kv(box, "Download", PanelView.rate(s.netRx), pal.cyan)
        kv(box, "Upload", PanelView.rate(s.netTx), pal.cyan)
        kv(box, "Ligado há", PanelView.uptime(s.uptime))
        if (s.alerts.isNotEmpty()) {
            section(box, "Alertas")
            for (a in s.alerts) note(box, "⚠ $a", pal.red)
        }
    }

    private fun thisDevice(box: LinearLayout) {
        val d = lastDevice ?: runCatching { DeviceInfo.collect(this) }.getOrNull() ?: JSONObject()
        section(box, d.optString("name", DeviceInfo.name()))
        if (d.has("battery")) {
            val b = d.optDouble("battery")
            val col = if (b <= 15) pal.red else if (b <= 30) pal.orange else pal.green
            barRow(box, "Bateria", b, col, "%.0f%%".format(b) + if (d.optBoolean("charging")) " · carregando" else "")
        }
        if (d.has("battery_temp")) kv(box, "Temp. da bateria", "%.0f °C".format(d.optDouble("battery_temp")))
        if (d.has("thermal")) {
            val (txt, col) = when (d.optInt("thermal")) {
                0 -> "normal" to pal.green
                1 -> "morno" to pal.green
                2 -> "quente" to pal.yellow
                3 -> "muito quente" to pal.orange
                else -> "crítico" to pal.red
            }
            kv(box, "Aquecimento", txt, col)
        }
        if (d.has("cpu_temp")) kv(box, "Processador", "%.0f °C".format(d.optDouble("cpu_temp")))
        if (d.has("ram_total")) {
            val u = d.optLong("ram_used")
            val t = d.optLong("ram_total")
            barRow(box, "Memória", if (t > 0) u * 100.0 / t else 0.0, pal.amber, "${PanelView.gb(u)} / ${PanelView.gb(t)} GB")
        }
        if (d.has("storage_total")) {
            val f = d.optLong("storage_free")
            val t = d.optLong("storage_total")
            barRow(box, "Armazenamento", if (t > 0) (t - f) * 100.0 / t else 0.0, pal.cyan, "${PanelView.gb(f)} GB livres de ${PanelView.gb(t)}")
        }
        if (d.has("wifi_rssi")) {
            val r = d.optInt("wifi_rssi")
            val mbps = if (d.has("wifi_mbps")) " · ${d.optInt("wifi_mbps")} Mbit/s" else ""
            kv(box, "Wi-Fi", "${DeviceInfo.wifiQuality(r)} · $r dBm$mbps", if (r >= -67) pal.green else if (r >= -75) pal.yellow else pal.red)
        }
        if (d.has("uptime")) kv(box, "Ligado há", PanelView.uptime(d.optLong("uptime")))
        note(box, if (shareDevice) "Estes números também aparecem no Mark II do PC (dá para desligar em Ajustes)." else "Não estão sendo enviados ao PC (ligue em Ajustes).")
    }

    private fun settings(box: LinearLayout) {
        section(box, "Conexão")
        kv(box, "PC", last?.host ?: "—")
        kv(box, "Endereço", address)
        kv(box, "Situação", liveStatus, if (liveStatus == "ao vivo") pal.green else pal.muted)
        section(box, "Aparência")
        val themeName = Themes.choices().firstOrNull { it.first == Themes.selectedKey(this) }?.second ?: "—"
        kv(box, "Tema", themeName)
        box.addView(accentButton("Trocar tema", false).apply { setOnClickListener { pickTheme() } }, LinearLayout.LayoutParams(WRAP_CONTENT, WRAP_CONTENT))
        section(box, "Privacidade")
        box.addView(android.widget.CheckBox(this).apply {
            text = "Mostrar a bateria e o status deste aparelho no Mark II do PC"
            setTextColor(pal.text)
            buttonTintList = ColorStateList.valueOf(pal.amber)
            isChecked = shareDevice
            setOnCheckedChangeListener { _, v -> prefs.edit().putBoolean("share_device", v).apply() }
        })
        note(box, "O app só lê os números do PC; nunca envia comandos.")
        section(box, "Sair")
        box.addView(accentButton("Desconectar deste PC", false).apply {
            setTextColor(pal.red)
            setOnClickListener {
                prefs.edit().remove("token").apply()
                last = null
                hist.clear()
                showConnect(null)
            }
        }, LinearLayout.LayoutParams(WRAP_CONTENT, WRAP_CONTENT))
        val ver = runCatching { packageManager.getPackageInfo(packageName, 0).versionName }.getOrNull() ?: "?"
        note(box, "Mark II para Android v$ver", pal.dim)
    }

    // ---------------------------------------------------------------- refresh
    private fun startPolling() {
        if (!polling.compareAndSet(false, true)) return
        io.execute {
            var fails = 0
            while (polling.get()) {
                val me = runCatching { DeviceInfo.collect(this) }.getOrNull()
                val r = runCatching { Api.snapshot(address, token, if (shareDevice) me else null) }
                val e = r.exceptionOrNull()
                if (e is Api.HttpError && e.code == 401) {
                    polling.set(false)
                    prefs.edit().remove("token").apply()
                    ui.post { showConnect("O PC removeu este aparelho. Pareie de novo com o PIN.") }
                    break
                }
                fails = if (r.isSuccess) 0 else fails + 1
                ui.post {
                    if (panel == null) return@post
                    lastDevice = me
                    deviceLine = me?.let { d ->
                        listOfNotNull(
                            d.optString("name").ifBlank { null },
                            if (d.has("battery")) "bateria %.0f%%".format(d.optDouble("battery")) else null,
                            if (d.has("wifi_rssi")) "Wi-Fi ${DeviceInfo.wifiQuality(d.optInt("wifi_rssi"))}" else null,
                        ).joinToString(" · ")
                    } ?: ""
                    r.onSuccess {
                        last = it
                        hist.addLast(it.cpuLoad)
                        while (hist.size > 30) hist.removeFirst()
                        liveStatus = "ao vivo"
                    }
                    if (r.isFailure) liveStatus = if (fails > 2) "sem conexão com o PC (o Mark II está aberto?)" else "reconectando…"
                    when (tab) {
                        0 -> panel?.let { pv ->
                            pv.cpuHist = hist.toList()
                            pv.device = deviceLine
                            pv.status = liveStatus
                            pv.snap = last
                        }
                        1, 2 -> fillList()
                    }
                }
                Thread.sleep(1000)
            }
        }
    }
}

/** Thin rounded progress bar used in the list tabs. */
class BarView(context: Context, private val pct: Double, private val color: Int, private val track: Int) : android.view.View(context) {
    private val p = android.graphics.Paint(android.graphics.Paint.ANTI_ALIAS_FLAG)
    private val r = android.graphics.RectF()

    override fun onDraw(c: android.graphics.Canvas) {
        val h = height.toFloat()
        p.color = track
        r.set(0f, 0f, width.toFloat(), h)
        c.drawRoundRect(r, h / 2, h / 2, p)
        val k = (pct.coerceIn(0.0, 100.0) / 100.0).toFloat()
        if (k > 0) {
            p.color = color
            r.set(0f, 0f, width * k, h)
            c.drawRoundRect(r, h / 2, h / 2, p)
        }
    }
}
