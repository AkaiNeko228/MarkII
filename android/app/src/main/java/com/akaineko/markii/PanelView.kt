package com.akaineko.markii

import android.content.Context
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.RectF
import android.graphics.Typeface
import android.view.View

/**
 * The "Painel" tab. Standing (phone): three rings and cards. Lying (Quest / Pico
 * window, tablet): three big columns readable from a distance. Scales to any size.
 */
class PanelView(context: Context) : View(context) {
    var snap: Snap? = null
        set(v) { field = v; invalidate() }
    var status: String = "conectando…"
        set(v) { field = v; invalidate() }
    /** One line about this phone / headset ("Quest 2 · bateria 81% · Wi-Fi bom"). */
    var device: String = ""
        set(v) { field = v; invalidate() }
    /** Last CPU loads, oldest first. */
    var cpuHist: List<Double> = emptyList()
    var pal: Palette = Themes.current(context)
        set(v) { field = v; invalidate() }

    private val p = Paint(Paint.ANTI_ALIAS_FLAG)
    private val bold = Typeface.create(Typeface.DEFAULT, Typeface.BOLD)
    private val r = RectF()

    companion object {
        fun gb(b: Long): String {
            val g = b / 1073741824.0
            return if (g >= 10) "%.0f".format(g) else "%.1f".format(g)
        }

        fun rate(bps: Double): String {
            var v = bps
            val u = arrayOf("B/s", "KB/s", "MB/s", "GB/s")
            var i = 0
            while (v >= 1000 && i < u.size - 1) { v /= 1024; i++ }
            return (if (v >= 100 || i == 0) "%.0f".format(v) else "%.1f".format(v)) + " " + u[i]
        }

        fun opt(v: Double?, unit: String) = v?.let { "%.0f%s".format(it, unit) } ?: "—"

        fun uptime(s: Long): String {
            val h = s / 3600
            return if (h >= 24) "${h / 24}d ${h % 24}h" else "${h}h ${s % 3600 / 60}m"
        }
    }

    // ---------------------------------------------------------------- drawing helpers
    private fun text(c: Canvas, s: String, x: Float, y: Float, size: Float, color: Int, b: Boolean = false, align: Paint.Align = Paint.Align.LEFT) {
        p.style = Paint.Style.FILL
        p.color = color
        p.textSize = size
        p.typeface = if (b) bold else Typeface.DEFAULT
        p.textAlign = align
        c.drawText(s, x, y, p)
        p.textAlign = Paint.Align.LEFT
    }

    /** Cuts `s` to fit `w`, with an ellipsis. */
    private fun fit(s: String, w: Float, size: Float): String {
        p.textSize = size
        if (p.measureText(s) <= w) return s
        var t = s
        while (t.isNotEmpty() && p.measureText("$t…") > w) t = t.dropLast(1)
        return "$t…"
    }

    private fun card(c: Canvas, x: Float, y: Float, w: Float, h: Float, color: Int = pal.panel) {
        p.style = Paint.Style.FILL
        p.color = color
        r.set(x, y, x + w, y + h)
        c.drawRoundRect(r, 14f, 14f, p)
    }

    private fun bar(c: Canvas, x: Float, y: Float, w: Float, h: Float, pct: Double, color: Int) {
        card(c, x, y, w, h, pal.line)
        val k = (pct.coerceIn(0.0, 100.0) / 100.0).toFloat()
        if (k > 0) {
            p.color = color
            r.set(x, y, x + w * k, y + h)
            c.drawRoundRect(r, h / 2, h / 2, p)
        }
    }

    private fun ring(c: Canvas, cx: Float, cy: Float, rad: Float, pct: Double, color: Int, label: String) {
        p.style = Paint.Style.STROKE
        p.strokeWidth = rad * 0.22f
        p.strokeCap = Paint.Cap.ROUND
        p.color = pal.line
        r.set(cx - rad, cy - rad, cx + rad, cy + rad)
        c.drawArc(r, 0f, 360f, false, p)
        p.color = color
        c.drawArc(r, -90f, (pct.coerceIn(0.0, 100.0) * 3.6).toFloat(), false, p)
        p.style = Paint.Style.FILL
        text(c, "%.0f%%".format(pct), cx, cy + rad * 0.2f, rad * 0.55f, pal.text, true, Paint.Align.CENTER)
        text(c, label, cx, cy + rad + 30f, 22f, pal.muted, align = Paint.Align.CENTER)
    }

    private fun spark(c: Canvas, x: Float, y: Float, w: Float, h: Float, data: List<Double>, color: Int) {
        val n = 30
        val bw = w / n
        val d = data.takeLast(n)
        p.style = Paint.Style.FILL
        p.color = color
        d.forEachIndexed { i, v ->
            val bh = (h * (v.coerceIn(2.0, 100.0) / 100.0)).toFloat()
            val bx = x + (n - d.size + i) * bw
            c.drawRect(bx + 1, y + h - bh, bx + bw - 1, y + h, p)
        }
    }

    private fun header(c: Canvas, s: Snap?, w: Float) {
        text(c, "MARK", 24f, 52f, 34f, pal.text, true)
        p.textSize = 34f
        p.typeface = bold
        text(c, "II", 24f + p.measureText("MARK "), 52f, 34f, pal.amber, true)
        if (s != null) text(c, fit("${s.host} · ligado há ${uptime(s.uptime)}", w - 220f, 20f), 24f, 82f, 20f, pal.muted)
        val live = status == "ao vivo"
        text(c, (if (live) "● " else "") + status, w - 24f, 50f, 20f, if (live) pal.green else pal.muted, align = Paint.Align.RIGHT)
    }

    private fun gpuShort(s: Snap) = s.gpuName.replace("NVIDIA ", "").replace("AMD ", "").replace("GeForce ", "").ifBlank { "GPU" }

    private fun cpuShort(s: Snap) = s.cpuName.replace("(R)", "").replace("(TM)", "").replace("CPU", "")
        .substringBefore("@").replace(Regex("\\s+"), " ").replace(" Processor", "").trim().ifBlank { "CPU" }

    override fun onDraw(c: Canvas) {
        c.drawColor(pal.bg)
        val portrait = height > width * 1.1f
        // design width: 600 standing, 1000 lying; height follows the window
        val dw = if (portrait) 600f else 1000f
        val k = width / dw
        c.save()
        c.scale(k, k)
        val dh = height / k
        val s = snap
        header(c, s, dw)
        if (s == null) {
            text(c, status, 24f, 160f, 26f, pal.muted)
        } else if (portrait) {
            drawPortrait(c, s, dw, dh)
        } else {
            drawLying(c, s, dw, dh)
        }
        c.restore()
    }

    private fun drawPortrait(c: Canvas, s: Snap, w: Float, h: Float) {
        val gpu = s.gpuLoad ?: 0.0
        val rad = 62f
        val y0 = 110f + rad
        ring(c, w * 0.18f, y0, rad, s.cpuLoad, pal.indigo, "CPU")
        ring(c, w * 0.5f, y0, rad, gpu, pal.green, "GPU")
        ring(c, w * 0.82f, y0, rad, s.ramPct, pal.amber, "RAM")
        var y = y0 + rad + 56f
        val cw = w - 48f
        // CPU
        card(c, 24f, y, cw, 150f)
        text(c, fit("CPU · ${cpuShort(s)}", cw - 140f, 21f), 42f, y + 36f, 21f, pal.muted)
        text(c, opt(s.cpuTemp, " °C"), 24f + cw - 18f, y + 36f, 22f, pal.tempCol(s.cpuTemp), true, Paint.Align.RIGHT)
        text(c, "${opt(s.cpuPower, " W")}  ·  %.1f GHz".format(s.cpuMhz / 1000.0), 42f, y + 70f, 21f, pal.text)
        spark(c, 42f, y + 86f, cw - 36f, 48f, cpuHist, pal.indigo)
        y += 166f
        // GPU
        card(c, 24f, y, cw, 128f)
        text(c, fit("GPU · ${gpuShort(s)}", cw - 140f, 21f), 42f, y + 36f, 21f, pal.muted)
        text(c, opt(s.gpuTemp, " °C"), 24f + cw - 18f, y + 36f, 22f, pal.tempCol(s.gpuTemp), true, Paint.Align.RIGHT)
        val vram = if (s.vramUsed != null && s.vramTotal != null && s.vramTotal > 0) "  ·  VRAM ${gb(s.vramUsed)} / ${gb(s.vramTotal)} GB" else ""
        text(c, fit("${opt(s.gpuPower, " W")}$vram", cw - 36f, 21f), 42f, y + 70f, 21f, pal.text)
        bar(c, 42f, y + 92f, cw - 36f, 12f, gpu, pal.green)
        y += 144f
        // network + fans
        card(c, 24f, y, cw, 64f)
        text(c, "↓ ${rate(s.netRx)}   ↑ ${rate(s.netTx)}", 42f, y + 41f, 20f, pal.cyan)
        s.fans.firstOrNull()?.let { text(c, "fan %.0f rpm".format(it.second), 24f + cw - 18f, y + 41f, 20f, pal.muted, align = Paint.Align.RIGHT) }
        y += 80f
        y = alertOrDevice(c, s, 24f, y, cw)
    }

    private fun drawLying(c: Canvas, s: Snap, w: Float, h: Float) {
        val gap = 16f
        val colW = (w - 48f - gap * 2) / 3
        val top = 108f
        val ch = 200f
        data class Col(val name: String, val pct: Double, val color: Int, val detail: String, val dcol: Int)
        val vram = if (s.vramUsed != null && s.vramTotal != null && s.vramTotal > 0) " · ${gb(s.vramUsed)}/${gb(s.vramTotal)} GB" else ""
        val cols = listOf(
            Col("CPU", s.cpuLoad, pal.indigo, "${opt(s.cpuTemp, " °C")} · ${opt(s.cpuPower, " W")}", pal.tempCol(s.cpuTemp)),
            Col("GPU", s.gpuLoad ?: 0.0, pal.green, "${opt(s.gpuTemp, " °C")} · ${opt(s.gpuPower, " W")}$vram", pal.tempCol(s.gpuTemp)),
            Col("RAM", s.ramPct, pal.amber, "${gb(s.ramUsed)} / ${gb(s.ramTotal)} GB", pal.muted),
        )
        cols.forEachIndexed { i, col ->
            val x = 24f + i * (colW + gap)
            card(c, x, top, colW, ch)
            text(c, col.name, x + 20f, top + 40f, 22f, pal.muted)
            text(c, "%.0f%%".format(col.pct), x + 20f, top + 104f, 58f, pal.text, true)
            bar(c, x + 20f, top + 126f, colW - 40f, 12f, col.pct, col.color)
            text(c, fit(col.detail, colW - 40f, 21f), x + 20f, top + 174f, 21f, col.dcol)
        }
        var y = top + ch + gap
        val halfW = (w - 48f - gap) / 2
        card(c, 24f, y, halfW, 64f)
        text(c, "↓ ${rate(s.netRx)}   ↑ ${rate(s.netTx)}", 44f, y + 41f, 21f, pal.cyan)
        card(c, 24f + halfW + gap, y, halfW, 64f)
        text(c, fit(device.ifBlank { s.fans.take(2).joinToString("  ") { "${it.first.split(" ").first()} %.0f rpm".format(it.second) } }, halfW - 40f, 21f),
            44f + halfW + gap, y + 41f, 21f, pal.muted)
        y += 64f + gap
        if (s.alerts.isNotEmpty()) {
            card(c, 24f, y, w - 48f, 52f, pal.alertBg)
            text(c, fit("⚠ " + s.alerts.joinToString(" · "), w - 88f, 21f), 44f, y + 34f, 21f, pal.red)
        }
    }

    private fun alertOrDevice(c: Canvas, s: Snap, x: Float, y: Float, w: Float): Float {
        if (s.alerts.isNotEmpty()) {
            card(c, x, y, w, 56f, pal.alertBg)
            text(c, fit("⚠ " + s.alerts.joinToString(" · "), w - 36f, 20f), x + 18f, y + 36f, 20f, pal.red)
            return y + 72f
        }
        return y
    }
}
