package com.dosaki.maya.mobile

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.wifi.WifiManager
import android.os.Build
import android.os.IBinder
import android.os.PowerManager
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat

/**
 * Keeps the process, and so the Rust server in it, alive with the screen
 * off: a foreground service with an ongoing notification, a partial wake
 * lock and a Wi‑Fi lock. It runs nothing itself.
 */
class KeepAliveService : Service() {
    private var wake: PowerManager.WakeLock? = null
    private var wifi: WifiManager.WifiLock? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_STOP) {
            release()
            ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE)
            stopSelf()
            return START_NOT_STICKY
        }
        // A restart by Android after the process died: the Rust server died
        // with it and nothing here can bring it back, so do not hold locks for it.
        if (intent == null) {
            stopSelf()
            return START_NOT_STICKY
        }
        val line = intent.getStringExtra(EXTRA_LINE) ?: ""
        ensureChannel()
        val type = if (Build.VERSION.SDK_INT >= 29) ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE else 0
        ServiceCompat.startForeground(this, NOTIFICATION_ID, notification(this, line), type)
        acquire()
        running = true
        return START_NOT_STICKY
    }

    override fun onDestroy() {
        release()
        super.onDestroy()
    }

    private fun ensureChannel() {
        val nm = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        val channel = NotificationChannel(CHANNEL, "Server", NotificationManager.IMPORTANCE_LOW)
        channel.description = "Keeps the main Maya running with the screen off"
        nm.createNotificationChannel(channel)
    }

    private fun acquire() {
        if (wake == null) {
            val pm = getSystemService(Context.POWER_SERVICE) as PowerManager
            wake = pm.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "maya:server").also { it.acquire() }
        }
        if (wifi == null) {
            val wm = applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager
            @Suppress("DEPRECATION")
            wifi = wm.createWifiLock(WifiManager.WIFI_MODE_FULL_HIGH_PERF, "maya:server").also { it.acquire() }
        }
    }

    private fun release() {
        running = false
        wake?.takeIf { it.isHeld }?.release()
        wake = null
        wifi?.takeIf { it.isHeld }?.release()
        wifi = null
    }

    companion object {
        const val CHANNEL = "service"
        const val NOTIFICATION_ID = 1
        const val ACTION_STOP = "com.dosaki.maya.mobile.STOP"
        const val EXTRA_LINE = "line"

        /** Whether the service is in the foreground, so `update` has a notification to replace. */
        @Volatile
        var running = false
            private set

        private fun notification(ctx: Context, line: String): Notification {
            val launch = ctx.packageManager.getLaunchIntentForPackage(ctx.packageName)
            val open = PendingIntent.getActivity(ctx, 0, launch, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
            return NotificationCompat.Builder(ctx, CHANNEL)
                .setSmallIcon(R.drawable.ic_stat_maya)
                .setContentTitle("Maya")
                .setContentText(line)
                .setOngoing(true)
                .setContentIntent(open)
                .setPriority(NotificationCompat.PRIORITY_LOW)
                .build()
        }

        fun start(ctx: Context, line: String) {
            val intent = Intent(ctx, KeepAliveService::class.java).putExtra(EXTRA_LINE, line)
            if (Build.VERSION.SDK_INT >= 26) ctx.startForegroundService(intent) else ctx.startService(intent)
        }

        /**
         * Replaces the running service's notification with a new line, in
         * place: no new foreground-service start, which Android 12+ refuses
         * from the background. Does nothing when the service is not running.
         */
        fun update(ctx: Context, line: String) {
            if (!running) return
            val nm = ctx.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
            nm.notify(NOTIFICATION_ID, notification(ctx, line))
        }

        fun stop(ctx: Context) {
            ctx.stopService(Intent(ctx, KeepAliveService::class.java))
        }
    }
}
