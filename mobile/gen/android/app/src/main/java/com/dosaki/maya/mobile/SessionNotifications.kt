package com.dosaki.maya.mobile

import android.app.Notification
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import androidx.core.app.NotificationCompat
import androidx.core.app.RemoteInput
import app.tauri.annotation.InvokeArg
import app.tauri.plugin.Channel
import app.tauri.plugin.JSObject
import java.util.concurrent.ConcurrentHashMap

@InvokeArg
class ButtonArgs {
    var label: String = ""
    /** "option" picks a menu row; "own" and "reply" open a text box. */
    var kind: String = ""
    var ask: Long = 0
    var question: Int = 0
    var index: Int = 0
}

@InvokeArg
class NotifyArgs {
    var id: Int = 0
    var channelId: String = ""
    var title: String = ""
    var body: String = ""
    var longBody: String = ""
    var sessionId: String = ""
    var buttons: Array<ButtonArgs> = arrayOf()
    /** Replaces the one on screen without a sound of its own. */
    var update: Boolean = false
}

/**
 * Maya's notifications for a session's ask or finished turn: the whole text
 * when pulled down, and buttons that answer without opening the app. The
 * notification plugin cannot post these: its buttons open the app. A press
 * goes to `SessionNotificationReceiver`, and on to the Rust side through
 * the channel `KeepAlivePlugin.watchButtons` keeps here.
 */
object SessionNotifications {
    @Volatile
    var events: Channel? = null

    /** What is on screen, by id, to show again once a button is pressed. */
    private val shown = ConcurrentHashMap<Int, NotifyArgs>()

    fun post(ctx: Context, n: NotifyArgs) {
        shown[n.id] = n
        manager(ctx).notify(n.id, build(ctx, n, sent = null, typed = null, again = false))
    }

    /**
     * After a press: the notification without its buttons, saying what was
     * sent, until the board moves on and clears it (or the Rust side puts it
     * back with why it failed). Android shows a spinner on a reply until the
     * notification is posted again.
     */
    fun sent(ctx: Context, id: Int, line: String, typed: String?) {
        val n = shown[id] ?: return manager(ctx).cancel(id)
        manager(ctx).notify(id, build(ctx, n, line, typed, again = true))
    }

    /** A blank reply: the notification as it was, to stop the spinner. */
    fun restore(ctx: Context, id: Int) {
        val n = shown[id] ?: return manager(ctx).cancel(id)
        manager(ctx).notify(id, build(ctx, n, sent = null, typed = null, again = true))
    }

    /**
     * A press after Android ended the process: the Rust side that would send
     * it is gone, and what the notification showed with it. Say so, and let
     * a tap open the app.
     */
    fun notRunning(ctx: Context, id: Int, channelId: String) {
        val n = NotificationCompat.Builder(ctx, channelId)
            .setSmallIcon(R.drawable.ic_stat_maya)
            .setContentTitle("Not sent")
            .setContentText("Maya was not running. Open it to answer.")
            .setContentIntent(tap(ctx, id))
            .setAutoCancel(true)
            .setOnlyAlertOnce(true)
            .build()
        manager(ctx).notify(id, n)
    }

    private fun manager(ctx: Context) = ctx.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager

    // `ic_stat_maya` is a white glyph: Android draws status-bar icons as
    // alpha masks, and the launcher icon shows as a blob.
    private fun build(ctx: Context, n: NotifyArgs, sent: String?, typed: String?, again: Boolean): Notification {
        val text = sent ?: n.body
        val long = n.longBody.ifEmpty { n.body }
        val b = NotificationCompat.Builder(ctx, n.channelId)
            .setSmallIcon(R.drawable.ic_stat_maya)
            .setContentTitle(n.title)
            .setContentText(text)
            .setStyle(NotificationCompat.BigTextStyle().bigText(if (sent != null) "$sent\n\n$long" else long))
            .setContentIntent(tap(ctx, n.id))
            .setAutoCancel(true)
            .setOnlyAlertOnce(n.update || again)
        if (typed != null) b.setRemoteInputHistory(arrayOf<CharSequence>(typed))
        if (sent == null) n.buttons.forEachIndexed { i, button -> b.addAction(action(ctx, n, i, button)) }
        return b.build()
    }

    // The notification plugin's own tap intent, so a tap opens the card
    // the way it always has (`KeepAlivePlugin.remember`, the page's `onAction`).
    private fun tap(ctx: Context, id: Int): PendingIntent {
        val intent = Intent(ctx, MainActivity::class.java).apply {
            action = Intent.ACTION_MAIN
            addCategory(Intent.CATEGORY_LAUNCHER)
            flags = Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP
            putExtra(NOTIFICATION_ID, id)
            putExtra(NOTIFICATION_ACTION, TAP)
        }
        return PendingIntent.getActivity(ctx, id, intent, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
    }

    private fun action(ctx: Context, n: NotifyArgs, i: Int, button: ButtonArgs): NotificationCompat.Action {
        val intent = Intent(ctx, SessionNotificationReceiver::class.java).apply {
            // Tells each button's intent apart: the extras do not.
            data = Uri.parse("maya-notification://${n.id}/$i")
            putExtra(EXTRA_ID, n.id)
            putExtra(EXTRA_CHANNEL, n.channelId)
            putExtra(EXTRA_SESSION, n.sessionId)
            putExtra(EXTRA_LABEL, button.label)
            putExtra(EXTRA_KIND, button.kind)
            putExtra(EXTRA_ASK, button.ask)
            putExtra(EXTRA_QUESTION, button.question)
            putExtra(EXTRA_INDEX, button.index)
        }
        val typed = button.kind != OPTION
        // A text box fills in the intent, so its intent must be mutable.
        val mutability = if (typed && Build.VERSION.SDK_INT >= 31) PendingIntent.FLAG_MUTABLE else PendingIntent.FLAG_IMMUTABLE
        val pending = PendingIntent.getBroadcast(ctx, i, intent, PendingIntent.FLAG_UPDATE_CURRENT or mutability)
        val a = NotificationCompat.Action.Builder(0, button.label, pending)
        if (typed) a.addRemoteInput(RemoteInput.Builder(TYPED).setLabel(button.label).build()).setAllowGeneratedReplies(false)
        return a.build()
    }
}

/** A button pressed on one of Maya's notifications. */
class SessionNotificationReceiver : BroadcastReceiver() {
    override fun onReceive(ctx: Context, intent: Intent) {
        val id = intent.getIntExtra(EXTRA_ID, Int.MIN_VALUE)
        val kind = intent.getStringExtra(EXTRA_KIND) ?: return
        if (id == Int.MIN_VALUE) return
        val text = RemoteInput.getResultsFromIntent(intent)?.getCharSequence(TYPED)?.toString()?.trim() ?: ""
        if (kind != OPTION && text.isEmpty()) return SessionNotifications.restore(ctx, id)
        val events = SessionNotifications.events
            ?: return SessionNotifications.notRunning(ctx, id, intent.getStringExtra(EXTRA_CHANNEL) ?: "decisions")
        events.send(JSObject().apply {
            put("sessionId", intent.getStringExtra(EXTRA_SESSION) ?: "")
            put("kind", kind)
            put("ask", intent.getLongExtra(EXTRA_ASK, 0))
            put("question", intent.getIntExtra(EXTRA_QUESTION, 0))
            put("index", intent.getIntExtra(EXTRA_INDEX, 0))
            put("text", text)
        })
        if (kind == OPTION) {
            SessionNotifications.sent(ctx, id, "Sent: ${intent.getStringExtra(EXTRA_LABEL) ?: ""}", null)
        } else {
            SessionNotifications.sent(ctx, id, "Sent", text)
        }
    }
}

private const val OPTION = "option"
/** The text box's key in the reply intent. */
private const val TYPED = "typed"

private const val EXTRA_ID = "com.dosaki.maya.mobile.id"
private const val EXTRA_CHANNEL = "com.dosaki.maya.mobile.channel"
private const val EXTRA_SESSION = "com.dosaki.maya.mobile.session"
private const val EXTRA_LABEL = "com.dosaki.maya.mobile.label"
private const val EXTRA_KIND = "com.dosaki.maya.mobile.kind"
private const val EXTRA_ASK = "com.dosaki.maya.mobile.ask"
private const val EXTRA_QUESTION = "com.dosaki.maya.mobile.question"
private const val EXTRA_INDEX = "com.dosaki.maya.mobile.index"
