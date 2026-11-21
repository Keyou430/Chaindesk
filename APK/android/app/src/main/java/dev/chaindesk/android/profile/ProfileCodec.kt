package dev.chaindesk.android.profile

import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream
import java.io.DataInputStream
import java.io.DataOutputStream
import java.nio.ByteBuffer
import java.nio.CharBuffer
import java.nio.charset.CodingErrorAction

internal object ProfileCodec {
    const val MAX_SUBSCRIPTION_BYTES = 10 * 1024 * 1024
    const val MAX_FILE_BYTES = 12 * 1024 * 1024
    private const val MAX_FRONT_COUNT = 5000

    fun encode(profile: ProfileData): ByteArray {
        if (profile.exitPort !in 1..65535 || profile.excludedFrontIds.size > MAX_FRONT_COUNT) {
            throw ProfileException("PROFILE_INVALID")
        }
        val username = utf8(profile.exitUsername, 255)
        val password = utf8(profile.exitPassword, 255)
        try {
            val buffer = ByteArrayOutputStream()
            DataOutputStream(buffer).use { output ->
                output.writeInt(2)
                output.writeString(profile.subscription, MAX_SUBSCRIPTION_BYTES)
                output.writeString(profile.sourceUrl, 8192)
                output.writeString(profile.selectedFrontId, 256)
                output.writeInt(profile.excludedFrontIds.size)
                profile.excludedFrontIds.forEach { output.writeString(it, 256) }
                output.writeString(profile.exitHost, 253)
                output.writeInt(profile.exitPort)
                output.writeString(if (profile.rememberCredentials) profile.exitUsername else "", 255)
                output.writeString(if (profile.rememberCredentials) profile.exitPassword else "", 255)
                output.writeBoolean(profile.rememberCredentials)
                output.writeString(profile.preset, 32)
                output.writeString(profile.expectedIpv4, 15)
                output.writeBoolean(profile.automaticFront)
                output.writeBoolean(profile.healthMonitoring)
                output.writeBoolean(profile.exitAuthenticationRequired || profile.exitUsername.isNotEmpty() || profile.exitPassword.isNotEmpty())
            }
            return buffer.toByteArray()
        } finally {
            username.fill(0)
            password.fill(0)
        }
    }

    fun decode(bytes: ByteArray): ProfileData {
        try {
            DataInputStream(ByteArrayInputStream(bytes)).use { input ->
                if (input.readInt() != 2) throw ProfileException("PROFILE_UNSUPPORTED_VERSION")
                val subscription = input.readString(MAX_SUBSCRIPTION_BYTES)
                val sourceUrl = input.readString(8192)
                val selectedFrontId = input.readString(256)
                val count = input.readInt()
                if (count !in 0..MAX_FRONT_COUNT) throw ProfileException("PROFILE_CORRUPT")
                val excluded = LinkedHashSet<String>()
                repeat(count) {
                    if (!excluded.add(input.readString(256))) throw ProfileException("PROFILE_CORRUPT")
                }
                val profile = ProfileData(
                    subscription = subscription,
                    sourceUrl = sourceUrl,
                    selectedFrontId = selectedFrontId,
                    excludedFrontIds = excluded,
                    exitHost = input.readString(253),
                    exitPort = input.readInt(),
                    exitUsername = input.readString(255),
                    exitPassword = input.readString(255),
                    rememberCredentials = input.readFlag(),
                    preset = input.readString(32),
                    expectedIpv4 = input.readString(15),
                    automaticFront = input.readFlag(),
                    healthMonitoring = input.readFlag(),
                    exitAuthenticationRequired = input.readFlag(),
                )
                if (input.read() != -1 || profile.exitPort !in 1..65535 ||
                    (!profile.rememberCredentials && (profile.exitUsername.isNotEmpty() || profile.exitPassword.isNotEmpty()))) {
                    throw ProfileException("PROFILE_CORRUPT")
                }
                return profile
            }
        } catch (error: ProfileException) {
            throw error
        } catch (_: Exception) {
            throw ProfileException("PROFILE_CORRUPT")
        }
    }

    private fun utf8(value: String, maximum: Int): ByteArray {
        if (value.length > maximum) throw ProfileException("PROFILE_TOO_LARGE")
        val bytes = try {
            Charsets.UTF_8.newEncoder().onMalformedInput(CodingErrorAction.REPORT)
                .onUnmappableCharacter(CodingErrorAction.REPORT).encode(CharBuffer.wrap(value))
        } catch (_: Exception) {
            throw ProfileException("PROFILE_INVALID")
        }
        if (bytes.remaining() > maximum) throw ProfileException("PROFILE_TOO_LARGE")
        return ByteArray(bytes.remaining()).also { bytes.get(it) }
    }

    private fun DataOutputStream.writeString(value: String, maximum: Int) {
        val bytes = utf8(value, maximum)
        try {
            writeInt(bytes.size)
            write(bytes)
        } finally {
            bytes.fill(0)
        }
    }

    private fun DataInputStream.readString(maximum: Int): String {
        val size = readInt()
        if (size !in 0..maximum || size > available()) throw ProfileException("PROFILE_CORRUPT")
        val bytes = ByteArray(size)
        try {
            readFully(bytes)
            return Charsets.UTF_8.newDecoder().onMalformedInput(CodingErrorAction.REPORT)
                .onUnmappableCharacter(CodingErrorAction.REPORT).decode(ByteBuffer.wrap(bytes)).toString()
        } finally {
            bytes.fill(0)
        }
    }

    private fun DataInputStream.readFlag(): Boolean = when (readUnsignedByte()) {
        0 -> false
        1 -> true
        else -> throw ProfileException("PROFILE_CORRUPT")
    }
}
