package dev.chaindesk.android

import android.content.Context
import android.os.SystemClock
import dev.chaindesk.android.profile.ProfileDownloader
import dev.chaindesk.android.profile.ProfileException
import java.io.ByteArrayOutputStream
import java.io.Closeable
import java.io.IOException
import java.net.InetAddress
import java.net.SocketException
import java.net.URL
import java.security.KeyStore
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger
import java.util.zip.GZIPOutputStream
import javax.net.ssl.HttpsURLConnection
import javax.net.ssl.KeyManagerFactory
import javax.net.ssl.SSLContext
import javax.net.ssl.SSLServerSocket
import javax.net.ssl.SSLSocket
import javax.net.ssl.SSLSocketFactory
import javax.net.ssl.TrustManagerFactory

object DownloaderChecks {
    fun run(context: Context) {
        val tls = TlsFixtures(context)
        checkHttpsAndPolicies(tls)
        checkWrongHostname(tls)
        checkCancellation(tls)
        checkTotalDeadline(tls)
    }

    private fun checkHttpsAndPolicies(tls: TlsFixtures) {
        val exactGzip = gzip(10 * 1024 * 1024)
        val oversizedGzip = gzip(10 * 1024 * 1024 + 1)
        val recoveries = AtomicInteger()
        LocalTlsServer(tls.localhost) { request ->
            when {
                request.path == "/gzip-exact" -> Reply(headers = mapOf("Content-Encoding" to "gzip"), body = exactGzip)
                request.path == "/gzip-over" -> Reply(headers = mapOf("Content-Encoding" to "gzip"), body = oversizedGzip)
                request.path == "/gzip-corrupt" -> Reply(headers = mapOf("Content-Encoding" to "gzip"), body = byteArrayOf(1, 2, 3))
                request.path == "/loop" -> Reply(302, mapOf("Location" to "/loop"))
                request.path == "/downgrade" -> Reply(302, mapOf("Location" to "http://127.0.0.1/forbidden"))
                request.path.startsWith("/redirect/") -> {
                    val step = request.path.substringAfterLast('/').toInt()
                    if (step == 3) Reply() else Reply(302, mapOf("Location" to "/redirect/${step + 1}"))
                }
                request.path == "/auth" -> Reply(401)
                request.path == "/unavailable" -> Reply(503)
                request.path == "/recover" && recoveries.incrementAndGet() < 3 -> Reply(503)
                else -> Reply()
            }
        }.use { server ->
            expectCode("DOWNLOAD_TLS_FAILED") { ProfileDownloader.download(server.url("/untrusted")) }
            check(server.accepted.get() == 1 && server.requests.isEmpty()) { "Default TLS trusted the fixture certificate or retried TLS" }
            val factory = tls.connections(server)
            check(download(server, "/ok", factory).contentEquals(BODY)) { "HTTPS body changed" }
            check(download(server, "/redirect/0", factory).contentEquals(BODY)) { "Three HTTPS redirects were rejected" }
            check(server.requests.count { it.path.startsWith("/redirect/") } == 4) { "HTTPS redirect chain was not followed" }

            val boundary = download(server, "/gzip-exact", factory)
            check(boundary.size == 10 * 1024 * 1024 && boundary.first() == 120.toByte() && boundary.last() == 120.toByte()) {
                "Decoded gzip boundary changed"
            }
            expectCode("DOWNLOAD_TOO_LARGE") { download(server, "/gzip-over", factory) }
            check(server.requests.count { it.path == "/gzip-over" } == 1) { "Oversized body was retried" }
            expectCode("DOWNLOAD_ENCODING_INVALID") { download(server, "/gzip-corrupt", factory) }
            check(server.requests.count { it.path == "/gzip-corrupt" } == 1) { "Invalid gzip was retried" }

            expectCode("DOWNLOAD_URL_INVALID") { download(server, "/downgrade", factory) }
            check(server.requests.count { it.path == "/downgrade" } == 1 && server.requests.none { it.path == "/forbidden" }) {
                "HTTPS downgrade was followed"
            }
            expectCode("DOWNLOAD_REDIRECT_LIMIT") { download(server, "/loop", factory) }
            check(server.requests.count { it.path == "/loop" } == 4) { "Redirect limit did not stop after three hops" }
            expectCode("DOWNLOAD_AUTH_FAILED") { download(server, "/auth", factory) }
            check(server.requests.count { it.path == "/auth" } == 1) { "Authentication failure was retried" }

            check(download(server, "/recover", factory).contentEquals(BODY)) { "Transient 503 did not recover" }
            assertBackoff(server.requests.filter { it.path == "/recover" })
            expectCode("DOWNLOAD_FAILED") { download(server, "/unavailable", factory) }
            assertBackoff(server.requests.filter { it.path == "/unavailable" })
            check(server.requests.none { it.hasAuthorization }) { "Downloader added unexpected credentials" }
        }
    }

    private fun checkWrongHostname(tls: TlsFixtures) {
        LocalTlsServer(tls.wrongHost) { Reply() }.use { server ->
            expectCode("DOWNLOAD_TLS_FAILED") { download(server, "/wrong-host", tls.connections(server)) }
            check(server.accepted.get() == 1 && server.requests.isEmpty()) { "Wrong-host certificate was accepted or retried" }
        }
    }

    private fun checkCancellation(tls: TlsFixtures) {
        LocalTlsServer(tls.localhost) { Reply(body = ByteArray(1024), byteDelayMillis = 100) }.use { server ->
            val started = SystemClock.elapsedRealtime()
            expectCode("DOWNLOAD_CANCELED") {
                ProfileDownloader.download(server.url("/cancel"), {
                    server.requests.firstOrNull()?.let { SystemClock.elapsedRealtime() - it.at >= 250 } == true
                }, tls.connections(server))
            }
            check(SystemClock.elapsedRealtime() - started < 5000 && server.requests.size == 1) { "I/O cancellation was delayed or retried" }
            server.awaitClientClose()
        }
    }

    private fun checkTotalDeadline(tls: TlsFixtures) {
        LocalTlsServer(tls.localhost) { request ->
            if (request.number < 3) Reply(503, body = byteArrayOf()) else Reply(body = ByteArray(128), byteDelayMillis = 400)
        }.use { server ->
            val started = SystemClock.elapsedRealtime()
            expectCode("DOWNLOAD_TIMEOUT") { download(server, "/slow", tls.connections(server)) }
            val elapsed = SystemClock.elapsedRealtime() - started
            check(elapsed in 19_000..22_000 && server.requests.size == 3) { "Retries and slow body exceeded the shared 20-second budget: $elapsed ms" }
            assertBackoff(server.requests.toList())
            check(server.writtenBytes.get() in 10..127) { "Deadline test did not exercise an unfinished streaming body" }
            server.awaitClientClose()
        }
    }

    private fun assertBackoff(requests: List<Request>) {
        check(requests.size == 3) { "Transient failure exceeded or skipped the two retries" }
        check(requests[1].at - requests[0].at >= 900 && requests[2].at - requests[1].at >= 1900) {
            "Retry backoff was shorter than 1/2 seconds"
        }
    }

    private fun download(server: LocalTlsServer, path: String, factory: (URL) -> HttpsURLConnection): ByteArray =
        ProfileDownloader.download(server.url(path), { false }, factory)

    private fun gzip(size: Int): ByteArray {
        val output = ByteArrayOutputStream()
        GZIPOutputStream(output).use { compressed ->
            val chunk = ByteArray(8192) { 120 }
            var remaining = size
            while (remaining > 0) {
                val count = minOf(remaining, chunk.size)
                compressed.write(chunk, 0, count)
                remaining -= count
            }
        }
        return output.toByteArray()
    }

    private fun expectCode(code: String, operation: () -> Unit) {
        try {
            operation()
            error("Expected $code")
        } catch (error: ProfileException) {
            check(error.code == code && error.message == code && error.cause == null) {
                "Expected $code, received ${error.code} or unsanitized failure"
            }
        }
    }

    private class TlsFixtures(context: Context) {
        private val password = "download-checks-only".toCharArray()
        private val localKeys = keys(context, "localhost.p12")
        private val wrongKeys = keys(context, "wrong-host.p12")
        val localhost = serverContext(localKeys)
        val wrongHost = serverContext(wrongKeys)
        private val clientSockets: SSLSocketFactory

        init {
            val trust = KeyStore.getInstance(KeyStore.getDefaultType()).apply {
                load(null)
                setCertificateEntry("localhost", localKeys.getCertificate("download-checks"))
                setCertificateEntry("wrong-host", wrongKeys.getCertificate("download-checks"))
            }
            val managers = TrustManagerFactory.getInstance(TrustManagerFactory.getDefaultAlgorithm()).apply { init(trust) }
            clientSockets = SSLContext.getInstance("TLS").apply { init(null, managers.trustManagers, null) }.socketFactory
        }

        fun connections(server: LocalTlsServer): (URL) -> HttpsURLConnection = { url ->
            check(url.protocol == "https" && url.host == "127.0.0.1" && url.port == server.port) { "Fixture connection escaped its local TLS server" }
            (url.openConnection() as HttpsURLConnection).apply { sslSocketFactory = clientSockets }
        }

        private fun keys(context: Context, name: String): KeyStore = KeyStore.getInstance("PKCS12").apply {
            context.assets.open("download-checks/$name").use { load(it, password) }
        }

        private fun serverContext(keys: KeyStore): SSLContext {
            val managers = KeyManagerFactory.getInstance(KeyManagerFactory.getDefaultAlgorithm()).apply { init(keys, password) }
            return SSLContext.getInstance("TLS").apply { init(managers.keyManagers, null, null) }
        }
    }

    private data class Request(val number: Int, val path: String, val at: Long, val hasAuthorization: Boolean)
    private data class Reply(
        val status: Int = 200,
        val headers: Map<String, String> = emptyMap(),
        val body: ByteArray = BODY,
        val byteDelayMillis: Long = 0,
    )

    private class LocalTlsServer(tls: SSLContext, private val reply: (Request) -> Reply) : Closeable {
        private val socket = tls.serverSocketFactory.createServerSocket(0, 8, InetAddress.getByName("127.0.0.1")) as SSLServerSocket
        private val closed = AtomicBoolean(false)
        private val active = CopyOnWriteArrayList<SSLSocket>()
        private val workers = CopyOnWriteArrayList<Thread>()
        val requests = CopyOnWriteArrayList<Request>()
        val accepted = AtomicInteger()
        val writtenBytes = AtomicInteger()
        val port: Int get() = socket.localPort
        private val accepting = Thread({
            while (!closed.get()) {
                val client = try {
                    socket.accept() as SSLSocket
                } catch (error: SocketException) {
                    if (closed.get()) break
                    throw error
                }
                accepted.incrementAndGet()
                active.add(client)
                val worker = Thread({ serve(client) }, "download-check-server-request").apply { isDaemon = true }
                workers.add(worker)
                worker.start()
            }
        }, "download-check-server").apply { isDaemon = true; start() }

        fun url(path: String): String = "https://127.0.0.1:$port$path"

        fun awaitClientClose() {
            val deadline = SystemClock.elapsedRealtime() + 5000
            while (active.isNotEmpty() && SystemClock.elapsedRealtime() < deadline) Thread.sleep(20)
            check(active.isEmpty()) { "Downloader retained its TLS connection after cancellation/timeout" }
        }

        private fun serve(client: SSLSocket) {
            try {
                client.use {
                    client.soTimeout = 5000
                    client.startHandshake()
                    val input = client.inputStream.bufferedReader(Charsets.US_ASCII)
                    val firstLine = input.readLine() ?: return
                    var headerSize = firstLine.length
                    var hasAuthorization = false
                    while (true) {
                        val line = input.readLine() ?: return
                        if (line.isEmpty()) break
                        headerSize += line.length
                        check(headerSize <= 16_384) { "Fixture request headers exceeded bound" }
                        if (line.startsWith("Authorization:", ignoreCase = true)) hasAuthorization = true
                    }
                    val request = Request(requests.size + 1, firstLine.split(' ')[1], SystemClock.elapsedRealtime(), hasAuthorization)
                    requests.add(request)
                    val response = reply(request)
                    val headers = buildString {
                        append("HTTP/1.1 ${response.status} Fixture\r\nConnection: close\r\nContent-Length: ${response.body.size}\r\n")
                        response.headers.forEach { (name, value) -> append("$name: $value\r\n") }
                        append("\r\n")
                    }
                    val output = client.outputStream
                    output.write(headers.toByteArray(Charsets.US_ASCII))
                    output.flush()
                    if (response.byteDelayMillis == 0L) {
                        output.write(response.body)
                        writtenBytes.addAndGet(response.body.size)
                        output.flush()
                    } else {
                        for (byte in response.body) {
                            output.write(byte.toInt())
                            output.flush()
                            writtenBytes.incrementAndGet()
                            Thread.sleep(response.byteDelayMillis)
                        }
                    }
                }
            } catch (_: IOException) {
                // TLS rejection and canceled clients close their fixture sockets.
            } catch (_: InterruptedException) {
                Thread.currentThread().interrupt()
            } finally {
                active.remove(client)
            }
        }

        override fun close() {
            closed.set(true)
            socket.close()
            accepting.join(1000)
            active.forEach { runCatching { it.close() } }
            workers.forEach { it.interrupt() }
            workers.forEach { it.join(1000) }
            check(!accepting.isAlive && workers.none { it.isAlive }) { "Fixture server threads did not stop" }
        }
    }

    private val BODY = "proxies: []\n".toByteArray(Charsets.UTF_8)
}
