package dev.chaindesk.android.security

import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyPermanentlyInvalidatedException
import android.security.keystore.KeyProperties
import dev.chaindesk.android.profile.ProfileException
import java.security.GeneralSecurityException
import java.security.KeyStore
import java.security.ProviderException
import java.security.UnrecoverableKeyException
import javax.crypto.AEADBadTagException
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

internal class CredentialStore(private val alias: String) {
    fun encrypt(plaintext: ByteArray, allowCreate: Boolean): ByteArray = protect {
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.ENCRYPT_MODE, key(allowCreate))
        cipher.updateAAD(HEADER)
        val nonce = cipher.iv
        if (nonce.size != NONCE_SIZE) throw ProfileException("PROFILE_KEY_INVALID")
        HEADER + nonce + cipher.doFinal(plaintext)
    }

    fun decrypt(encrypted: ByteArray): ByteArray {
        if (encrypted.size < HEADER.size + NONCE_SIZE + 16 ||
            !encrypted.copyOfRange(0, 4).contentEquals(HEADER.copyOfRange(0, 4))) {
            throw ProfileException("PROFILE_CORRUPT")
        }
        if (encrypted[4] != HEADER[4]) throw ProfileException("PROFILE_UNSUPPORTED_VERSION")
        return protect {
            val cipher = Cipher.getInstance("AES/GCM/NoPadding")
            val nonce = encrypted.copyOfRange(HEADER.size, HEADER.size + NONCE_SIZE)
            cipher.init(Cipher.DECRYPT_MODE, key(false), GCMParameterSpec(128, nonce))
            cipher.updateAAD(HEADER)
            cipher.doFinal(encrypted, HEADER.size + NONCE_SIZE, encrypted.size - HEADER.size - NONCE_SIZE)
        }
    }

    fun clearKey() {
        try {
            KeyStore.getInstance("AndroidKeyStore").apply {
                load(null)
                deleteEntry(alias)
            }
        } catch (_: Exception) {
            throw ProfileException("PROFILE_CLEAR_FAILED")
        }
    }

    private fun key(allowCreate: Boolean): SecretKey {
        val store = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        if (store.containsAlias(alias)) {
            return store.getKey(alias, null) as? SecretKey ?: throw ProfileException("PROFILE_KEY_INVALID")
        }
        if (!allowCreate) throw ProfileException("PROFILE_KEY_MISSING")
        return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore").apply {
            init(KeyGenParameterSpec.Builder(alias, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setKeySize(256)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setRandomizedEncryptionRequired(true)
                .build())
        }.generateKey()
    }

    private fun <T> protect(operation: () -> T): T = try {
        operation()
    } catch (error: ProfileException) {
        throw error
    } catch (_: AEADBadTagException) {
        throw ProfileException("PROFILE_CORRUPT")
    } catch (_: KeyPermanentlyInvalidatedException) {
        throw ProfileException("PROFILE_KEY_INVALID")
    } catch (_: UnrecoverableKeyException) {
        throw ProfileException("PROFILE_KEY_INVALID")
    } catch (_: GeneralSecurityException) {
        throw ProfileException("PROFILE_KEY_INVALID")
    } catch (_: ProviderException) {
        throw ProfileException("PROFILE_KEY_INVALID")
    } catch (_: Exception) {
        throw ProfileException("PROFILE_KEY_INVALID")
    }

    companion object {
        private val HEADER = byteArrayOf(0x43, 0x44, 0x50, 0x46, 1)
        private const val NONCE_SIZE = 12
    }
}
