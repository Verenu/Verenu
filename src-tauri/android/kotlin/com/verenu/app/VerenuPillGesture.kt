package com.verenu.app

class VerenuPillGesture {
    enum class Event { NONE, HOLD_START, HOLD_END, DRAG_START, DRAG_END }

    var holding = false
        private set
    var dragging = false
        private set
    var suppressClick = false
        private set
    private var pressed = false

    fun down() {
        pressed = true
        suppressClick = false
    }

    fun consumeClickSuppression(): Boolean {
        val suppressed = suppressClick
        suppressClick = false
        return suppressed
    }

    fun longPress(idle: Boolean, recording: Boolean): Event {
        if (!pressed || holding || dragging) return Event.NONE
        return when {
            idle -> { holding = true; suppressClick = true; Event.HOLD_START }
            recording -> { dragging = true; suppressClick = true; Event.DRAG_START }
            else -> Event.NONE
        }
    }

    fun movedPastSlop(idle: Boolean): Event {
        if (!pressed || holding || dragging || !idle) return Event.NONE
        dragging = true
        suppressClick = true
        return Event.DRAG_START
    }

    fun up(cancelled: Boolean = false): List<Event> {
        pressed = false
        if (cancelled) suppressClick = false
        val out = ArrayList<Event>(2)
        if (holding) { holding = false; out += Event.HOLD_END }
        if (dragging) { dragging = false; out += Event.DRAG_END }
        return out
    }
}

class VerenuHoldRelease {
    private var startedByHold = false
    private var stopWhenRecording = false
    var startInFlight = false
        private set

    fun holdStart(idle: Boolean): Boolean {
        if (startInFlight) return false
        stopWhenRecording = false
        startedByHold = idle
        return startedByHold
    }

    fun beginStart(): Boolean {
        if (startInFlight) return false
        startInFlight = true
        return true
    }

    fun startFinished(recording: Boolean): Boolean {
        startInFlight = false
        val stop = recording && stopWhenRecording
        stopWhenRecording = false
        if (!recording) startedByHold = false
        return stop
    }

    fun holdEnd(recording: Boolean): Boolean {
        if (!startedByHold) return false
        startedByHold = false
        if (recording) return true
        if (startInFlight) stopWhenRecording = true
        return false
    }
}
