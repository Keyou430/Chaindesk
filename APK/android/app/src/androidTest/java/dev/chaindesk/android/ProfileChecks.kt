package dev.chaindesk.android

import android.content.Context
import android.system.Os
import dev.chaindesk.android.profile.ProfileData
import dev.chaindesk.android.profile.ProfileDownloader
import dev.chaindesk.android.profile.ProfileException
import dev.chaindesk.android.profile.ProfileRepository
import dev.chaindesk.android.security.CredentialStore
import java.io.File
import java.nio.ByteBuffer
import java.security.KeyStore
import java.util.UUID

object ProfileChecks {
    fun run(context: Context) {
        val fixtureId = UUID.randomUUID().toString()
        val directory = File(context.noBackupFilesDir, "profile-checks-$fixtureId")
        check(directory.mkdir()) { "Cannot create isolated profile fixtures" }
        val file = File(directory, "profile.bin")
        val alias = "${context.packageName}.profile-checks.$fixtureId"
        val repository = ProfileRepository(file, alias)
        val profile = ProfileData(
            subscription = "proxies:\n  - name: fixture-secret-node\n    password: fixture-node-secret\n",
            sourceUrl = "https://example.com/subscription?token=fixture-secret-token",
            selectedFrontId = "fixture-front",
            excludedFrontIds = setOf("excluded-front"),
            exitHost = "exit.example.com",
            exitPort = 1081,
            exitUsername = "fixture-secret-user",
            exitPassword = "fixture-secret-password",
            rememberCredentials = true,
            expectedIpv4 = "192.0.2.8",
            automaticFront = true,
            healthMonitoring = true,
        )
        try {
            check(repository.load() == null) { "Empty vault was not empty" }
            repository.save(profile)
            check(ProfileRepository(file, alias).load() == profile.copy(exitAuthenticationRequired = true)) { "Keystore roundtrip lost profile or required authentication" }
            val keyStore = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
            check(keyStore.getKey(alias, null).encoded == null) { "Profile key was exportable" }
            assertNoPlaintext(directory)
            val first = file.readBytes()
            repository.save(profile)
            check(!first.contentEquals(file.readBytes())) { "Repeated saves reused encryption output" }

            val forgotten = repository.forgetCredentials()
            val withoutCredentials = profile.copy(exitUsername = "", exitPassword = "", rememberCredentials = false, exitAuthenticationRequired = true)
            check(forgotten == withoutCredentials && repository.load() == withoutCredentials) { "Forget changed other fields" }
            val transientCredentials = profile.copy(rememberCredentials = false)
            repository.save(transientCredentials)
            check(transientCredentials.exitPassword == "fixture-secret-password") { "Save mutated runtime credentials" }
            check(ProfileRepository(file, alias).load() == withoutCredentials) { "Unremembered credentials survived restart" }
            repository.save(withoutCredentials.copy(exitAuthenticationRequired = false))
            check(repository.load()?.exitAuthenticationRequired == false) { "Explicit no-auth profile retained authentication requirement" }
            repository.save(transientCredentials)

            expectCode("PROFILE_TOO_LARGE") {
                repository.save(profile.copy(subscription = "x".repeat(10 * 1024 * 1024 + 1)))
            }
            expectCode("PROFILE_TOO_LARGE") {
                repository.save(profile.copy(exitPassword = "界".repeat(86)))
            }
            check(repository.load() == withoutCredentials) { "Rejected input replaced old profile" }
            Os.chmod(directory.path, 320)
            Os.chmod(file.path, 256)
            try {
                expectCode("PROFILE_SAVE_FAILED") { repository.save(profile) }
                check(repository.load() == withoutCredentials) { "Failed disk write replaced old profile" }
            } finally {
                Os.chmod(directory.path, 448)
                Os.chmod(file.path, 384)
            }
            assertNoPlaintext(directory)

            val valid = file.readBytes()
            File("${file.path}.bak").writeBytes(valid)
            file.writeBytes(valid.copyOf(8))
            check(repository.load() == withoutCredentials) { "Interrupted atomic save did not restore old ciphertext" }
            val tampered = valid.copyOf().apply { this[lastIndex] = (this[lastIndex].toInt() xor 1).toByte() }
            file.writeBytes(tampered)
            expectCode("PROFILE_CORRUPT") { repository.load() }
            expectCode("PROFILE_CORRUPT") { repository.save(profile) }
            check(file.readBytes().contentEquals(tampered)) { "Tampered data was silently overwritten" }

            file.writeBytes(CredentialStore(alias).encrypt(ByteBuffer.allocate(4).putInt(99).array(), false))
            expectCode("PROFILE_UNSUPPORTED_VERSION") { repository.load() }
            file.writeBytes(valid)
            Os.chmod(directory.path, 320)
            try {
                expectCode("PROFILE_CLEAR_FAILED") { repository.clear() }
                check(file.readBytes().contentEquals(valid) && keyStore.containsAlias(alias)) { "Failed reset removed the profile key" }
            } finally {
                Os.chmod(directory.path, 448)
            }
            repository.clear()
            check(repository.load() == null && !keyStore.containsAlias(alias)) { "Reset retained profile or its key" }
            repository.save(profile)
            check(repository.load() == profile.copy(exitAuthenticationRequired = true)) { "Fresh profile failed after key reset" }
            val resetCiphertext = file.readBytes()
            keyStore.deleteEntry(alias)
            expectCode("PROFILE_KEY_MISSING") { repository.load() }
            expectCode("PROFILE_KEY_MISSING") { repository.save(profile) }
            check(file.readBytes().contentEquals(resetCiphertext) && !keyStore.containsAlias(alias)) { "Missing key was silently regenerated" }
            repository.clear()
            check(repository.load() == null) { "Clear retained profile" }

            for (url in listOf("http://example.com", "file:///tmp/profile", "https://user:pass@example.com", "https://example.com/#fragment")) {
                expectCode("DOWNLOAD_URL_INVALID") { ProfileDownloader.download(url) }
            }
            expectCode("DOWNLOAD_CANCELED") { ProfileDownloader.download("https://example.com") { true } }
        } finally {
            Os.chmod(directory.path, 448)
            directory.listFiles()?.forEach { check(it.delete()) { "Cannot remove owned profile fixture" } }
            check(directory.delete()) { "Cannot remove owned profile fixture directory" }
            KeyStore.getInstance("AndroidKeyStore").apply { load(null); deleteEntry(alias) }
        }
    }

    private fun assertNoPlaintext(directory: File) {
        val secrets = listOf("fixture-secret-node", "fixture-node-secret", "fixture-secret-token", "fixture-secret-user", "fixture-secret-password")
        directory.listFiles()?.filter { it.isFile }?.forEach { file ->
            val bytes = file.readBytes()
            for (secret in secrets) {
                val needle = secret.toByteArray(Charsets.UTF_8)
                check((0..bytes.size - needle.size).none { offset -> needle.indices.all { bytes[offset + it] == needle[it] } }) {
                    "Plaintext secret persisted in profile fixture"
                }
            }
        }
    }

    private fun expectCode(code: String, operation: () -> Unit) {
        try {
            operation()
            error("Expected $code")
        } catch (error: ProfileException) {
            check(error.code == code && error.message == code && error.cause == null) { "Unexpected or unsanitized profile failure" }
        }
    }
}
