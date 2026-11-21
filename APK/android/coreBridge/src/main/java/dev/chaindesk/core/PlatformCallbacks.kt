package dev.chaindesk.core

interface PlatformCallbacks {
    fun protectSocket(fd: Int, networkHandle: Long): Boolean
}
