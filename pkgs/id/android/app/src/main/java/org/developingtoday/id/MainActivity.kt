package org.developingtoday.id

import android.app.Activity
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.text.InputType
import android.view.Gravity
import android.view.View
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import org.json.JSONObject

/** Native shell for an explicit, foreground-only remote-drive session. */
class MainActivity : Activity() {
    private val mainHandler = Handler(Looper.getMainLooper())
    private var bridge: NativeBridge? = null
    private var activeSession: Long? = null
    private var poller: Runnable? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        val status = TextView(this).apply {
            text = "Not connected"
            textSize = 18f
            gravity = Gravity.CENTER
            contentDescription = "Connection status: disconnected"
        }
        val surface = TextView(this).apply {
            text = "Remote app surface\n\nConnect to a live app to view its screen."
            textSize = 16f
            gravity = Gravity.CENTER
            setBackgroundColor(0xffeeeeee.toInt())
            contentDescription = "Remote app surface (disconnected)"
        }
        val address = EditText(this).apply {
            hint = "Node ID, node-ID@IP:port, or Iroh EndpointAddr JSON"
            inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS
            setSingleLine(true)
            contentDescription = "World host address"
        }
        val world = EditText(this).apply {
            hint = "World name (optional)"
            inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS
            setSingleLine(true)
            contentDescription = "World name"
        }
        val capability = EditText(this).apply {
            hint = "Join capability"
            setSingleLine(true)
            inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD
            isSaveEnabled = false
            contentDescription = "World join capability"
        }
        val connect = Button(this).apply {
            text = "Connect"
            setOnClickListener {
                startConnection(address.text.toString(), world.text.toString(), capability.text.toString(), status, surface)
                capability.text.clear()
            }
        }
        val disconnect = Button(this).apply {
            text = "Disconnect"
            setOnClickListener { disconnect(status, surface) }
        }

        val content = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(24, 24, 24, 24)
            addView(status, LinearLayout.LayoutParams(-1, 0, 1f))
            addView(address, LinearLayout.LayoutParams(-1, -2))
            addView(world, LinearLayout.LayoutParams(-1, -2))
            addView(capability, LinearLayout.LayoutParams(-1, -2))
            addView(connect, LinearLayout.LayoutParams(-1, -2))
            addView(disconnect, LinearLayout.LayoutParams(-1, -2))
            addView(surface, LinearLayout.LayoutParams(-1, 0, 3f))
        }
        content.importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_YES
        setContentView(ScrollView(this).apply { addView(content) })

        try {
            bridge = NativeBridge()
        } catch (error: UnsatisfiedLinkError) {
            status.text = "Native transport unavailable: ${error.message.orEmpty()}"
        }
    }

    private fun startConnection(
        address: String,
        world: String,
        capability: String,
        status: TextView,
        surface: TextView,
    ) {
        val native = bridge
        if (native == null) {
            status.text = "Native transport is not loaded."
            return
        }
        if (address.isBlank() || capability.isBlank()) {
            status.text = "Enter a host address and join capability."
            return
        }

        disconnect(status, surface)
        val sessionId = native.nativeStart(address.trim(), world.trim(), capability)
        if (sessionId == 0L) {
            status.text = "Could not start the Iroh session."
            return
        }
        activeSession = sessionId
        status.text = "Connecting to world host…"
        surface.text = "Remote app surface\n\nWaiting for a contract-valid host view."
        surface.contentDescription = "Remote app surface (connecting)"
        poll(sessionId, status, surface)
    }

    private fun poll(sessionId: Long, status: TextView, surface: TextView) {
        val native = bridge ?: return
        val task = object : Runnable {
            override fun run() {
                if (activeSession != sessionId) return
                val snapshotText = native.nativeSnapshot(sessionId)
                if (snapshotText == null) {
                    status.text = "Could not read session status."
                    return
                }
                try {
                    val snapshot = JSONObject(snapshotText)
                    val state = snapshot.optString("status", "error")
                    when (state) {
                        "connected" -> {
                            status.text = "Connected to live app"
                            status.contentDescription = "Connection status: connected"
                            val view = snapshot.optJSONObject("view")
                            val text = view?.optString("data").orEmpty()
                            surface.text = if (text.isBlank()) "Remote app returned an empty view." else text
                            surface.contentDescription = "Remote app view"
                        }
                        "connecting" -> status.text = "Connecting to world host…"
                        "error" -> {
                            status.text = "Connection failed: ${snapshot.optString("error", "unknown error")}"
                            surface.contentDescription = "Remote app surface (connection failed)"
                            return
                        }
                        else -> {
                            status.text = "Not connected"
                            surface.contentDescription = "Remote app surface (disconnected)"
                            return
                        }
                    }
                } catch (error: Exception) {
                    status.text = "Invalid native session status: ${error.message.orEmpty()}"
                    return
                }
                mainHandler.postDelayed(this, 500)
            }
        }
        poller = task
        mainHandler.post(task)
    }

    private fun disconnect(status: TextView, surface: TextView) {
        poller?.let(mainHandler::removeCallbacks)
        poller = null
        activeSession?.let { bridge?.nativeStop(it) }
        activeSession = null
        status.text = "Not connected"
        status.contentDescription = "Connection status: disconnected"
        surface.text = "Remote app surface\n\nConnect to a live app to view its screen."
        surface.contentDescription = "Remote app surface (disconnected)"
    }

    override fun onDestroy() {
        val native = bridge
        activeSession?.let { native?.nativeStop(it) }
        poller?.let(mainHandler::removeCallbacks)
        super.onDestroy()
    }
}
