package com.dosaki.maya.mobile

import android.content.Intent
import android.os.Bundle
import androidx.activity.enableEdgeToEdge

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
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
