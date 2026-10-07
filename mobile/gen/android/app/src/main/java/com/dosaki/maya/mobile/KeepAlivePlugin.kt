package com.dosaki.maya.mobile

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.PowerManager
import android.provider.Settings
import androidx.core.app.NotificationManagerCompat
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.net.Inet4Address
import java.net.NetworkInterface

@InvokeArg
class LineArgs {
    var line: String = ""
}

/**
 * What the Rust side cannot do itself: the foreground service that keeps
 * the process alive, the battery exemption dialog, whether Android lets
 * Maya notify, and the device's name and addresses.
 */
@TauriPlugin
class KeepAlivePlugin(private val activity: Activity) : Plugin(activity) {
    @Command
    fun start(invoke: Invoke) {
        val args = invoke.parseArgs(LineArgs::class.java)
        KeepAliveService.start(activity, args.line)
        invoke.resolve()
    }

    @Command
    fun update(invoke: Invoke) {
        val args = invoke.parseArgs(LineArgs::class.java)
        KeepAliveService.update(activity, args.line)
        invoke.resolve()
    }

    @Command
    fun stop(invoke: Invoke) {
        KeepAliveService.stop(activity)
        invoke.resolve()
    }

    @Command
    fun requestBatteryExemption(invoke: Invoke) {
        val pm = activity.getSystemService(Context.POWER_SERVICE) as PowerManager
        if (!pm.isIgnoringBatteryOptimizations(activity.packageName)) {
            val intent = Intent(Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS, Uri.parse("package:" + activity.packageName))
            activity.startActivity(intent)
        }
        invoke.resolve()
    }

    @Command
    fun notificationsAllowed(invoke: Invoke) {
        val ret = JSObject()
        ret.put("value", NotificationManagerCompat.from(activity).areNotificationsEnabled())
        invoke.resolve(ret)
    }

    @Command
    fun deviceModel(invoke: Invoke) {
        val ret = JSObject()
        ret.put("value", Build.MODEL ?: "")
        invoke.resolve(ret)
    }

    @Command
    fun localAddresses(invoke: Invoke) {
        val out = JSArray()
        try {
            for (nic in NetworkInterface.getNetworkInterfaces().toList()) {
                if (!nic.isUp || nic.isLoopback) continue
                for (addr in nic.inetAddresses.toList()) {
                    if (addr is Inet4Address && !addr.isLoopbackAddress) out.put(addr.hostAddress)
                }
            }
        } catch (_: Exception) {
            // no interfaces to list: the page says to connect to Wi‑Fi
        }
        val ret = JSObject()
        ret.put("value", out)
        invoke.resolve(ret)
    }
}
