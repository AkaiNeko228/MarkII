package com.akaineko.markii

import android.content.Context
import android.os.Build

/**
 * The Mark II palettes (same as crates/markii-gpui/src/theme.rs) plus "Material You",
 * which takes the colors of the wallpaper on Android 12+.
 */
class Palette(val key: String, val name: String, val light: Boolean, grey: IntArray, accent: IntArray) {
    private fun argb(c: Int) = c or 0xFF000000.toInt()
    val bg = argb(grey[0])
    val panel = argb(grey[1])
    val panel2 = argb(grey[2])
    val line = argb(grey[3])
    val line2 = argb(grey[4])
    val dim = argb(grey[5])
    val muted = argb(grey[6])
    val text = argb(grey[7])
    val amber = argb(accent[0]) // main accent
    val indigo = argb(accent[1])
    val green = argb(accent[2])
    val cyan = argb(accent[3])
    val red = argb(accent[4])
    val pink = argb(accent[5])
    val yellow = argb(accent[6])
    val orange = argb(accent[7])

    /** Red tinted towards the background, for the alert strip. */
    val alertBg: Int get() = mix(red, bg, 0.22)

    fun tempCol(t: Double?) = when {
        t == null -> muted
        t >= 85 -> red
        t >= 72 -> amber
        else -> green
    }

    companion object {
        fun mix(a: Int, b: Int, k: Double): Int {
            fun ch(c: Int, s: Int) = (c shr s) and 0xFF
            fun m(s: Int) = (ch(a, s) * k + ch(b, s) * (1 - k)).toInt().coerceIn(0, 255)
            return (0xFF shl 24) or (m(16) shl 16) or (m(8) shl 8) or m(0)
        }
    }
}

object Themes {
    const val MATERIAL_YOU = "material"
    private const val PREF = "theme"

    val fixed = listOf(
        Palette("grafite", "Grafite", false,
            intArrayOf(0x0a0d11, 0x11151b, 0x171c24, 0x222a35, 0x2c3542, 0x556070, 0x8592a3, 0xe8edf3),
            intArrayOf(0xffb020, 0x7b7ff5, 0x3fd17a, 0x44c7e0, 0xff5a52, 0xff5c8a, 0xf2cc3d, 0xff8a3d)),
        Palette("roxo", "Roxo", false,
            intArrayOf(0x07060b, 0x0e0c15, 0x15121f, 0x221d33, 0x2d2643, 0x5e5577, 0x9a90b5, 0xece8f6),
            intArrayOf(0xb57cff, 0x8f6bff, 0x5fd49a, 0x7aa8ff, 0xff5f7a, 0xe06bff, 0xf0c75e, 0xff8f6b)),
        Palette("carmesim", "Carmesim", false,
            intArrayOf(0x0b0708, 0x130c0e, 0x1b1114, 0x2a181c, 0x372026, 0x6e5157, 0xa88d92, 0xf2e6e8),
            intArrayOf(0xe0284f, 0xff6b81, 0x5fd49a, 0x6fb8e8, 0xff3b3b, 0xff7aa8, 0xf2c14e, 0xff7a45)),
        Palette("azul", "Azul profundo", false,
            intArrayOf(0x050a14, 0x0a1220, 0x0f1a2c, 0x17263f, 0x1f3150, 0x4d6385, 0x8ea3c4, 0xe6eefb),
            intArrayOf(0x3d8bff, 0x6c8cff, 0x4fd6a0, 0x4cc9f0, 0xff5a6a, 0xb48cff, 0xf2d06b, 0xff9a5a)),
        Palette("lavanda", "Lavanda", false,
            intArrayOf(0x14121d, 0x1b1927, 0x232031, 0x2f2b40, 0x3a354e, 0x6b6585, 0xa39cbc, 0xeeeaf8),
            intArrayOf(0xf6c98e, 0xb6a9f6, 0x9fdfb9, 0xa0d6ee, 0xf4a0a0, 0xf3abcb, 0xf1df9c, 0xf5b58f)),
        Palette("menta", "Menta", false,
            intArrayOf(0x0e1818, 0x142121, 0x1a2a2a, 0x243736, 0x2e4443, 0x5c7672, 0x93afaa, 0xe6f3f0),
            intArrayOf(0xf1d19c, 0xabb9f1, 0x9fe5c2, 0x91dbe0, 0xf1a3a3, 0xefb2d1, 0xede2a1, 0xf2be96)),
        Palette("algodao", "Algodão-doce", true,
            intArrayOf(0xf5eff7, 0xfffbff, 0xf2e9f5, 0xe4d7ea, 0xd5c5dc, 0xa797b1, 0x776a85, 0x382e44),
            intArrayOf(0xd08a2c, 0x7d6fd9, 0x2f9f6a, 0x2c92b5, 0xd9534f, 0xcf5a92, 0xa98c12, 0xd97b3d)),
        Palette("pistache", "Pistache", true,
            intArrayOf(0xeff5ef, 0xfbfefb, 0xe8f2ea, 0xd4e4d8, 0xc2d6c8, 0x98ae9f, 0x667d70, 0x2a3831),
            intArrayOf(0xc8862a, 0x6a80d2, 0x2a9663, 0x2a8faa, 0xd0524d, 0xc35b8a, 0x9c8410, 0xcf773d)),
    )

    val materialAvailable get() = Build.VERSION.SDK_INT >= Build.VERSION_CODES.S

    /** Every choice shown to the user: (key, name). */
    fun choices(): List<Pair<String, String>> =
        (if (materialAvailable) listOf(MATERIAL_YOU to "Material You (cores do papel de parede)") else emptyList()) +
            fixed.map { it.key to it.name }

    fun selectedKey(ctx: Context): String {
        val def = if (materialAvailable) MATERIAL_YOU else "grafite"
        return ctx.getSharedPreferences("markii", Context.MODE_PRIVATE).getString(PREF, def) ?: def
    }

    fun select(ctx: Context, key: String) {
        ctx.getSharedPreferences("markii", Context.MODE_PRIVATE).edit().putString(PREF, key).apply()
    }

    fun current(ctx: Context): Palette {
        val key = selectedKey(ctx)
        if (key == MATERIAL_YOU && materialAvailable) return materialYou(ctx)
        return fixed.firstOrNull { it.key == key } ?: fixed[0]
    }

    @android.annotation.TargetApi(Build.VERSION_CODES.S)
    /** Dark Material You palette from the system's dynamic tonal palettes (Android 12+). */
    private fun materialYou(ctx: Context): Palette {
        fun c(id: Int) = ctx.getColor(id) and 0xFFFFFF
        val grafite = fixed[0]
        // status colors stay recognisable (green = ok, red = hot) but lean toward the wallpaper
        fun harmonize(base: Int, tint: Int) = Palette.mix(base, tint or 0xFF000000.toInt(), 0.8) and 0xFFFFFF
        val tint = c(android.R.color.system_accent1_200)
        return Palette(
            MATERIAL_YOU, "Material You", false,
            intArrayOf(
                c(android.R.color.system_neutral1_900), c(android.R.color.system_neutral1_800), c(android.R.color.system_neutral2_800),
                c(android.R.color.system_neutral2_700), c(android.R.color.system_neutral2_600), c(android.R.color.system_neutral2_500),
                c(android.R.color.system_neutral2_200), c(android.R.color.system_neutral1_50),
            ),
            intArrayOf(
                c(android.R.color.system_accent1_200), c(android.R.color.system_accent3_200), harmonize(grafite.green, tint),
                c(android.R.color.system_accent2_200), harmonize(grafite.red, tint), c(android.R.color.system_accent3_300),
                harmonize(grafite.yellow, tint), harmonize(grafite.orange, tint),
            ),
        )
    }
}
