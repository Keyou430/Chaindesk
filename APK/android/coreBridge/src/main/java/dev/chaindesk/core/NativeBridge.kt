package dev.chaindesk.core

object NativeBridge {
    init {
        System.loadLibrary("chaindesk_jni")
    }

    external fun selfCheck(): String
    external fun validateFixture(data: ByteArray): Boolean
    external fun start(data: ByteArray, tunFd: Int, networkHandle: Long, callbacks: PlatformCallbacks): Long
    external fun stop(sessionId: Long): Boolean
    external fun blockSockets(sessionId: Long)
    external fun cancelProbe(sessionId: Long)
    external fun activate(sessionId: Long): Boolean
    external fun importSubscription(data: ByteArray): ByteArray
    external fun compileChain(data: ByteArray): ByteArray
    external fun startChain(data: ByteArray, tunFd: Int, networkHandle: Long, callbacks: PlatformCallbacks): Long
    external fun probe(sessionId: Long, data: ByteArray): ByteArray
    external fun ruleManifest(): ByteArray
    external fun setRulesDirectory(data: ByteArray): ByteArray
}
