package com.akaineko.markii

import android.content.Context
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.RectF
import android.graphics.Typeface
import android.view.View
import kotlin.math.min
import kotlin.math.roundToInt

/** The Mark II panel, drawn like the SteamVR wrist panel; scales to any window size. */
class PanelView(context: Context) : View(context) {
    var snap: Snap? = null
        set(v) { field = v; invalidate() }
    var status: String = "conectando…"
        set(v) { field = v; invalidate() }

    private val p = Paint(Paint.ANTI_ALIAS_FLAG)
    private val bold = Typeface.create(Typeface.DEFAULT, Typeface.BOLD)

    var pal: Palette = Themes.current(context)
        set(v) { field = v; invalidate() }

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
    }

    private fun text(c: Canvas, s: String, x: Float, y: Float, size: Float, color: Int, b: Boolean = false) {
        p.color = color
        p.textSize = size
        p.typeface = if (b) bold else Typeface.DEFAULT
        c.drawText(s, x, y, p)
    }

    private fun rect(c: Canvas, x: Float, y: Float, w: Float, h: Float, color: Int) {
        p.color = color
        c.drawRect(x, y, x + w, y + h, p)
    }

    private fun bar(c: Canvas, x: Float, y: Float, w: Float, h: Float, pct: Double, color: Int) {
        val segs = 24
        val gap = h / 6
        val on = ((pct.coerceIn(0.0, 100.0) / 100.0) * segs).roundToInt()
        val sw = (w - (segs - 1) * gap) / segs
        for (i in 0 until segs) rect(c, x + i * (sw + gap), y, sw, h, if (i < on) color else pal.line)
    }

    override fun onDraw(c: Canvas) {
        c.drawColor(pal.bg)
        // design units: 640 x 400
        val k = min(width / 640f, height / 400f)
        val ox = (width - 640 * k) / 2
        val oy = (height - 400 * k) / 2
        c.save()
        c.translate(ox, oy)
        c.scale(k, k)
        rect(c, 0f, 0f, 640f, 4f, pal.amber)
        text(c, "MARK", 22f, 44f, 26f, pal.text, true)
        text(c, "II", 22f + p.measureText("MARK "), 44f, 26f, pal.amber, true)
        val s = snap
        if (s == null) {
            text(c, status, 22f, 120f, 22f, pal.muted)
            c.restore()
            return
        }
        text(c, s.host.take(24), 300f, 42f, 18f, pal.muted)

        data class Row(val name: String, val col: Int, val pct: Double, val big: String, val detail: String, val dcol: Int)
        fun opt(v: Double?, unit: String) = v?.let { "%.0f%s".format(it, unit) } ?: "—"
        val vram = if (s.vramUsed != null && s.vramTotal != null && s.vramTotal > 0) "  ·  ${gb(s.vramUsed)}/${gb(s.vramTotal)} GB" else ""
        val rows = listOf(
            Row("CPU", pal.indigo, s.cpuLoad, "%.0f%%".format(s.cpuLoad), "${opt(s.cpuTemp, "°C")}  ·  ${opt(s.cpuPower, " W")}", pal.tempCol(s.cpuTemp)),
            Row("GPU", pal.green, s.gpuLoad ?: 0.0, opt(s.gpuLoad, "%"), "${opt(s.gpuTemp, "°C")}  ·  ${opt(s.gpuPower, " W")}$vram", pal.tempCol(s.gpuTemp)),
            Row("RAM", pal.amber, s.ramPct, "%.0f%%".format(s.ramPct), "${gb(s.ramUsed)} / ${gb(s.ramTotal)} GB", pal.muted),
        )
        var y = 66f
        for (r in rows) {
            rect(c, 14f, y, 612f, 74f, pal.panel)
            rect(c, 14f, y + 12f, 3f, 18f, r.col)
            text(c, r.name, 26f, y + 30f, 18f, pal.muted)
            text(c, r.big, 26f, y + 64f, 32f, pal.text, true)
            bar(c, 150f, y + 18f, 460f, 12f, r.pct, r.col)
            text(c, r.detail, 150f, y + 62f, 21f, r.dcol)
            y += 82f
        }
        text(c, "↓ ${rate(s.netRx)}   ↑ ${rate(s.netTx)}", 24f, y + 20f, 17f, pal.cyan)
        val fans = s.fans.take(3).joinToString("  ") { "${it.first.split(" ").first()} ${"%.0f".format(it.second)} rpm" }
        if (fans.isNotEmpty()) text(c, fans, 300f, y + 20f, 17f, pal.muted)
        y += 32f
        if (s.alerts.isNotEmpty()) {
            rect(c, 14f, y, 612f, 28f, pal.alertBg)
            text(c, "⚠ " + s.alerts.joinToString(" · "), 24f, y + 20f, 17f, pal.red)
        } else {
            text(c, status, 24f, y + 20f, 15f, if (status == "ao vivo") pal.green else pal.muted)
        }
        c.restore()
    }
}
