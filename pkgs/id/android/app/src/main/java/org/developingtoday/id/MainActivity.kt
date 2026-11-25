package org.developingtoday.id

import android.app.Activity
import android.os.Bundle
import android.view.Gravity
import android.view.View
import android.widget.LinearLayout
import android.widget.TextView

/** Minimal native shell; relay transport is added by the next S3b slice. */
class MainActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        val status = TextView(this).apply {
            text = "id relay PoC\nNot connected — relay transport is not wired yet."
            textSize = 18f
            gravity = Gravity.CENTER
        }
        val surface = TextView(this).apply {
            text = "Remote app surface\n\nConnection and input relay are not available in this build."
            textSize = 16f
            gravity = Gravity.CENTER
            setBackgroundColor(0xffeeeeee.toInt())
            contentDescription = "Remote app surface (disconnected)"
        }
        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(24, 24, 24, 24)
            addView(status, LinearLayout.LayoutParams(-1, 0, 1f))
            addView(surface, LinearLayout.LayoutParams(-1, 0, 3f))
        }
        root.importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_YES
        setContentView(root)
    }
}
