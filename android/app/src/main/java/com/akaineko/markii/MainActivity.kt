package com.akaineko.markii

import android.app.Activity
import android.app.AlertDialog
import android.content.res.ColorStateList
import android.content.Context
import android.graphics.Typeface
import android.os.Build
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
import android.widget.TextView
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

    private fun showPanel(keep: Snap? = null) {
        val root = FrameLayout(this)
        val pv = PanelView(this)
        pv.pal = pal
        if (keep != null) { pv.snap = keep; pv.status = "ao vivo" }
        root.addView(pv, FrameLayout.LayoutParams(MATCH_PARENT, MATCH_PARENT))
        val bar = LinearLayout(this).apply { orientation = LinearLayout.HORIZONTAL; alpha = 0.8f }
        bar.addView(accentButton("Tema", false).apply {
            textSize = 12f
            setOnClickListener { pickTheme() }
        })
        bar.addView(accentButton("Desconectar", false).apply {
            textSize = 12f
            setOnClickListener {
                prefs.edit().remove("token").apply()
                showConnect(null)
            }
        }, LinearLayout.LayoutParams(WRAP_CONTENT, WRAP_CONTENT).apply { marginStart = dp(6) })
        root.addView(bar, FrameLayout.LayoutParams(WRAP_CONTENT, WRAP_CONTENT, Gravity.BOTTOM or Gravity.END).apply {
            setMargins(0, 0, dp(8), dp(8))
        })
        setContentView(root)
        panel = pv
        startPolling()
    }

    private fun startPolling() {
        if (!polling.compareAndSet(false, true)) return
        io.execute {
            var fails = 0
            while (polling.get()) {
                val r = runCatching {
                    val me = if (shareDevice) runCatching { DeviceInfo.collect(this) }.getOrNull() else null
                    Api.snapshot(address, token, me)
                }
                val e = r.exceptionOrNull()
                if (e is Api.HttpError && e.code == 401) {
                    polling.set(false)
                    prefs.edit().remove("token").apply()
                    ui.post { showConnect("O PC removeu este aparelho. Pareie de novo com o PIN.") }
                    break
                }
                fails = if (r.isSuccess) 0 else fails + 1
                ui.post {
                    val pv = panel ?: return@post
                    r.onSuccess { pv.snap = it; pv.status = "ao vivo" }
                    if (r.isFailure) pv.status = if (fails > 2) "sem conexão com o PC (o Mark II está aberto?)" else "reconectando…"
                }
                Thread.sleep(1000)
            }
        }
    }
}
