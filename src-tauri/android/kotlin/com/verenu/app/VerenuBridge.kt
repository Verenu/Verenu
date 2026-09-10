package com.verenu.app

import android.content.Context
import android.util.Log
import org.json.JSONObject
import java.io.File
import java.net.HttpURLConnection
import java.net.URL

/**
 * HTTP client for Verenu's loopback bridge (see `src-tauri/src/android/bridge.rs`,
 * which is the protocol's source of truth — this file mirrors it).
 *
 * The Rust backend listens on 127.0.0.1 with a per-boot token published in
 * `android_bridge.json` inside the app-private files dir (same UID, MODE_PRIVATE
 * storage). Every request carries `Authorization: Bearer <token>`.
 *
 * Only Android SDK + org.json APIs are used here — no Tauri, no OkHttp, no
 * coroutines — so this compiles against any AGP with `minSdk 26`.
 */
data class BridgeConnection(val port: Int, val token: String)

data class BridgeOverlay(
    val state: String,
    val visible: Boolean,
    val dictationActive: Boolean,
)

data class BridgePendingInsertion(
    val seq: Long,
    val text: String,
    val ageMs: Long,
)

data class BridgeErrorSnapshot(
    val message: String,
    val ageMs: Long,
)

data class BridgeStateSnapshot(
    val lifecycle: String,
    val dictationActive: Boolean,
    val pillStage: String,
    val audioLevel: Float,
    val keystorePending: Boolean,
    val lastError: BridgeErrorSnapshot?,
    val overlay: BridgeOverlay,
    val pendingInsertion: BridgePendingInsertion?,
    val targetPackage: String,
)

class VerenuBridge(appContext: Context) {
    private val appContext: Context = appContext.applicationContext

    companion object {
        const val TAG = "VerenuBridge"
        const val TIMEOUT_MS = 4000
    }

    @Volatile private var cached: BridgeConnection? = null

    /** Forget the cached connection (token rotates every boot). */
    fun invalidate() {
        cached = null
    }

    fun connection(): BridgeConnection? {
        cached?.let { return it }
        val candidates = listOf(
            // Tauri's Android PathPlugin resolves app_data_dir() to the
            // private application data root (Context.dataDir), not files/.
            File(appContext.dataDir, "android_bridge.json"),
            File(appContext.filesDir, "android_bridge.json"),
            File(appContext.dataDir, "files/android_bridge.json"),
        )
        for (file in candidates) {
            try {
                if (!file.exists()) continue
                val json = JSONObject(file.readText())
                val conn = BridgeConnection(json.getInt("port"), json.getString("token"))
                cached = conn
                return conn
            } catch (e: Exception) {
                Log.w(TAG, "cannot read bridge connection file ${file.path}", e)
            }
        }
        return null
    }

    private fun request(method: String, path: String, body: JSONObject? = null): JSONObject? {
        val conn = connection() ?: return null
        var urlConn: HttpURLConnection? = null
        return try {
            urlConn = URL("http://127.0.0.1:${conn.port}$path").openConnection() as HttpURLConnection
            urlConn.requestMethod = method
            urlConn.connectTimeout = TIMEOUT_MS
            urlConn.readTimeout = TIMEOUT_MS
            // The WebView never calls this client; CORS is irrelevant, but a
            // permissive header costs nothing if the policy ever changes.
            urlConn.setRequestProperty("Authorization", "Bearer ${conn.token}")
            if (body != null) {
                val bytes = body.toString().toByteArray(Charsets.UTF_8)
                urlConn.doOutput = true
                urlConn.setRequestProperty("Content-Type", "application/json")
                urlConn.setFixedLengthStreamingMode(bytes.size)
                urlConn.outputStream.use { it.write(bytes) }
            }
            val code = urlConn.responseCode
            if (code == 401 || code == 403) {
                // Token rotated (backend restarted): drop the cache so the
                // next call re-reads the connection file.
                invalidate()
                return null
            }
            val stream = if (code in 200..299) urlConn.inputStream else urlConn.errorStream
            val text = stream?.bufferedReader()?.readText() ?: "{}"
            JSONObject(text)
        } catch (e: Exception) {
            // The accessibility service can start before Rust republishes the
            // current boot's port, leaving a stale connection cached from a
            // previous process. Re-read the private connection file on the
            // next request instead of retrying a dead port forever.
            invalidate()
            Log.w(TAG, "$method $path failed", e)
            null
        } finally {
            urlConn?.disconnect()
        }
    }

    fun getState(): BridgeStateSnapshot? {
        val json = request("GET", "/v1/state") ?: return null
        if (!json.optBoolean("ok")) return null
        val overlay = json.optJSONObject("overlay")
        val pending = json.optJSONObject("pendingInsertion")
        return BridgeStateSnapshot(
            lifecycle = json.optString("lifecycle", "unknown"),
            dictationActive = json.optBoolean("dictationActive", false),
            pillStage = json.optString("pillStage", ""),
            audioLevel = json.optDouble("audioLevel", 0.0).toFloat(),
            keystorePending = json.optBoolean("keystorePending", false),
            lastError = json.optJSONObject("lastError")?.let { error ->
                BridgeErrorSnapshot(
                    message = error.optString("message", "Something went wrong"),
                    ageMs = error.optLong("ageMs", 0L),
                )
            },
            targetPackage = json.optString("targetPackage", ""),
            overlay = BridgeOverlay(
                state = overlay?.optString("state", "hidden") ?: "hidden",
                visible = overlay?.optBoolean("visible", false) ?: false,
                dictationActive = overlay?.optBoolean("dictationActive", false) ?: false,
            ),
            pendingInsertion = if (pending != null) BridgePendingInsertion(
                seq = pending.optLong("seq", 0L),
                text = pending.optString("text", ""),
                ageMs = pending.optLong("ageMs", 0L),
            ) else null,
        )
    }

    /** Reports IME + focus; returns the overlay decision (visible/state). */
    fun postFocus(keyboardVisible: Boolean, hasEditableFocus: Boolean): JSONObject? =
        request(
            "POST", "/v1/focus",
            JSONObject()
                .put("keyboardVisible", keyboardVisible)
                .put("hasEditableFocus", hasEditableFocus),
        )

    fun startRecording(pkg: String, hasEditableFocus: Boolean, supportsSetText: Boolean): JSONObject? =
        request(
            "POST", "/v1/recording/start",
            JSONObject()
                .put("package", pkg)
                .put("hasEditableFocus", hasEditableFocus)
                .put("supportsSetText", supportsSetText),
        )

    fun stopRecording(): JSONObject? = request("POST", "/v1/recording/stop")

    fun cancelRecording(): JSONObject? = request("POST", "/v1/recording/cancel")

    fun retryTranscription(): JSONObject? = request("POST", "/v1/recording/retry")

    fun ackInsertion(
        seq: Long,
        success: Boolean,
        strategy: String,
        error: String?,
        pkg: String,
        discard: Boolean = false,
    ): JSONObject? =
        request(
            "POST", "/v1/insertion/ack",
            JSONObject()
                .put("seq", seq)
                .put("success", success)
                .put("strategy", strategy)
                .put("error", error ?: "")
                .put("package", pkg)
                .put("discard", discard),
        )

    /** Push one Keystore-unlocked credential into Rust's memory cache. */
    fun pushCredential(provider: String, key: String): JSONObject? =
        request(
            "POST", "/v1/credential",
            JSONObject().put("provider", provider).put("key", key),
        )

    /** Clear Rust's transient credential cache on lock or service revocation. */
    fun clearCredentials(): JSONObject? = request("POST", "/v1/credentials/clear")

    /**
     * Fetch a staged Keystore rotation (single delivery — the backend clears
     * it on read and never persists it). Returns null when nothing is staged.
     */
    fun takeKeystorePending(): Pair<String, String>? {
        val json = request("GET", "/v1/keystore/pending") ?: return null
        if (json.isNull("provider")) return null
        return Pair(json.optString("provider", ""), json.optString("key", ""))
    }
}
