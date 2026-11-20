package com.akaineko.markii

import org.json.JSONArray
import org.json.JSONObject
import java.net.HttpURLConnection
import java.net.URL

/** The calls of the Mark II server (crates/markii-core/src/remote.rs). */
object Api {
    const val DEFAULT_PORT = 47820

    class HttpError(val code: Int, message: String) : Exception(message)

    private fun url(address: String, path: String): URL {
        val a = address.trim().removePrefix("http://").trimEnd('/')
        val withPort = if (a.contains(":")) a else "$a:$DEFAULT_PORT"
        return URL("http://$withPort$path")
    }

    private fun call(address: String, path: String, token: String? = null, body: String? = null): JSONObject {
        val c = url(address, path).openConnection() as HttpURLConnection
        try {
            c.connectTimeout = 2500
            c.readTimeout = 2500
            c.useCaches = false
            if (token != null) c.setRequestProperty("Authorization", "Bearer $token")
            if (body != null) {
                c.requestMethod = "POST"
                c.doOutput = true
                c.setRequestProperty("Content-Type", "application/json")
                c.outputStream.use { it.write(body.toByteArray()) }
            }
            val code = c.responseCode
            val text = (if (code in 200..299) c.inputStream else c.errorStream)?.bufferedReader()?.use { it.readText() } ?: ""
            val json = runCatching { JSONObject(text) }.getOrElse { JSONObject() }
            if (code !in 200..299) throw HttpError(code, json.optString("error", "erro $code"))
            return json
        } finally {
            c.disconnect()
        }
    }

    /** Pairs with the PIN shown in Mark II; returns the token to keep. */
    fun pair(address: String, pin: String, device: String): String {
        val body = JSONObject().put("pin", pin.trim()).put("device", device).toString()
        return call(address, "/v1/pair", body = body).getString("token")
    }

    /** Reads the PC; when `device` is given, also tells the PC this device's status. */
    fun snapshot(address: String, token: String, device: JSONObject? = null): Snap =
        Snap.from(call(address, "/v1/snapshot", token = token, body = device?.toString()))
}

/** Mirror of markii_core::snapshot::Snapshot. */
data class Snap(
    val host: String,
    val uptime: Long,
    val cpuName: String,
    val cpuLoad: Double,
    val cpuTemp: Double?,
    val cpuPower: Double?,
    val cpuMhz: Long,
    val gpuName: String,
    val gpuLoad: Double?,
    val gpuTemp: Double?,
    val gpuPower: Double?,
    val gpuHotspot: Double?,
    val gpuClock: Long?,
    val gpuFan: Long?,
    val vramUsed: Long?,
    val vramTotal: Long?,
    val ramUsed: Long,
    val ramTotal: Long,
    val netRx: Double,
    val netTx: Double,
    val fans: List<Pair<String, Double>>,
    val alerts: List<String>,
    /** Load of each logical thread, 0-100 (PC v0.7.2+; empty before). */
    val cores: List<Double> = emptyList(),
    /** (drive, used, total) */
    val disks: List<Triple<String, Long, Long>> = emptyList(),
    /** SteamVR batteries: (name, percent, charging) (PC v0.7.3+). */
    val vrBatteries: List<Triple<String, Double, Boolean>> = emptyList(),
) {
    val ramPct: Double get() = if (ramTotal > 0) ramUsed * 100.0 / ramTotal else 0.0

    companion object {
        private fun JSONObject.num(k: String): Double? = if (isNull(k) || !has(k)) null else optDouble(k).takeIf { !it.isNaN() }
        private fun JSONObject.long(k: String): Long? = num(k)?.toLong()

        fun from(j: JSONObject): Snap {
            val fans = mutableListOf<Pair<String, Double>>()
            (j.optJSONArray("fans") ?: JSONArray()).let { a ->
                for (i in 0 until a.length()) {
                    val f = a.optJSONArray(i) ?: continue
                    fans.add(f.optString(0) to f.optDouble(1))
                }
            }
            val cores = mutableListOf<Double>()
            (j.optJSONArray("cores") ?: JSONArray()).let { a -> for (i in 0 until a.length()) cores.add(a.optDouble(i, 0.0)) }
            val disks = mutableListOf<Triple<String, Long, Long>>()
            (j.optJSONArray("disks") ?: JSONArray()).let { a ->
                for (i in 0 until a.length()) {
                    val d = a.optJSONArray(i) ?: continue
                    disks.add(Triple(d.optString(0), d.optLong(1), d.optLong(2)))
                }
            }
            val vrb = mutableListOf<Triple<String, Double, Boolean>>()
            (j.optJSONArray("vr_batteries") ?: JSONArray()).let { a ->
                for (i in 0 until a.length()) {
                    val b = a.optJSONObject(i) ?: continue
                    vrb.add(Triple(b.optString("name"), b.optDouble("pct", 0.0), b.optBoolean("charging")))
                }
            }
            val alerts = mutableListOf<String>()
            (j.optJSONArray("alerts") ?: JSONArray()).let { a -> for (i in 0 until a.length()) alerts.add(a.optString(i)) }
            return Snap(
                host = j.optString("host"),
                uptime = j.long("uptime") ?: 0,
                cpuName = j.optString("cpu_name"),
                cpuLoad = j.num("cpu_load") ?: 0.0,
                cpuTemp = j.num("cpu_temp"),
                cpuPower = j.num("cpu_power"),
                cpuMhz = j.long("cpu_mhz") ?: 0,
                gpuName = j.optString("gpu_name"),
                gpuLoad = j.num("gpu_load"),
                gpuTemp = j.num("gpu_temp"),
                gpuPower = j.num("gpu_power"),
                gpuHotspot = j.num("gpu_hotspot"),
                gpuClock = j.long("gpu_clock"),
                gpuFan = j.long("gpu_fan"),
                vramUsed = j.long("vram_used"),
                vramTotal = j.long("vram_total"),
                ramUsed = j.long("ram_used") ?: 0,
                ramTotal = j.long("ram_total") ?: 0,
                netRx = j.num("net_rx") ?: 0.0,
                netTx = j.num("net_tx") ?: 0.0,
                fans = fans,
                alerts = alerts,
                cores = cores,
                disks = disks,
                vrBatteries = vrb,
            )
        }
    }
}
