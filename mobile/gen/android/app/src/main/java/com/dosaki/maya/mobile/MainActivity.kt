package com.dosaki.maya.mobile

import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import androidx.activity.enableEdgeToEdge
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    requestLocalNetwork()
  }

  // Android 16 gates the local network behind a runtime permission. Without
  // it the OS drops the main's packets to the LAN while the internet still
  // works: an assistant's pairing connection gets no reply and times out.
  // The grant takes effect at once, so the server need not restart.
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
