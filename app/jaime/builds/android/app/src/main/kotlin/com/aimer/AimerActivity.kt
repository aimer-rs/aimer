package com.aimer

import android.app.NativeActivity
import android.content.Context
import android.os.Bundle
import android.text.Editable
import android.text.InputType
import android.text.TextWatcher
import android.text.method.PasswordTransformationMethod
import android.view.ViewGroup
import android.view.inputmethod.BaseInputConnection
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputConnection
import android.view.inputmethod.InputConnectionWrapper
import android.view.inputmethod.InputMethodManager
import android.widget.EditText

/**
 * Thin wrapper around [NativeActivity] that gives the framework a working
 * software keyboard with full IME support (Chinese / Japanese / Korean, emoji,
 * autocorrect, ...).
 *
 * A bare `NativeActivity` renders into a native surface that has no
 * [android.view.inputmethod.InputConnection], so the system IME has nowhere to
 * deliver composed text and CJK input is silently dropped. To fix this we add a
 * hidden, focusable [EditText] on top of the native surface and mirror the
 * focused Rust field into it. Text, selection, and composing changes are sent
 * back as revisioned deltas, which lets replacement, cursor movement, and
 * multistage input methods use Android's native editing behavior.
 *
 * Rust calls [showKeyboard] / [hideKeyboard] via JNI when a text field gains or
 * loses focus.
 */
class AimerActivity : NativeActivity() {

    private var inputView: MirroredEditText? = null
    private var inputConnection: InputConnection? = null
    private var sessionId = 0L
    private var revision = 0L
    private var suppressCallbacks = false
    private var pendingReplaceStart = 0
    private var pendingReplaceEnd = 0
    private var pendingReplacementLength = 0
    private var lastSelectionStart = -1
    private var lastSelectionEnd = -1
    private var lastComposingStart = -1
    private var lastComposingEnd = -1
    private var secureEntry = false
    private var inputKind = INPUT_KIND_TEXT

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        runOnUiThread(::setupInputView)
    }

    private fun setupInputView() {
        val view = MirroredEditText(this).apply {
            // Effectively invisible but still focusable so it can own the IME session.
            alpha = 0f
            isFocusable = true
            isFocusableInTouchMode = true
            inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS
            imeOptions = EditorInfo.IME_FLAG_NO_EXTRACT_UI
        }

        view.addTextChangedListener(object : TextWatcher {
            override fun beforeTextChanged(s: CharSequence?, start: Int, count: Int, after: Int) {
                pendingReplaceStart = start
                pendingReplaceEnd = start + count
            }

            override fun onTextChanged(s: CharSequence?, start: Int, before: Int, count: Int) {
                pendingReplacementLength = count
            }

            override fun afterTextChanged(s: Editable) {
                if (suppressCallbacks || sessionId == 0L) {
                    return
                }
                val replacementStart = pendingReplaceStart.coerceIn(0, s.length)
                val replacementEnd =
                    (replacementStart + pendingReplacementLength).coerceIn(replacementStart, s.length)
                reportDelta(
                    pendingReplaceStart,
                    pendingReplaceEnd,
                    s.subSequence(replacementStart, replacementEnd).toString(),
                )
            }
        })

        addContentView(view, ViewGroup.LayoutParams(1, 1))
        inputView = view
    }

    private fun reportDelta(replaceStart: Int, replaceEnd: Int, replacementText: String) {
        val view = inputView ?: return
        if (suppressCallbacks || sessionId == 0L) {
            return
        }
        val textLength = view.text.length
        val selectionStart = view.selectionStart.coerceIn(0, textLength)
        val selectionEnd = view.selectionEnd.coerceIn(0, textLength)
        val composingStart = composingStart(view.text)
        val composingEnd = composingEnd(view.text)
        nativeTextEditingDelta(
            sessionId,
            revision,
            replaceStart,
            replaceEnd,
            replacementText,
            selectionStart,
            selectionEnd,
            composingStart,
            composingEnd,
        )
        revision += 1
        rememberEditorState(selectionStart, selectionEnd, composingStart, composingEnd)
    }

    private fun reportSelectionOrComposingChange() {
        val view = inputView ?: return
        if (suppressCallbacks || sessionId == 0L) {
            return
        }
        val textLength = view.text.length
        val selectionStart = view.selectionStart.coerceIn(0, textLength)
        val selectionEnd = view.selectionEnd.coerceIn(0, textLength)
        val composingStart = composingStart(view.text)
        val composingEnd = composingEnd(view.text)
        if (
            selectionStart == lastSelectionStart &&
                selectionEnd == lastSelectionEnd &&
                composingStart == lastComposingStart &&
                composingEnd == lastComposingEnd
        ) {
            return
        }
        reportDelta(selectionStart, selectionStart, "")
    }

    private fun composingStart(editable: Editable): Int {
        val start = BaseInputConnection.getComposingSpanStart(editable)
        val end = BaseInputConnection.getComposingSpanEnd(editable)
        return if (start >= 0 && end >= start && end <= editable.length) start else -1
    }

    private fun composingEnd(editable: Editable): Int {
        val start = BaseInputConnection.getComposingSpanStart(editable)
        val end = BaseInputConnection.getComposingSpanEnd(editable)
        return if (start >= 0 && end >= start && end <= editable.length) end else -1
    }

    private fun rememberEditorState(
        selectionStart: Int,
        selectionEnd: Int,
        composingStart: Int,
        composingEnd: Int,
    ) {
        lastSelectionStart = selectionStart
        lastSelectionEnd = selectionEnd
        lastComposingStart = composingStart
        lastComposingEnd = composingEnd
    }

    /** Replaces the hidden editor with the authoritative Rust snapshot. */
    @Suppress("unused")
    fun syncTextState(
        sessionId: Long,
        revision: Long,
        text: String,
        selectionStart: Int,
        selectionEnd: Int,
        composingStart: Int,
        composingEnd: Int,
        secure: Boolean,
        inputKind: Int,
    ) {
        runOnUiThread {
            val view = inputView ?: return@runOnUiThread
            if (this.sessionId == sessionId && revision < this.revision) {
                return@runOnUiThread
            }
            this.sessionId = sessionId
            this.revision = revision
            suppressCallbacks = true
            try {
                val shouldRestartInput = configureInput(view, secure, inputKind)
                if (view.text.toString() != text) {
                    view.setText(text)
                }
                val editable = view.text
                BaseInputConnection.removeComposingSpans(editable)
                if (
                    composingStart >= 0 &&
                        composingEnd >= composingStart &&
                        composingEnd <= editable.length
                ) {
                    val connection =
                        inputConnection ?: view.onCreateInputConnection(EditorInfo())
                    connection?.setComposingRegion(composingStart, composingEnd)
                }
                val clampedSelectionStart = selectionStart.coerceIn(0, editable.length)
                val clampedSelectionEnd = selectionEnd.coerceIn(0, editable.length)
                view.setSelection(clampedSelectionStart, clampedSelectionEnd)
                rememberEditorState(
                    clampedSelectionStart,
                    clampedSelectionEnd,
                    composingStart(editable),
                    composingEnd(editable),
                )
                if (shouldRestartInput) {
                    inputConnection = null
                    val imm =
                        getSystemService(Context.INPUT_METHOD_SERVICE) as? InputMethodManager
                    imm?.restartInput(view)
                }
            } finally {
                suppressCallbacks = false
            }
        }
    }

    private fun configureInput(view: EditText, secure: Boolean, inputKind: Int): Boolean {
        val nextSecureEntry = secure || inputKind == INPUT_KIND_OBSCURE
        if (secureEntry == nextSecureEntry && this.inputKind == inputKind) {
            return false
        }
        secureEntry = nextSecureEntry
        this.inputKind = inputKind
        view.inputType = when {
            nextSecureEntry ->
                InputType.TYPE_CLASS_TEXT or
                    InputType.TYPE_TEXT_VARIATION_PASSWORD or
                    InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS
            inputKind == INPUT_KIND_NUMBER ->
                InputType.TYPE_CLASS_NUMBER or
                    InputType.TYPE_NUMBER_FLAG_DECIMAL or
                    InputType.TYPE_NUMBER_FLAG_SIGNED
            else -> InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS
        }
        view.transformationMethod =
            if (nextSecureEntry) PasswordTransformationMethod.getInstance() else null
        return true
    }

    /** Called from Rust via JNI when a text field gains focus. */
    @Suppress("unused")
    fun showKeyboard() {
        runOnUiThread {
            val view = inputView ?: return@runOnUiThread
            view.requestFocus()
            val imm = getSystemService(Context.INPUT_METHOD_SERVICE) as? InputMethodManager
            // No flags: `SHOW_IMPLICIT` is deprecated and, since Android 13, is
            // ignored outright by some IMEs. The view was just focused above, so
            // an explicit request is exactly what is wanted here.
            imm?.showSoftInput(view, 0)
        }
    }

    /** Called from Rust via JNI when the focused text field is dismissed. */
    @Suppress("unused")
    fun hideKeyboard() {
        runOnUiThread {
            val view = inputView ?: return@runOnUiThread
            val imm = getSystemService(Context.INPUT_METHOD_SERVICE) as? InputMethodManager
            imm?.hideSoftInputFromWindow(view.windowToken, 0)
            sessionId = 0L
        }
    }

    private inner class MirroredEditText(context: Context) : EditText(context) {
        override fun onSelectionChanged(selectionStart: Int, selectionEnd: Int) {
            super.onSelectionChanged(selectionStart, selectionEnd)
            if (!suppressCallbacks) {
                post { reportSelectionOrComposingChange() }
            }
        }

        override fun onCreateInputConnection(outAttrs: EditorInfo): InputConnection? {
            val connection = super.onCreateInputConnection(outAttrs) ?: return null
            val wrapped = object : InputConnectionWrapper(connection, false) {
                override fun setComposingText(text: CharSequence?, newCursorPosition: Int): Boolean {
                    val changed = super.setComposingText(text, newCursorPosition)
                    this@MirroredEditText.post { reportSelectionOrComposingChange() }
                    return changed
                }

                override fun commitText(text: CharSequence?, newCursorPosition: Int): Boolean {
                    val changed = super.commitText(text, newCursorPosition)
                    this@MirroredEditText.post { reportSelectionOrComposingChange() }
                    return changed
                }

                override fun setComposingRegion(start: Int, end: Int): Boolean {
                    val changed = super.setComposingRegion(start, end)
                    this@MirroredEditText.post { reportSelectionOrComposingChange() }
                    return changed
                }

                override fun finishComposingText(): Boolean {
                    val changed = super.finishComposingText()
                    this@MirroredEditText.post { reportSelectionOrComposingChange() }
                    return changed
                }
            }
            inputConnection = wrapped
            return wrapped
        }
    }

    companion object {
        private const val INPUT_KIND_TEXT = 0
        private const val INPUT_KIND_NUMBER = 1
        private const val INPUT_KIND_OBSCURE = 2

        /**
         * Reports a native editing transaction to Rust.
         *
         * `@JvmStatic` is required: the Rust symbol is bound to a *static* native
         * method on `com.aimer.AimerActivity`, which a plain companion member
         * would not produce.
         */
        @JvmStatic
        external fun nativeTextEditingDelta(
            sessionId: Long,
            revision: Long,
            replaceStart: Int,
            replaceEnd: Int,
            replacementText: String,
            selectionStart: Int,
            selectionEnd: Int,
            composingStart: Int,
            composingEnd: Int,
        )
    }
}
