package com.dosaki.maya.mobile

import android.Manifest
import android.app.Activity
import android.app.NotificationManager
import android.content.Context
import android.content.Intent
import android.media.AudioManager
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.speech.RecognitionListener
import android.speech.RecognizerIntent
import android.speech.SpeechRecognizer
import android.speech.tts.TextToSpeech
import android.speech.tts.UtteranceProgressListener
import android.webkit.WebView
import androidx.lifecycle.DefaultLifecycleObserver
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.ProcessLifecycleOwner
import app.tauri.PermissionState
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.Permission
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Channel
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.util.UUID

@InvokeArg
class ListenArgs {
    var wake: Boolean = false
}

@InvokeArg
class SpeakArgs {
    var line: String = ""
}

@InvokeArg
class WatchArgs {
    lateinit var channel: Channel
}

/**
 * Android's recogniser and voice for Maya's voice conversation, which the
 * Rust side runs. Nothing listens or speaks while the app is off screen:
 * this class stops both itself on `ON_STOP` and refuses `listen` and
 * `speak` until `ON_START`, whatever the page or Rust ask.
 */
@TauriPlugin(permissions = [Permission(strings = [Manifest.permission.RECORD_AUDIO], alias = "microphone")])
class VoicePlugin(private val activity: Activity) : Plugin(activity), DefaultLifecycleObserver {
    private val main = Handler(Looper.getMainLooper())
    private var recognizer: SpeechRecognizer? = null
    private var recognizerOnDevice = false
    private var tts: TextToSpeech? = null
    @Volatile private var ttsReady = false
    /** The line being spoken; an interrupted one reports nothing (ledger ruling R9). */
    @Volatile private var utterance: String? = null
    @Volatile private var onScreen = true
    /** Where events go: the Rust side's channel, from `watch`. */
    @Volatile private var events: Channel? = null

    private fun emit(event: String, fill: JSObject.() -> Unit = {}) {
        events?.send(JSObject().apply { put("event", event); fill() })
    }

    @Command
    fun watch(invoke: Invoke) {
        events = invoke.parseArgs(WatchArgs::class.java).channel
        // ON_START may have come before the channel did.
        emit("screen") { put("on", onScreen) }
        invoke.resolve()
    }

    override fun load(webView: WebView) {
        super.load(webView)
        main.post { ProcessLifecycleOwner.get().lifecycle.addObserver(this) }
        tts = TextToSpeech(activity) { status ->
            ttsReady = status == TextToSpeech.SUCCESS
            tts?.setOnUtteranceProgressListener(object : UtteranceProgressListener() {
                override fun onStart(utteranceId: String?) {}
                override fun onDone(utteranceId: String?) = finished(utteranceId, true)
                @Deprecated("Deprecated in Java")
                override fun onError(utteranceId: String?) = finished(utteranceId, false)
                override fun onError(utteranceId: String?, errorCode: Int) = finished(utteranceId, false)
            })
        }
    }

    override fun onStart(owner: LifecycleOwner) {
        onScreen = true
        emit("screen") { put("on", true) }
    }

    override fun onStop(owner: LifecycleOwner) {
        onScreen = false
        utterance = null
        main.post { stopRecognizer() }
        tts?.stop()
        emit("screen") { put("on", false) }
    }

    private fun spoken(ok: Boolean) {
        emit("spoken") { put("ok", ok) }
    }

    /** Only the latest line's end counts: a newer `speak` replaced the rest. */
    private fun finished(id: String?, ok: Boolean) {
        if (id == null || id != utterance) return
        utterance = null
        spoken(ok)
    }

    private fun onDeviceAvailable(): Boolean =
        Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU && SpeechRecognizer.isOnDeviceRecognitionAvailable(activity)

    @Command
    fun state(invoke: Invoke) {
        val ret = JSObject()
        ret.put("onScreen", onScreen)
        ret.put("available", SpeechRecognizer.isRecognitionAvailable(activity))
        ret.put("onDevice", onDeviceAvailable())
        invoke.resolve(ret)
    }

    @Command
    fun listen(invoke: Invoke) {
        val args = invoke.parseArgs(ListenArgs::class.java)
        if (!onScreen) return invoke.reject("Maya only listens while she is on screen.")
        if (!SpeechRecognizer.isRecognitionAvailable(activity)) {
            fail("This phone has no speech recognition.", true)
            return invoke.resolve()
        }
        if (getPermissionState("microphone") != PermissionState.GRANTED) {
            fail("Microphone access is off.", true)
            return invoke.resolve()
        }
        if (args.wake && !onDeviceAvailable()) {
            fail("The wake word needs on-device speech recognition, which this phone does not have.", true)
            return invoke.resolve()
        }
        main.post { startRecognizer(args.wake) }
        invoke.resolve()
    }

    @Command
    fun stopListening(invoke: Invoke) {
        main.post { stopRecognizer() }
        invoke.resolve()
    }

    @Command
    fun speak(invoke: Invoke) {
        val args = invoke.parseArgs(SpeakArgs::class.java)
        val engine = tts
        if (!onScreen || engine == null || !ttsReady) {
            spoken(false)
            return invoke.resolve()
        }
        main.post { stopRecognizer() }
        val id = UUID.randomUUID().toString()
        utterance = id
        engine.speak(args.line, TextToSpeech.QUEUE_FLUSH, Bundle(), id)
        invoke.resolve()
    }

    @Command
    fun stopSpeaking(invoke: Invoke) {
        utterance = null
        tts?.stop()
        invoke.resolve()
    }

    /** On silent or vibrate, or in Do Not Disturb: board events stay unspoken. */
    @Command
    fun quiet(invoke: Invoke) {
        val audio = activity.getSystemService(Context.AUDIO_SERVICE) as AudioManager
        val notifications = activity.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        val quiet = audio.ringerMode != AudioManager.RINGER_MODE_NORMAL ||
            notifications.currentInterruptionFilter != NotificationManager.INTERRUPTION_FILTER_ALL
        invoke.resolve(JSObject().apply { put("value", quiet) })
    }

    private fun fail(message: String, fatal: Boolean) {
        emit("failed") { put("message", message); put("fatal", fatal) }
    }

    private fun startRecognizer(wake: Boolean) {
        if (!onScreen) return
        val onDevice = wake || onDeviceAvailable()
        if (recognizer == null || recognizerOnDevice != onDevice) {
            recognizer?.destroy()
            recognizer = if (onDevice && Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                SpeechRecognizer.createOnDeviceSpeechRecognizer(activity)
            } else {
                SpeechRecognizer.createSpeechRecognizer(activity)
            }
            recognizerOnDevice = onDevice
            recognizer?.setRecognitionListener(listener)
        }
        val intent = Intent(RecognizerIntent.ACTION_RECOGNIZE_SPEECH).apply {
            putExtra(RecognizerIntent.EXTRA_LANGUAGE_MODEL, RecognizerIntent.LANGUAGE_MODEL_FREE_FORM)
            putExtra(RecognizerIntent.EXTRA_LANGUAGE, "en-GB")
            putExtra(RecognizerIntent.EXTRA_PARTIAL_RESULTS, true)
            if (onDevice) putExtra(RecognizerIntent.EXTRA_PREFER_OFFLINE, true)
        }
        // A tap during the wake loop finds the recogniser still listening:
        // without this it answers ERROR_RECOGNIZER_BUSY. The cancel's own
        // ERROR_CLIENT is ignored below.
        recognizer?.cancel()
        recognizer?.startListening(intent)
    }

    private fun stopRecognizer() {
        recognizer?.cancel()
    }

    private val listener = object : RecognitionListener {
        override fun onReadyForSpeech(params: Bundle?) {}
        override fun onBeginningOfSpeech() {}
        override fun onRmsChanged(rmsdB: Float) {}
        override fun onBufferReceived(buffer: ByteArray?) {}
        override fun onEndOfSpeech() {}
        override fun onEvent(eventType: Int, params: Bundle?) {}

        override fun onPartialResults(partialResults: Bundle?) {
            val text = partialResults?.getStringArrayList(SpeechRecognizer.RESULTS_RECOGNITION)?.firstOrNull() ?: return
            emit("partial") { put("text", text) }
        }

        override fun onResults(results: Bundle?) {
            val text = results?.getStringArrayList(SpeechRecognizer.RESULTS_RECOGNITION)?.firstOrNull()
            if (text.isNullOrBlank()) emit("ended")
            else emit("final") { put("text", text) }
        }

        override fun onError(error: Int) {
            when (error) {
                // Nothing said, or nothing understood: not a failure.
                SpeechRecognizer.ERROR_NO_MATCH, SpeechRecognizer.ERROR_SPEECH_TIMEOUT -> emit("ended")
                // Our own cancel (leaving the screen, speaking): say nothing.
                SpeechRecognizer.ERROR_CLIENT -> {}
                SpeechRecognizer.ERROR_INSUFFICIENT_PERMISSIONS -> fail("Microphone access is off.", true)
                SpeechRecognizer.ERROR_LANGUAGE_NOT_SUPPORTED, SpeechRecognizer.ERROR_LANGUAGE_UNAVAILABLE ->
                    fail("English speech recognition is not installed on this phone.", true)
                else -> fail(describe(error), false)
            }
        }
    }

    private fun describe(error: Int): String = when (error) {
        SpeechRecognizer.ERROR_RECOGNIZER_BUSY -> "the recogniser is busy"
        SpeechRecognizer.ERROR_NETWORK, SpeechRecognizer.ERROR_NETWORK_TIMEOUT -> "no network for speech recognition"
        SpeechRecognizer.ERROR_AUDIO -> "the microphone failed"
        SpeechRecognizer.ERROR_SERVER, SpeechRecognizer.ERROR_SERVER_DISCONNECTED -> "the speech service failed"
        SpeechRecognizer.ERROR_TOO_MANY_REQUESTS -> "too many requests"
        else -> "speech recognition failed ($error)"
    }

    override fun onDestroy() {
        main.post { ProcessLifecycleOwner.get().lifecycle.removeObserver(this) }
        recognizer?.destroy()
        tts?.shutdown()
        super.onDestroy()
    }
}
