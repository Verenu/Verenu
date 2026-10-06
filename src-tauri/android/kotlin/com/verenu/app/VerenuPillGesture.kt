package com.verenu.app

/**
 * Touch decisions for the idle/recording pill, kept free of Android types so
 * they can be unit-tested. [VerenuOverlayView] feeds it events and acts on the
 * returned [Event]s.
 *
 * - Idle: hold still → push-to-talk ([Event.HOLD_START] … [Event.HOLD_END]);
 *   move past touch slop first → drag.
 * - Recording: hold → drag; tap is handled by the view's click listener.
 */
class VerenuPillGesture {
    enum class Event { NONE, HOLD_START, HOLD_END, DRAG_START, DRAG_END }

    var holding = false
        private set
    var dragging = false
        private set
    /** True once a hold or drag began, so the finger lifting is not a tap. */
    var suppressClick = false
        private set
    private var pressed = false

    fun down() {
        pressed = true
        suppressClick = false
    }

    /** The long-press timeout elapsed while the finger is still down and in place. */
    fun longPress(idle: Boolean, recording: Boolean): Event {
        if (!pressed || holding || dragging) return Event.NONE
        return when {
            idle -> { holding = true; suppressClick = true; Event.HOLD_START }
            recording -> { dragging = true; suppressClick = true; Event.DRAG_START }
            else -> Event.NONE
        }
    }

    /** The finger moved past touch slop before the long press fired. */
    fun movedPastSlop(idle: Boolean): Event {
        if (!pressed || holding || dragging || !idle) return Event.NONE
        dragging = true
        suppressClick = true
        return Event.DRAG_START
    }

    /** Finger lifted or the touch was cancelled. Both end a hold and a drag. */
    fun up(): List<Event> {
        pressed = false
        val out = ArrayList<Event>(2)
        if (holding) { holding = false; out += Event.HOLD_END }
        if (dragging) { dragging = false; out += Event.DRAG_END }
        return out
    }
}

/**
 * Decides when a pending hold-to-dictate release must stop the recording.
 * Starting the recording is asynchronous, so a release can arrive first; that
 * stop is remembered and applied once the recording is up, and discarded only
 * when the start fails or a new hold begins.
 */
class VerenuHoldRelease {
    private var startedByHold = false
    private var stopWhenRecording = false
    var startInFlight = false
        private set

    /** A hold began; returns whether it should start a dictation. */
    fun holdStart(idle: Boolean): Boolean {
        stopWhenRecording = false
        startedByHold = idle && !startInFlight
        return startedByHold
    }

    /** A dictation start was requested; false if one is already pending. */
    fun beginStart(): Boolean {
        if (startInFlight) return false
        startInFlight = true
        return true
    }

    /** The start finished. Returns true if a queued release should stop it now. */
    fun startFinished(recording: Boolean): Boolean {
        startInFlight = false
        val stop = recording && stopWhenRecording
        stopWhenRecording = false
        if (!recording) startedByHold = false
        return stop
    }

    /** The hold was released; returns true if the recording should stop immediately. */
    fun holdEnd(recording: Boolean): Boolean {
        if (!startedByHold) return false
        startedByHold = false
        if (recording) return true
        if (startInFlight) stopWhenRecording = true
        return false
    }
}
