package com.aimer

import android.app.Activity
import android.app.Application
import android.content.ClipData
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.provider.MediaStore
import android.provider.OpenableColumns
import java.io.IOException
import java.lang.ref.WeakReference
import java.util.concurrent.ConcurrentHashMap

/**
 * The Android half of `aimer_file_picker`.
 *
 * `startActivityForResult` answers an `Activity`, which Rust cannot be, so a
 * request is run by [PickerActivity]: a transparent activity this bridge starts,
 * which launches the system picker, receives its result and hands it back through
 * [nativeComplete]. A request is identified by the `token` Rust gave it; Rust
 * keeps the future that is waiting on it.
 *
 * Add the activity to the app's manifest:
 *
 * ```xml
 * <activity
 *     android:name="com.aimer.PickerActivity"
 *     android:exported="false"
 *     android:excludeFromRecents="true"
 *     android:theme="@android:style/Theme.Translucent.NoTitleBar" />
 * ```
 *
 * Nothing needs to be called from the app's own activity: the bridge tracks the
 * foreground activity itself, from the `Application` the Rust runtime hands it.
 *
 * What the user picks arrives as `content://` URIs, which the app may read until
 * its process ends. [read] reads one.
 */
object FilePickerBridge {

    /** Requests that have not been answered yet, by token. */
    internal val requests = ConcurrentHashMap<Long, Request>()

    private val main = Handler(Looper.getMainLooper())

    @Volatile
    private var application: Application? = null

    /** The foreground activity, so the picker opens in the app's own task. Weak: it must not keep a destroyed activity alive. */
    @Volatile
    private var current: WeakReference<Activity>? = null

    private var tracking = false

    internal class Request(val intent: Intent, val multiple: Boolean) {
        /** The running [PickerActivity], so [cancel] can dismiss the picker. */
        @Volatile
        var activity: PickerActivity? = null
    }

    /** Called by Rust with the application context when the bridge is first used. */
    @JvmStatic
    fun init(context: Context) {
        val app = context.applicationContext as? Application ?: return
        application = app
        track(app)
    }

    @Synchronized
    private fun track(app: Application) {
        if (tracking) return
        tracking = true
        app.registerActivityLifecycleCallbacks(object : Application.ActivityLifecycleCallbacks {
            override fun onActivityResumed(activity: Activity) {
                if (activity !is PickerActivity) current = WeakReference(activity)
            }

            override fun onActivityDestroyed(activity: Activity) {
                if (current?.get() === activity) current = null
            }

            override fun onActivityCreated(activity: Activity, savedInstanceState: Bundle?) {}
            override fun onActivityStarted(activity: Activity) {}
            override fun onActivityPaused(activity: Activity) {}
            override fun onActivityStopped(activity: Activity) {}
            override fun onActivitySaveInstanceState(activity: Activity, outState: Bundle) {}
        })
    }

    /** Reports an outcome to Rust: `status` is [OK] or an error kind. */
    @JvmStatic
    external fun nativeComplete(token: Long, status: String, payload: String)

    /**
     * Shows a picker.
     *
     * [mimeTypes] is a comma-separated list of MIME types and wildcards, empty
     * for any file. [media] asks for the system photo picker, which only
     * [mimeTypes] made of `image/*` and `video/*` may use. [limit] is the most
     * files the photo picker lets the user choose, or 0 for no cap; the document
     * browser has no cap, so Rust truncates.
     *
     * Completes with [OK] and one record per file, or an error kind.
     */
    @JvmStatic
    fun pick(token: Long, mimeTypes: String, multiple: Boolean, media: Boolean, limit: Int) {
        main.post {
            val app = application ?: return@post nativeComplete(token, "NoContext", "")
            val types = if (mimeTypes.isEmpty()) emptyList() else mimeTypes.split(',')
            val intent = if (media && Build.VERSION.SDK_INT >= 33) {
                photoPicker(types, multiple, limit)
            } else {
                documentBrowser(types, multiple)
            }
            requests[token] = Request(intent, multiple)

            val launcher = Intent(app, PickerActivity::class.java).putExtra(PickerActivity.TOKEN, token)
            val host = current?.get()?.takeUnless { it.isFinishing || it.isDestroyed }
            try {
                if (host != null) {
                    host.startActivity(launcher)
                } else {
                    app.startActivity(launcher.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
                }
            } catch (e: android.content.ActivityNotFoundException) {
                finish(token, "PickerActivityMissing", "declare com.aimer.PickerActivity in AndroidManifest.xml")
            } catch (e: Exception) {
                finish(token, e.javaClass.simpleName, e.message ?: "")
            }
        }
    }

    private fun photoPicker(types: List<String>, multiple: Boolean, limit: Int): Intent {
        val intent = Intent(MediaStore.ACTION_PICK_IMAGES)
        // The photo picker takes one family; with both, it shows both by default.
        if (types.size == 1) intent.type = types[0]
        if (multiple) {
            val ceiling = MediaStore.getPickImagesMaxLimit()
            val max = if (limit > 0) minOf(limit, ceiling) else ceiling
            // EXTRA_PICK_IMAGES_MAX must be greater than one.
            if (max > 1) intent.putExtra(MediaStore.EXTRA_PICK_IMAGES_MAX, max)
        }
        return intent
    }

    private fun documentBrowser(types: List<String>, multiple: Boolean): Intent {
        val intent = Intent(Intent.ACTION_OPEN_DOCUMENT)
            .addCategory(Intent.CATEGORY_OPENABLE)
            .putExtra(Intent.EXTRA_ALLOW_MULTIPLE, multiple)
        when (types.size) {
            0 -> intent.type = "*/*"
            1 -> intent.type = types[0]
            else -> {
                intent.type = "*/*"
                intent.putExtra(Intent.EXTRA_MIME_TYPES, types.toTypedArray())
            }
        }
        return intent
    }

    /** Dismisses the picker of the request `token`, if it is still up. */
    @JvmStatic
    fun cancel(token: Long) {
        val request = requests.remove(token) ?: return
        main.post { request.activity?.dismiss() }
    }

    /** Turns the picker's result into a reply for Rust. Called by [PickerActivity]. */
    internal fun deliver(token: Long, context: Context, resultCode: Int, data: Intent?) {
        if (requests.remove(token) == null) return // cancelled: nobody is waiting
        if (resultCode != Activity.RESULT_OK || data == null) return nativeComplete(token, OK, "")
        try {
            val uris = ArrayList<Uri>()
            val clip: ClipData? = data.clipData
            if (clip != null) {
                for (i in 0 until clip.itemCount) clip.getItemAt(i).uri?.let(uris::add)
            } else {
                data.data?.let(uris::add)
            }
            val records = uris.joinToString(RECORD.toString()) { describe(context, it) }
            nativeComplete(token, OK, records)
        } catch (e: Exception) {
            nativeComplete(token, e.javaClass.simpleName, e.message ?: "")
        }
    }

    /** `uri`, name, MIME type and size of a picked document, as one record. */
    private fun describe(context: Context, uri: Uri): String {
        var name = uri.lastPathSegment ?: ""
        var size = -1L
        try {
            context.contentResolver.query(uri, null, null, null, null)?.use { cursor ->
                if (cursor.moveToFirst()) {
                    val nameColumn = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME)
                    if (nameColumn >= 0 && !cursor.isNull(nameColumn)) name = cursor.getString(nameColumn)
                    val sizeColumn = cursor.getColumnIndex(OpenableColumns.SIZE)
                    if (sizeColumn >= 0 && !cursor.isNull(sizeColumn)) size = cursor.getLong(sizeColumn)
                }
            }
        } catch (e: Exception) {
            // A provider that refuses the query still gave us a readable URI.
        }
        val mime = context.contentResolver.getType(uri) ?: ""
        return listOf(uri.toString(), name, mime, size.toString()).joinToString(FIELD.toString())
    }

    /** Reads everything a picked `content://` URI holds. Throws [IOException] if it cannot be opened. */
    @JvmStatic
    @Throws(IOException::class)
    fun read(uri: String): ByteArray {
        val resolver = application?.contentResolver ?: throw IOException("the bridge has no context")
        val stream = resolver.openInputStream(Uri.parse(uri)) ?: throw IOException("no content provider for $uri")
        return stream.use { it.readBytes() }
    }

    internal fun finish(token: Long, status: String, payload: String) {
        if (requests.remove(token) != null) nativeComplete(token, status, payload)
    }

    private const val OK = "ok"
    private const val FIELD = '\u001f'
    private const val RECORD = '\u001e'
}
