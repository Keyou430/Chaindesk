package dev.chaindesk.android.profile

import android.content.Context
import android.util.AtomicFile
import dev.chaindesk.android.security.CredentialStore
import java.io.ByteArrayOutputStream
import java.io.File
import java.io.FileOutputStream

class ProfileRepository internal constructor(private val file: File, keyAlias: String) {
    constructor(context: Context) : this(
        File(context.noBackupFilesDir, "profile-v1.bin"),
        "${context.packageName}.profile.v1",
    )

    private val atomicFile = AtomicFile(file)
    private val credentials = CredentialStore(keyAlias)

    fun load(): ProfileData? = synchronized(LOCK) { loadLocked() }

    fun save(profile: ProfileData): Unit = synchronized(LOCK) {
        try {
            saveLocked(profile)
        } catch (error: ProfileException) {
            throw error
        } catch (_: Exception) {
            throw ProfileException("PROFILE_SAVE_FAILED")
        }
    }

    fun forgetCredentials(): ProfileData? = synchronized(LOCK) {
        loadLocked()?.copy(exitUsername = "", exitPassword = "", rememberCredentials = false)?.also { saveLocked(it) }
    }

    fun clear(): Unit = synchronized(LOCK) {
        try {
            atomicFile.delete()
            if (file.exists() || File("${file.path}.bak").exists() || File("${file.path}.new").exists()) {
                throw ProfileException("PROFILE_CLEAR_FAILED")
            }
            credentials.clearKey()
        } catch (error: ProfileException) {
            throw error
        } catch (_: Exception) {
            throw ProfileException("PROFILE_CLEAR_FAILED")
        }
    }

    private fun loadLocked(): ProfileData? {
        if (!file.exists() && !File("${file.path}.bak").exists()) return null
        val encrypted = try {
            atomicFile.openRead().use { input ->
                val result = ByteArrayOutputStream()
                val buffer = ByteArray(8192)
                while (true) {
                    val count = input.read(buffer)
                    if (count == -1) break
                    if (result.size() + count > ProfileCodec.MAX_FILE_BYTES) throw ProfileException("PROFILE_CORRUPT")
                    result.write(buffer, 0, count)
                }
                result.toByteArray()
            }
        } catch (error: ProfileException) {
            throw error
        } catch (_: Exception) {
            throw ProfileException("PROFILE_READ_FAILED")
        }
        try {
            val plaintext = credentials.decrypt(encrypted)
            try {
                return ProfileCodec.decode(plaintext)
            } finally {
                plaintext.fill(0)
            }
        } finally {
            encrypted.fill(0)
        }
    }

    private fun saveLocked(profile: ProfileData) {
        val existing = loadLocked() != null
        val plaintext = ProfileCodec.encode(profile)
        val encrypted = try {
            credentials.encrypt(plaintext, !existing)
        } finally {
            plaintext.fill(0)
        }
        var output: FileOutputStream? = null
        try {
            output = atomicFile.startWrite()
            output.write(encrypted)
            output.fd.sync()
            atomicFile.finishWrite(output)
            output = null
            atomicFile.openRead().use { input ->
                if (!input.readBytes().contentEquals(encrypted)) throw ProfileException("PROFILE_SAVE_FAILED")
            }
        } catch (_: Exception) {
            output?.let { runCatching { atomicFile.failWrite(it) } }
            throw ProfileException("PROFILE_SAVE_FAILED")
        } finally {
            encrypted.fill(0)
        }
    }

    companion object {
        private val LOCK = Any()
    }
}
