package com.dosaki.maya.mobile

import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Color
import android.os.Build
import android.os.Bundle
import android.view.View
import android.webkit.WebView
import androidx.activity.SystemBarStyle
import androidx.activity.enableEdgeToEdge
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    // The page is dark, so the bars over it get light icons in every system theme.
    enableEdgeToEdge(SystemBarStyle.dark(Color.TRANSPARENT), SystemBarStyle.dark(Color.TRANSPARENT))
    super.onCreate(savedInstanceState)
    insetForSystemBars()
    requestLocalNetwork()
  }

  // Edge-to-edge lays the web view under the status bar and the gesture bar,
  // and the web view does not inset itself for them (its safe-area insets
  // only report a display cutout). Pad the content view by the bars and the
  // keyboard instead, so the page always sits between them; the strips they
  // cover show the theme's window background, the page's own colour.
  private fun insetForSystemBars() {
    val content = findViewById<android.view.View>(android.R.id.content)
    ViewCompat.setOnApplyWindowInsetsListener(content) { view, insets ->
      val bars = insets.getInsets(WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout() or WindowInsetsCompat.Type.ime())
      view.setPadding(bars.left, bars.top, bars.right, bars.bottom)
      WindowInsetsCompat.CONSUMED
    }
  }

  // The page has nothing to autofill, but the web view offers its text boxes
  // to the autofill service anyway: a password manager then puts up its
  // popup on the reply box, which takes the focus and closes the keyboard
  // mid-word. Keep the web view and everything in it out of autofill.
  override fun onWebViewCreate(webView: WebView) {
    webView.importantForAutofill = View.IMPORTANT_FOR_AUTOFILL_NO_EXCLUDE_DESCENDANTS
  }

  // Android 16 gates the local network behind a runtime permission. Without
  // it the OS drops the main's packets to the LAN while the internet still
  // works: the phone's connection to the main gets no reply and times out.
  // The grant takes effect at once, so the connection need not restart.
  private fun requestLocalNetwork() {
    if (Build.VERSION.SDK_INT < 36) return
    if (ContextCompat.checkSelfPermission(this, LOCAL_NETWORK) == PackageManager.PERMISSION_GRANTED) return
    ActivityCompat.requestPermissions(this, arrayOf(LOCAL_NETWORK), LOCAL_NETWORK_REQUEST)
  }

  // A notification tapped after Android killed the app brings the task
  // back with its old intent and hands the tap to onNewIntent, before the
  // plugins are registered. Keeping it as the activity's intent lets
  // KeepAlivePlugin read it in load().
  override fun onNewIntent(intent: Intent) {
    setIntent(intent)
    super.onNewIntent(intent)
  }
}

/** Named by string: the SDK constant only exists from API 36. */
private const val LOCAL_NETWORK = "android.permission.ACCESS_LOCAL_NETWORK"
private const val LOCAL_NETWORK_REQUEST = 4127
