package com.dosaki.maya.mobile

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.PowerManager
import android.provider.Settings
import android.webkit.WebView
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
import org.json.JSONObject

@InvokeArg
class LineArgs {
    var line: String = ""
}

/**
 * What the Rust side cannot do itself: the foreground service that keeps
 * the process alive, the battery exemption dialog, whether Android lets
 * Maya notify, the device's name and addresses, and which notification
 * was tapped.
 */
@TauriPlugin
class KeepAlivePlugin(private val activity: Activity) : Plugin(activity) {
    /** The id of a tapped notification, until the page takes it. */
    @Volatile
    private var pendingId: Int? = null

    // The notification plugin's tap event never names the notification (it
    // leaves the posted JSON off the intent), and on a cold start it fires
    // before the page listens, or not at all when the tap arrives before
    // the plugins are registered (MainActivity keeps that intent for
    // `load()`). The page asks for the tap's id with `pendingTap` instead.
    override fun load(webView: WebView) {
        super.load(webView)
        activity.intent?.let { remember(it) }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        remember(intent)
    }

    private fun remember(intent: Intent) {
        // Reopened from recents, Android hands back the intent that first
        // launched the task; that tap was taken already.
        if (intent.flags and Intent.FLAG_ACTIVITY_LAUNCHED_FROM_HISTORY != 0) return
        if (intent.getBooleanExtra(TAP_TAKEN, false)) return
        if (intent.getStringExtra(NOTIFICATION_ACTION) != TAP) return
        val id = intent.getIntExtra(NOTIFICATION_ID, Int.MIN_VALUE)
        if (id == Int.MIN_VALUE) return
        // The activity keeps its intent across a recreation: mark it read.
        intent.putExtra(TAP_TAKEN, true)
        pendingId = id
    }

    @Command
    fun pendingTap(invoke: Invoke) {
        val ret = JSObject()
        ret.put("value", pendingId ?: JSONObject.NULL)
        pendingId = null
        invoke.resolve(ret)
    }

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

/** The extras tauri-plugin-notification puts on a tap's intent, and its tap action. */
private const val NOTIFICATION_ID = "NotificationId"
private const val NOTIFICATION_ACTION = "NotificationUserAction"
private const val TAP = "tap"

/** Our mark on an intent whose tap was already remembered. */
private const val TAP_TAKEN = "com.dosaki.maya.mobile.tapTaken"
