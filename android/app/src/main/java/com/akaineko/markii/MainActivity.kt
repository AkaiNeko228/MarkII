package com.akaineko.markii

import android.app.Activity
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

    private val address get() = prefs.getString("address", "") ?: ""
    private val token get() = prefs.getString("token", "") ?: ""

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        if (token.isEmpty()) showConnect(null) else showPanel()
    }

    override fun onResume() {
        super.onResume()
        if (panel != null) startPolling()
    }

    override fun onPause() {
        super.onPause()
        polling.set(false)
    }

    private fun dp(v: Int) = (v * resources.displayMetrics.density).toInt()

    private fun showConnect(message: String?) {
        polling.set(false)
        panel = null
        val col = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            gravity = Gravity.CENTER_HORIZONTAL
            setPadding(dp(32), dp(28), dp(32), dp(28))
            setBackgroundColor(PanelView.BG)
        }
        fun label(t: String, size: Float, color: Int, bold: Boolean = false) = TextView(this).apply {
            text = t
            textSize = size
            setTextColor(color)
            if (bold) typeface = Typeface.DEFAULT_BOLD
        }
        col.addView(label("MARK II", 28f, PanelView.AMBER, true))
        col.addView(label("No PC, abra o Mark II > Configurações > Celular e óculos, ligue a opção e use o endereço e o PIN mostrados.", 15f, PanelView.MUTED).apply {
            setPadding(0, dp(8), 0, dp(16))
            gravity = Gravity.CENTER
        })
        val addr = EditText(this).apply {
            hint = "endereço (ex.: 192.168.0.10:47820)"
            setText(address)
            inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_URI
            setTextColor(PanelView.TEXT)
            setHintTextColor(PanelView.MUTED)
            isSingleLine = true
        }
        val pin = EditText(this).apply {
            hint = "PIN (6 dígitos)"
            inputType = InputType.TYPE_CLASS_NUMBER
            setTextColor(PanelView.TEXT)
            setHintTextColor(PanelView.MUTED)
            isSingleLine = true
        }
        val status = label(message ?: "", 15f, if (message == null) PanelView.MUTED else PanelView.RED).apply {
            setPadding(0, dp(12), 0, 0)
            gravity = Gravity.CENTER
        }
        val go = Button(this).apply {
            text = "Conectar"
            setOnClickListener {
                val a = addr.text.toString().trim()
                val p = pin.text.toString().trim()
                if (a.isEmpty() || p.length != 6) {
                    status.setTextColor(PanelView.RED)
                    status.text = "Digite o endereço e os 6 dígitos do PIN."
                    return@setOnClickListener
                }
                isEnabled = false
                status.setTextColor(PanelView.MUTED)
                status.text = "conectando…"
                io.execute {
                    val r = runCatching { Api.pair(a, p, "${Build.MANUFACTURER} ${Build.MODEL}".trim()) }
                    ui.post {
                        isEnabled = true
                        r.onSuccess { t ->
                            prefs.edit().putString("address", a).putString("token", t).apply()
                            showPanel()
                        }.onFailure { e ->
                            status.setTextColor(PanelView.RED)
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
        setContentView(col)
    }

    private fun showPanel() {
        val root = FrameLayout(this)
        val pv = PanelView(this)
        root.addView(pv, FrameLayout.LayoutParams(MATCH_PARENT, MATCH_PARENT))
        val out = Button(this).apply {
            text = "Desconectar"
            textSize = 12f
            alpha = 0.7f
            setOnClickListener {
                prefs.edit().remove("token").apply()
                showConnect(null)
            }
        }
        root.addView(out, FrameLayout.LayoutParams(WRAP_CONTENT, WRAP_CONTENT, Gravity.BOTTOM or Gravity.END).apply {
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
                val r = runCatching { Api.snapshot(address, token) }
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
