package org.developingtoday.id

/** JNI entry points for the Iroh remote-drive session bridge. */
class NativeBridge {
    companion object {
        init {
            System.loadLibrary("id_android_remote")
        }
    }

    external fun nativeStart(address: String, world: String, capability: String): Long

    external fun nativeSnapshot(sessionId: Long): String?

    external fun nativeStop(sessionId: Long)
}
