package com.aimer

import android.app.Activity
import androidx.core.content.ContextCompat
import androidx.core.util.Consumer
import androidx.window.java.layout.WindowInfoTrackerCallbackAdapter
import androidx.window.layout.FoldingFeature
import androidx.window.layout.WindowInfoTracker
import androidx.window.layout.WindowLayoutInfo

/**
 * The Android half of `aimer_screen`'s folds and hinges.
 *
 * Jetpack WindowManager reports the display's [FoldingFeature]s through a Java
 * callback interface, which Rust cannot implement, so this object listens and
 * hands each report to Rust through [nativeDisplayFeatures]. The same API covers
 * a foldable phone's fold and the Microsoft Surface Duo's hinge: a hinge is a
 * feature that occludes the content beneath it.
 *
 * Nothing else lives here. Which areas a feature leaves usable is worked out in
 * Rust, from the bounds sent.
 *
 * Add `androidx.window:window` and `androidx.window:window-java` to the app's
 * dependencies, copy this file into the app's `com.aimer` package, and call
 * [attach] from the activity's `onCreate` (and [detach] from `onDestroy`).
 * Without them the application simply sees no display features.
 *
 * The wire form is `left,top,right,bottom,occludes,halfOpened,separating`
 * per feature, features separated by `;`, bounds in logical pixels (dp) from the
 * window's top-left. It must match `decode_display_features` in `aimer_screen`.
 */
object ScreenBridge {

    private var adapter: WindowInfoTrackerCallbackAdapter? = null
    private var listener: Consumer<WindowLayoutInfo>? = null

    /** Starts reporting [activity]'s display features. A second call replaces the first. */
    @JvmStatic
    fun attach(activity: Activity) {
        detach()
        val tracker = WindowInfoTrackerCallbackAdapter(WindowInfoTracker.getOrCreate(activity))
        val consumer = Consumer<WindowLayoutInfo> { info -> report(activity, info) }
        tracker.addWindowLayoutInfoListener(
            activity,
            ContextCompat.getMainExecutor(activity),
            consumer,
        )
        adapter = tracker
        listener = consumer
    }

    /** Stops reporting. Safe to call when nothing is attached. */
    @JvmStatic
    fun detach() {
        listener?.let { adapter?.removeWindowLayoutInfoListener(it) }
        adapter = null
        listener = null
    }

    private fun report(activity: Activity, info: WindowLayoutInfo) {
        // Bounds arrive in pixels; Rust works in logical pixels.
        val density = activity.resources.displayMetrics.density
        val encoded = info.displayFeatures
            .filterIsInstance<FoldingFeature>()
            .joinToString(";") { feature ->
                val bounds = feature.bounds
                listOf(
                    bounds.left / density,
                    bounds.top / density,
                    bounds.right / density,
                    bounds.bottom / density,
                    flag(feature.occlusionType == FoldingFeature.OcclusionType.FULL),
                    flag(feature.state == FoldingFeature.State.HALF_OPENED),
                    flag(feature.isSeparating),
                ).joinToString(",")
            }
        nativeDisplayFeatures(encoded)
    }

    private fun flag(value: Boolean): Int = if (value) 1 else 0

    /**
     * Reports the current display features to Rust, replacing the previous list.
     *
     * Called on the main thread; Rust only queues it for the event loop. An empty
     * string means the device is flat or has no folds.
     */
    @JvmStatic
    external fun nativeDisplayFeatures(encoded: String)
}
