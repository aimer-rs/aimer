package com.aimer

import android.app.Activity
import android.app.Application
import android.content.Context
import android.os.Build
import android.os.Bundle
import android.os.CancellationSignal
import android.os.Handler
import android.os.Looper
import androidx.core.content.ContextCompat
import androidx.credentials.CreateCredentialResponse
import androidx.credentials.CreatePublicKeyCredentialRequest
import androidx.credentials.CreatePublicKeyCredentialResponse
import androidx.credentials.CredentialManager
import androidx.credentials.CredentialManagerCallback
import androidx.credentials.GetCredentialRequest
import androidx.credentials.GetCredentialResponse
import androidx.credentials.GetPublicKeyCredentialOption
import androidx.credentials.PublicKeyCredential
import androidx.credentials.exceptions.CreateCredentialException
import androidx.credentials.exceptions.GetCredentialException
import androidx.credentials.exceptions.publickeycredential.CreatePublicKeyCredentialDomException
import androidx.credentials.exceptions.publickeycredential.GetPublicKeyCredentialDomException
import java.lang.ref.WeakReference
import java.util.concurrent.ConcurrentHashMap

/**
 * The Android half of `aimer_passkey`.
 *
 * Credential Manager reports through a Java callback interface, which Rust
 * cannot implement, so this object receives a ceremony from Rust, runs it, and
 * hands the outcome back through [nativeComplete]. A ceremony is identified by
 * the `token` Rust gave it; Rust keeps the future that is waiting on it.
 *
 * Nothing else about the ceremony lives here: requests and responses are the
 * WebAuthn JSON Credential Manager already speaks, and Rust builds and parses
 * them.
 *
 * Credential Manager shows its sheet over an [Activity], and the native side
 * cannot be given one: the Android context the Rust runtime holds is the
 * `Application`. So the bridge keeps track of the foreground activity itself.
 * Call [attach] from the activity's `onCreate` to make that certain; after that,
 * lifecycle callbacks keep it current as activities come and go.
 *
 * Add `androidx.credentials:credentials` and
 * `androidx.credentials:credentials-play-services-auth` (for devices before
 * Android 14) to the app's dependencies, and publish the app in the relying
 * party's `/.well-known/assetlinks.json`.
 */
object PasskeyBridge {

    /** In-flight ceremonies, so [cancel] can reach the system sheet. */
    private val signals = ConcurrentHashMap<Long, CancellationSignal>()

    private val main = Handler(Looper.getMainLooper())

    /** The activity ceremonies are presented over. Weak: the bridge must not keep a destroyed activity alive. */
    @Volatile
    private var current: WeakReference<Activity>? = null

    private var tracking = false

    /**
     * Makes [activity] the one ceremonies are presented over, and starts tracking
     * the foreground activity for as long as the process lives. Call it from
     * `onCreate`.
     */
    @JvmStatic
    fun attach(activity: Activity) {
        current = WeakReference(activity)
        track(activity.application)
    }

    /** Forgets [activity] if it is the current one. Optional: destruction is tracked anyway. */
    @JvmStatic
    fun detach(activity: Activity) {
        if (current?.get() === activity) current = null
    }

    /**
     * Called by Rust with the application context when the bridge is first used,
     * so activities that start later are tracked even if [attach] never ran.
     */
    @JvmStatic
    fun init(context: Context) {
        (context.applicationContext as? Application)?.let(::track)
    }

    @Synchronized
    private fun track(application: Application) {
        if (tracking) return
        tracking = true
        application.registerActivityLifecycleCallbacks(object : Application.ActivityLifecycleCallbacks {
            override fun onActivityResumed(activity: Activity) {
                current = WeakReference(activity)
            }

            override fun onActivityDestroyed(activity: Activity) = detach(activity)
            override fun onActivityCreated(activity: Activity, savedInstanceState: Bundle?) {}
            override fun onActivityStarted(activity: Activity) {}
            override fun onActivityPaused(activity: Activity) {}
            override fun onActivityStopped(activity: Activity) {}
            override fun onActivitySaveInstanceState(activity: Activity, outState: Bundle) {}
        })
    }

    private fun foreground(): Activity? = current?.get()?.takeUnless { it.isFinishing || it.isDestroyed }

    /** Reports an outcome to Rust. `status` is [OK] or an error kind; see below. */
    @JvmStatic
    external fun nativeComplete(token: Long, status: String, payload: String)

    /** Whether this device can create and use passkeys at all. */
    @JvmStatic
    fun isAvailable(): Boolean = Build.VERSION.SDK_INT >= Build.VERSION_CODES.P

    /**
     * Creates a passkey from the JSON creation options.
     *
     * Completes with [OK] and the `RegistrationResponseJSON`, or with the failing
     * exception's kind and message.
     */
    @JvmStatic
    fun create(token: Long, requestJson: String) {
        main.post {
            val activity = foreground() ?: return@post finish(token, NO_ACTIVITY, "")
            val signal = CancellationSignal()
            signals[token] = signal
            try {
                CredentialManager.create(activity).createCredentialAsync(
                    activity,
                    CreatePublicKeyCredentialRequest(requestJson),
                    signal,
                    ContextCompat.getMainExecutor(activity),
                    object : CredentialManagerCallback<CreateCredentialResponse, CreateCredentialException> {
                        override fun onResult(result: CreateCredentialResponse) {
                            val json = (result as? CreatePublicKeyCredentialResponse)?.registrationResponseJson
                            if (json != null) finish(token, OK, json)
                            else finish(token, "UnexpectedResponse", result.javaClass.name)
                        }

                        override fun onError(e: CreateCredentialException) {
                            finish(token, kindOf(e), e.errorMessage?.toString() ?: "")
                        }
                    },
                )
            } catch (e: Exception) {
                finish(token, e.javaClass.simpleName, e.message ?: "")
            }
        }
    }

    /**
     * Signs in with a passkey from the JSON request options.
     *
     * Completes with [OK] and the `AuthenticationResponseJSON`, or with the
     * failing exception's kind and message.
     */
    @JvmStatic
    fun get(token: Long, requestJson: String) {
        main.post {
            val activity = foreground() ?: return@post finish(token, NO_ACTIVITY, "")
            val signal = CancellationSignal()
            signals[token] = signal
            try {
                CredentialManager.create(activity).getCredentialAsync(
                    activity,
                    GetCredentialRequest(listOf(GetPublicKeyCredentialOption(requestJson))),
                    signal,
                    ContextCompat.getMainExecutor(activity),
                    object : CredentialManagerCallback<GetCredentialResponse, GetCredentialException> {
                        override fun onResult(result: GetCredentialResponse) {
                            val credential = result.credential
                            if (credential is PublicKeyCredential) {
                                finish(token, OK, credential.authenticationResponseJson)
                            } else {
                                finish(token, "UnexpectedResponse", credential.javaClass.name)
                            }
                        }

                        override fun onError(e: GetCredentialException) {
                            finish(token, kindOf(e), e.errorMessage?.toString() ?: "")
                        }
                    },
                )
            } catch (e: Exception) {
                finish(token, e.javaClass.simpleName, e.message ?: "")
            }
        }
    }

    /** Dismisses the system sheet of the ceremony `token`, if it is still up. */
    @JvmStatic
    fun cancel(token: Long) {
        signals.remove(token)?.cancel()
    }

    private fun finish(token: Long, status: String, payload: String) {
        signals.remove(token)
        nativeComplete(token, status, payload)
    }

    /**
     * The error kind Rust maps to a typed error: the exception's simple class
     * name, or `Dom.` and the DOM error's simple class name for the exceptions
     * that wrap one (a refused ceremony, a duplicate credential, a relying party
     * the app is not linked to).
     */
    private fun kindOf(e: Exception): String = when (e) {
        is CreatePublicKeyCredentialDomException -> "Dom." + e.domError.javaClass.simpleName
        is GetPublicKeyCredentialDomException -> "Dom." + e.domError.javaClass.simpleName
        else -> e.javaClass.simpleName
    }

    private const val OK = "ok"

    /** No activity is in the foreground to present the sheet over. */
    private const val NO_ACTIVITY = "NoActivity"
}
