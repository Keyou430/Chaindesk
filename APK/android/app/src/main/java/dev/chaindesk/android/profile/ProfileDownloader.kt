package dev.chaindesk.android.profile

import android.os.SystemClock
import java.io.ByteArrayOutputStream
import java.io.EOFException
import java.io.IOException
import java.io.InterruptedIOException
import java.net.ProtocolException
import java.net.URI
import java.net.URL
import java.net.UnknownServiceException
import java.util.concurrent.ExecutionException
import java.util.concurrent.FutureTask
import java.util.concurrent.TimeUnit
import java.util.concurrent.TimeoutException
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference
import java.util.zip.GZIPInputStream
import java.util.zip.ZipException
import javax.net.ssl.HttpsURLConnection
import javax.net.ssl.SSLException

object ProfileDownloader {
    fun download(url: String, canceled: () -> Boolean = { false }): ByteArray =
        download(url, canceled) { it.openConnection() as HttpsURLConnection }

    internal fun download(
        url: String,
        canceled: () -> Boolean,
        connectionFactory: (URL) -> HttpsURLConnection,
    ): ByteArray {
        val deadline = SystemClock.elapsedRealtime() + 20_000
        val source = checkedUrl(url)
        if (canceled()) throw ProfileException("DOWNLOAD_CANCELED")
        val stopped = AtomicBoolean(false)
        val active = AtomicReference<HttpsURLConnection?>()
        val task = FutureTask { fetch(source, deadline, stopped, active, connectionFactory) }
        Thread(task, "profile-download").apply { isDaemon = true; start() }
        try {
            while (true) {
                if (canceled()) throw ProfileException("DOWNLOAD_CANCELED")
                val remaining = remaining(deadline, stopped)
                try {
                    val bytes = task.get(minOf(remaining.toLong(), 100), TimeUnit.MILLISECONDS)
                    if (canceled()) {
                        bytes.fill(0)
                        throw ProfileException("DOWNLOAD_CANCELED")
                    }
                    if (SystemClock.elapsedRealtime() >= deadline) {
                        bytes.fill(0)
                        throw ProfileException("DOWNLOAD_TIMEOUT")
                    }
                    return bytes
                } catch (_: TimeoutException) {
                    continue
                }
            }
        } catch (error: ExecutionException) {
            throw error.cause as? ProfileException ?: ProfileException("DOWNLOAD_FAILED")
        } catch (_: InterruptedException) {
            Thread.currentThread().interrupt()
            throw ProfileException("DOWNLOAD_CANCELED")
        } finally {
            stopped.set(true)
            task.cancel(true)
            // DNS and TLS may hold platform locks; cleanup must not extend the caller's deadline.
            active.get()?.let { connection ->
                Thread({ runCatching { connection.disconnect() } }, "profile-download-close")
                    .apply { isDaemon = true; start() }
            }
        }
    }

    private fun fetch(
        source: URL,
        deadline: Long,
        stopped: AtomicBoolean,
        active: AtomicReference<HttpsURLConnection?>,
        connectionFactory: (URL) -> HttpsURLConnection,
    ): ByteArray {
        var redirects = 0
        repeat(3) { attempt ->
            var current = source
            try {
                while (true) {
                    val timeout = remaining(deadline, stopped)
                    val connection = connectionFactory(current)
                    active.set(connection)
                    try {
                        remaining(deadline, stopped)
                        connection.instanceFollowRedirects = false
                        connection.useCaches = false
                        connection.connectTimeout = timeout
                        connection.readTimeout = timeout
                        connection.setRequestProperty("Accept-Encoding", "gzip")
                        connection.setRequestProperty("Accept", "application/yaml, text/yaml, text/plain, */*")
                        val status = connection.responseCode
                        remaining(deadline, stopped)
                        when {
                            status in REDIRECT_STATUSES -> {
                                if (redirects == 3) throw ProfileException("DOWNLOAD_REDIRECT_LIMIT")
                                val location = connection.getHeaderField("Location")
                                    ?: throw ProfileException("DOWNLOAD_REDIRECT_INVALID")
                                current = try {
                                    checkedUrl(current.toURI().resolve(location).toString())
                                } catch (error: ProfileException) {
                                    throw error
                                } catch (_: Exception) {
                                    throw ProfileException("DOWNLOAD_REDIRECT_INVALID")
                                }
                                redirects++
                            }
                            status == 401 || status == 403 || status == 407 -> throw ProfileException("DOWNLOAD_AUTH_FAILED")
                            status in 500..599 -> throw RetryableResponse()
                            status !in 200..299 -> throw ProfileException("DOWNLOAD_HTTP_FAILED")
                            else -> return readBody(connection, deadline, stopped)
                        }
                    } finally {
                        try {
                            runCatching { connection.errorStream?.close() }
                            connection.disconnect()
                        } finally {
                            active.compareAndSet(connection, null)
                        }
                    }
                }
            } catch (error: ProfileException) {
                throw error
            } catch (_: SSLException) {
                throw ProfileException("DOWNLOAD_TLS_FAILED")
            } catch (_: ProtocolException) {
                throw ProfileException("DOWNLOAD_PROTOCOL_FAILED")
            } catch (_: UnknownServiceException) {
                throw ProfileException("DOWNLOAD_PROTOCOL_FAILED")
            } catch (_: ZipException) {
                throw ProfileException("DOWNLOAD_ENCODING_INVALID")
            } catch (_: InterruptedIOException) {
                remaining(deadline, stopped)
                if (attempt == 2) throw ProfileException("DOWNLOAD_FAILED")
            } catch (_: IOException) {
                remaining(deadline, stopped)
                if (attempt == 2) throw ProfileException("DOWNLOAD_FAILED")
            } catch (_: Exception) {
                throw ProfileException("DOWNLOAD_FAILED")
            }
            val resumeAt = SystemClock.elapsedRealtime() + (attempt + 1) * 1000
            while (SystemClock.elapsedRealtime() < resumeAt) {
                val delay = minOf(remaining(deadline, stopped).toLong(), resumeAt - SystemClock.elapsedRealtime(), 100)
                if (delay > 0) {
                    try {
                        Thread.sleep(delay)
                    } catch (_: InterruptedException) {
                        throw ProfileException("DOWNLOAD_CANCELED")
                    }
                }
            }
        }
        throw ProfileException("DOWNLOAD_FAILED")
    }

    private fun readBody(connection: HttpsURLConnection, deadline: Long, stopped: AtomicBoolean): ByteArray {
        val encoding = connection.contentEncoding?.trim()?.lowercase()
        if (encoding != null && encoding != "identity" && encoding != "gzip") throw ProfileException("DOWNLOAD_ENCODING_INVALID")
        if (encoding != "gzip" && connection.contentLengthLong > ProfileCodec.MAX_SUBSCRIPTION_BYTES) {
            throw ProfileException("DOWNLOAD_TOO_LARGE")
        }
        try {
            connection.inputStream.use { raw ->
                val decoded = if (encoding == "gzip") GZIPInputStream(raw) else raw
                decoded.use { input ->
                    val output = ByteArrayOutputStream()
                    val buffer = ByteArray(8192)
                    while (true) {
                        remaining(deadline, stopped)
                        val count = input.read(buffer)
                        if (count == -1) break
                        if (output.size() + count > ProfileCodec.MAX_SUBSCRIPTION_BYTES) throw ProfileException("DOWNLOAD_TOO_LARGE")
                        output.write(buffer, 0, count)
                    }
                    remaining(deadline, stopped)
                    return output.toByteArray()
                }
            }
        } catch (error: EOFException) {
            if (encoding == "gzip") throw ProfileException("DOWNLOAD_ENCODING_INVALID")
            throw error
        }
    }

    private fun checkedUrl(value: String): URL {
        try {
            if (value.length > 8192 || value.toByteArray(Charsets.UTF_8).size > 8192 || value.any { it <= ' ' || it == '\u007f' }) {
                throw ProfileException("DOWNLOAD_URL_INVALID")
            }
            val uri = URI(value)
            if (!uri.scheme.equals("https", ignoreCase = true) || uri.isOpaque || uri.host.isNullOrEmpty() ||
                uri.rawUserInfo != null || uri.rawFragment != null || (uri.port != -1 && uri.port !in 1..65535)) {
                throw ProfileException("DOWNLOAD_URL_INVALID")
            }
            return uri.toURL()
        } catch (_: Exception) {
            throw ProfileException("DOWNLOAD_URL_INVALID")
        }
    }

    private fun remaining(deadline: Long, stopped: AtomicBoolean): Int {
        if (stopped.get() || Thread.currentThread().isInterrupted) throw ProfileException("DOWNLOAD_CANCELED")
        val remaining = deadline - SystemClock.elapsedRealtime()
        if (remaining <= 0) throw ProfileException("DOWNLOAD_TIMEOUT")
        return remaining.toInt()
    }

    private class RetryableResponse : IOException()

    private val REDIRECT_STATUSES = setOf(301, 302, 303, 307, 308)
}
