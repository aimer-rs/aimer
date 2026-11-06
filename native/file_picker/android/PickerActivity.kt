package com.aimer

import android.app.Activity
import android.content.Intent
import android.os.Bundle

/**
 * A transparent activity that runs one system picker for [FilePickerBridge].
 *
 * `startActivityForResult` and `onActivityResult` belong to an `Activity`, and
 * the application's own activity is not ours to subclass. This one has no UI: it
 * launches the picker the bridge prepared, waits for the answer, hands it over
 * and finishes. Declare it in the manifest; see [FilePickerBridge].
 */
class PickerActivity : Activity() {

    private var token = 0L

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        token = intent.getLongExtra(TOKEN, 0L)
        val request = FilePickerBridge.requests[token]
        if (request == null) {
            finish()
            return
        }
        request.activity = this
        // After a configuration change or process death the picker is already
        // running and its result is coming; launching again would stack a second.
        if (savedInstanceState != null) return
        try {
            startActivityForResult(request.intent, PICK)
        } catch (e: Exception) {
            FilePickerBridge.finish(token, e.javaClass.simpleName, e.message ?: "")
            finish()
        }
    }

    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode == PICK) FilePickerBridge.deliver(token, this, resultCode, data)
        finish()
    }

    /** Closes the picker this activity launched, then this activity. */
    internal fun dismiss() {
        finishActivity(PICK)
        finish()
    }

    companion object {
        const val TOKEN = "token"
        private const val PICK = 0x4152
    }
}
