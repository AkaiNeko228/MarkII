package com.akaineko.markii

import android.app.ActivityManager
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.net.wifi.WifiManager
import android.os.BatteryManager
import android.os.Build
import android.os.Environment
import android.os.PowerManager
import android.os.StatFs
import android.os.SystemClock
import org.json.JSONObject
import java.io.File

/**
 * What this phone / headset tells the PC about itself (shown in Mark II on the PC).
 * Only status numbers: no files, apps, location or anything personal.
 */
object DeviceInfo {
    fun name(): String {
        val m = Build.MODEL.trim()
        val brand = Build.MANUFACTURER.trim()
        return if (m.startsWith(brand, ignoreCase = true)) m else "$brand $m"
    }

    fun collect(ctx: Context): JSONObject {
        val j = JSONObject().put("name", name())
        runCatching {
            val b = ctx.registerReceiver(null, IntentFilter(Intent.ACTION_BATTERY_CHANGED))
            if (b != null) {
                val level = b.getIntExtra(BatteryManager.EXTRA_LEVEL, -1)
                val scale = b.getIntExtra(BatteryManager.EXTRA_SCALE, 100)
                if (level >= 0 && scale > 0) j.put("battery", level * 100.0 / scale)
                val st = b.getIntExtra(BatteryManager.EXTRA_STATUS, -1)
                j.put("charging", st == BatteryManager.BATTERY_STATUS_CHARGING || st == BatteryManager.BATTERY_STATUS_FULL)
                val t = b.getIntExtra(BatteryManager.EXTRA_TEMPERATURE, Int.MIN_VALUE)
                if (t != Int.MIN_VALUE) j.put("battery_temp", t / 10.0)
            }
        }
        runCatching {
            val mi = ActivityManager.MemoryInfo()
            (ctx.getSystemService(Context.ACTIVITY_SERVICE) as ActivityManager).getMemoryInfo(mi)
            j.put("ram_total", mi.totalMem).put("ram_used", mi.totalMem - mi.availMem)
        }
        runCatching {
            val fs = StatFs(Environment.getDataDirectory().path)
            j.put("storage_total", fs.totalBytes).put("storage_free", fs.availableBytes)
        }
        runCatching {
            @Suppress("DEPRECATION")
            val wi = (ctx.applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager).connectionInfo
            if (wi != null && wi.rssi in -127..-1) j.put("wifi_rssi", wi.rssi)
            if (wi != null && wi.linkSpeed > 0) j.put("wifi_mbps", wi.linkSpeed)
        }
        runCatching {
            j.put("thermal", (ctx.getSystemService(Context.POWER_SERVICE) as PowerManager).currentThermalStatus)
        }
        cpuTemp()?.let { j.put("cpu_temp", it) }
        j.put("uptime", SystemClock.elapsedRealtime() / 1000)
        return j
    }

    /** Hottest CPU-ish thermal zone, when the system lets apps read it (many don't). */
    private fun cpuTemp(): Double? = runCatching {
        val zones = File("/sys/class/thermal").listFiles { f -> f.name.startsWith("thermal_zone") } ?: return null
        zones.mapNotNull { z ->
            val type = runCatching { File(z, "type").readText().trim().lowercase() }.getOrNull() ?: return@mapNotNull null
            if (!(type.contains("cpu") || type.contains("tsens") || type.contains("soc"))) return@mapNotNull null
            val raw = runCatching { File(z, "temp").readText().trim().toDouble() }.getOrNull() ?: return@mapNotNull null
            val c = if (raw > 1000) raw / 1000 else raw
            c.takeIf { it in 5.0..130.0 }
        }.maxOrNull()
    }.getOrNull()
}
