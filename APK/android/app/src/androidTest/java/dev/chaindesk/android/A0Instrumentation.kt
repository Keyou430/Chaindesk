package dev.chaindesk.android

import android.accessibilityservice.AccessibilityServiceInfo
import android.app.Activity
import android.app.Instrumentation
import android.app.ActivityManager
import android.content.Intent
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import android.net.VpnService
import android.os.Bundle
import android.os.Build
import android.os.ParcelFileDescriptor
import android.os.SystemClock
import android.system.Os
import android.util.Base64
import android.view.accessibility.AccessibilityNodeInfo
import dev.chaindesk.android.vpn.ChainVpnService
import dev.chaindesk.core.NativeBridge
import org.json.JSONObject
import java.io.ByteArrayInputStream
import java.io.File
import java.io.IOException
import java.net.InetSocketAddress
import java.net.Socket
import java.security.KeyStore
import java.security.cert.CertificateFactory
import javax.net.ssl.SSLContext
import javax.net.ssl.SSLSocket
import javax.net.ssl.TrustManagerFactory

class A0Instrumentation : Instrumentation() {
    private lateinit var inputs: Bundle

    override fun onCreate(arguments: Bundle?) {
        super.onCreate(arguments)
        inputs = arguments ?: Bundle()
        start()
    }

    override fun onStart() {
        val report = Bundle()
        try {
            if (inputs.getString("earlyCancelCheck") == "true") {
                AppFlowChecks(this).runEarlyCancellation()
                report.putString("earlyCancelCheck", "passed")
                report.putString("passed", "true")
                finish(Activity.RESULT_OK, report)
                return
            }
            inputs.getString("modelCheck")?.let {
                ModelCommitChecks.run(this, it)
                report.putString("modelCheck", it)
                report.putString("passed", "true")
                finish(Activity.RESULT_OK, report)
                return
            }
            inputs.getString("productFixture")?.let {
                inputs.getString("upgradeMode")?.let { mode ->
                    AppFlowChecks(this).runUpgrade(JSONObject(String(Base64.decode(it, Base64.DEFAULT), Charsets.UTF_8)), mode, report)
                    report.putString("passed", "true")
                    finish(Activity.RESULT_OK, report)
                    return
                }
                AppFlowChecks(this).run(JSONObject(String(Base64.decode(it, Base64.DEFAULT), Charsets.UTF_8)), report,
                    inputs.getString("lifecycleCycles", "0").toInt(), inputs.getString("networkSwitches", "0").toInt())
                report.putString("passed", "true")
                finish(Activity.RESULT_OK, report)
                return
            }
            if (inputs.getString("diagnosticChecks") == "true") {
                DiagnosticsChecks.run()
                report.putString("diagnosticChecks", "passed")
                report.putString("passed", "true")
                finish(Activity.RESULT_OK, report)
                return
            }
            if (inputs.getString("downloadChecks") == "true") {
                DownloaderChecks.run(context)
                report.putString("downloadChecks", "passed")
                report.putString("passed", "true")
                finish(Activity.RESULT_OK, report)
                return
            }
            if (inputs.getString("coreDataChecks") == "true") {
                CoreDataChecks.run(targetContext)
                report.putString("coreDataChecks", "passed")
                report.putString("passed", "true")
                finish(Activity.RESULT_OK, report)
                return
            }
            if (inputs.getString("profileChecks") == "true") {
                ProfileChecks.run(targetContext)
                report.putString("profileChecks", "passed")
                report.putString("passed", "true")
                finish(Activity.RESULT_OK, report)
                return
            }
            val native = JSONObject(NativeBridge.selfCheck())
            check(native.getBoolean("configValidated")) { "Native closed configuration failed" }
            check(!NativeBridge.validateFixture("{}".toByteArray())) { "Invalid fixture accepted" }
            report.putString("native", native.toString())
            val fixture = inputs.getString("fixture")
            if (fixture != null) {
                runVpnFlow(Base64.decode(fixture, Base64.DEFAULT), report)
            }
            report.putString("passed", "true")
            finish(Activity.RESULT_OK, report)
        } catch (error: Throwable) {
            runCatching { report.putString("failureNative", NativeBridge.selfCheck()) }
            runCatching {
                targetContext.startService(Intent(targetContext, ChainVpnService::class.java).setAction(ChainVpnService.ACTION_STOP))
            }
            report.putString("passed", "false")
            report.putString("failure", error.stackTraceToString())
            finish(Activity.RESULT_CANCELED, report)
        }
    }

    private fun runVpnFlow(fixture: ByteArray, report: Bundle) {
        check(NativeBridge.validateFixture(fixture)) { "Dedicated fixture rejected" }
        val activity = startActivitySync(Intent(targetContext, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        val authorization = VpnService.prepare(targetContext)
        if (authorization != null) {
            uiAutomation.serviceInfo = uiAutomation.serviceInfo.apply {
                flags = flags or AccessibilityServiceInfo.FLAG_REPORT_VIEW_IDS
            }
            runOnMainSync { activity.startActivityForResult(authorization, 1000) }
            await("VPN authorization dialog") {
                val button = uiAutomation.rootInActiveWindow
                    ?.findAccessibilityNodeInfosByViewId("android:id/button1")?.firstOrNull()
                button?.performAction(AccessibilityNodeInfo.ACTION_CLICK) == true
            }
            await("VPN authorization") { VpnService.prepare(targetContext) == null }
        }
        runOnMainSync { activity.finish() }
        await("probe activity shutdown") { activity.isDestroyed }

        if (inputs.getString("networkLoss") == "true") {
            check(Build.HARDWARE == "ranchu") { "Network toggling requires the dedicated emulator" }
            try {
                startVpn(fixture)
                requestTarget()
                for (command in listOf("svc wifi disable", "svc data disable")) {
                    ParcelFileDescriptor.AutoCloseInputStream(uiAutomation.executeShellCommand(command)).use { it.readBytes() }
                }
                await("native stop after network loss") {
                    JSONObject(NativeBridge.selfCheck()).optString("errorCode") != "CORE_SESSION_ACTIVE"
                }
                check(hasVpn()) { "Network loss released the original TUN" }
                ParcelFileDescriptor.AutoCloseInputStream(uiAutomation.executeShellCommand("svc wifi enable")).use { it.readBytes() }
                val connectivity = targetContext.getSystemService(ConnectivityManager::class.java)
                await("underlying network restoration") {
                    connectivity.allNetworks.any { network ->
                        connectivity.getNetworkCapabilities(network)?.let {
                            it.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_VPN) && it.hasCapability(NetworkCapabilities.NET_CAPABILITY_VALIDATED)
                        } == true
                    }
                }
                check(hasVpn()) { "Restoration released the original TUN" }
                try {
                    requestTarget()
                    error("Stopped native session reached HTTPS after network restoration")
                } catch (_: IOException) {
                    report.putString("networkLossHeldTun", "true")
                }
            } finally {
                for (command in listOf("svc wifi enable", "svc data enable")) {
                    ParcelFileDescriptor.AutoCloseInputStream(uiAutomation.executeShellCommand(command)).use { it.readBytes() }
                }
                stopVpn()
                fixture.fill(0)
            }
            return
        }

        if (inputs.getString("expectHttpsFailure") == "true") {
            startVpn(fixture)
            try {
                requestTarget()
                error("Invalid chain unexpectedly reached the HTTPS target")
            } catch (_: IOException) {
                report.putString("httpsBlocked", "true")
            } finally {
                stopVpn()
                fixture.fill(0)
            }
            return
        }

        startVpn(fixture)
        requestTarget()
        stopVpn()
        SystemClock.sleep(500)
        val baseline = fdCount()
        val baselineDescriptors = fdSnapshot()
        val counts = mutableListOf<Int>()
        val cycles = (inputs.getString("cycles") ?: "1").toInt()
        check(cycles in 1..50) { "Invalid cycle count" }
        repeat(cycles) {
            startVpn(fixture)
            requestTarget()
            stopVpn()
            SystemClock.sleep(200)
            counts += fdCount()
        }
        report.putString("cycles", cycles.toString())
        report.putString("fdBaseline", baseline.toString())
        report.putString("fdAfterCycles", counts.joinToString(","))
        report.putString("fdChanged", fdSnapshot().filter { baselineDescriptors[it.key] != it.value }.toString())
        check(counts.last() <= baseline) { "Process fd count grew across VPN cycles" }
        fixture.fill(0)
    }

    private fun startVpn(fixture: ByteArray) {
        val connectivity = targetContext.getSystemService(ConnectivityManager::class.java)
        val expectedNetwork = connectivity.activeNetwork
        val physicalTransports = listOf(NetworkCapabilities.TRANSPORT_WIFI, NetworkCapabilities.TRANSPORT_CELLULAR, NetworkCapabilities.TRANSPORT_ETHERNET)
        val expectedTransports = physicalTransports.filter { connectivity.getNetworkCapabilities(expectedNetwork)?.hasTransport(it) == true }
        targetContext.startForegroundService(Intent(targetContext, ChainVpnService::class.java)
            .setAction(ChainVpnService.ACTION_START)
            .putExtra(ChainVpnService.EXTRA_FIXTURE, fixture))
        await("native VPN start") {
            JSONObject(NativeBridge.selfCheck()).optString("errorCode") == "CORE_SESSION_ACTIVE" &&
                connectivity.getNetworkCapabilities(connectivity.activeNetwork)?.hasTransport(NetworkCapabilities.TRANSPORT_VPN) == true
        }
        if (inputs.getString("checkDefaultNetwork") == "true") {
            val capabilities = connectivity.allNetworks.mapNotNull { connectivity.getNetworkCapabilities(it) }
                .first { it.hasTransport(NetworkCapabilities.TRANSPORT_VPN) }
            check(expectedTransports.isNotEmpty() && physicalTransports.filter { capabilities.hasTransport(it) } == expectedTransports) {
                "VPN did not bind the pre-VPN default $expectedNetwork: $capabilities"
            }
        }
    }

    private fun stopVpn() {
        targetContext.startService(Intent(targetContext, ChainVpnService::class.java).setAction(ChainVpnService.ACTION_STOP))
        await("VPN shutdown") {
            val manager = targetContext.getSystemService(ActivityManager::class.java)
            val running = manager.getRunningServices(Int.MAX_VALUE).any { it.service.className == ChainVpnService::class.java.name }
            JSONObject(NativeBridge.selfCheck()).optString("errorCode") != "CORE_SESSION_ACTIVE" && !hasVpn() && !running
        }
    }

    private fun hasVpn(): Boolean {
        val connectivity = targetContext.getSystemService(ConnectivityManager::class.java)
        return connectivity.allNetworks.any {
            connectivity.getNetworkCapabilities(it)?.hasTransport(NetworkCapabilities.TRANSPORT_VPN) == true
        }
    }

    private fun requestTarget() {
        val connectivity = targetContext.getSystemService(ConnectivityManager::class.java)
        check(connectivity.getNetworkCapabilities(connectivity.activeNetwork)?.hasTransport(NetworkCapabilities.TRANSPORT_VPN) == true) { "HTTPS started before VPN became the app default" }
        val certificate = CertificateFactory.getInstance("X.509").generateCertificate(
            ByteArrayInputStream(Base64.decode(inputs.getString("certificate"), Base64.DEFAULT)),
        )
        val store = KeyStore.getInstance(KeyStore.getDefaultType()).apply {
            load(null)
            setCertificateEntry("a0-controlled-target", certificate)
        }
        val trust = TrustManagerFactory.getInstance(TrustManagerFactory.getDefaultAlgorithm()).apply { init(store) }
        val tls = SSLContext.getInstance("TLS").apply { init(null, trust.trustManagers, null) }
        val port = inputs.getString("targetPort")!!.toInt()
        Socket().use { connection ->
            connection.connect(InetSocketAddress("10.0.2.2", port), 10_000)
            (tls.socketFactory.createSocket(connection, "example.com", port, true) as SSLSocket).use { socket ->
                socket.soTimeout = 10_000
                socket.sslParameters = socket.sslParameters.apply { endpointIdentificationAlgorithm = "HTTPS" }
                socket.startHandshake()
                val request = "GET /a0?nonce=${System.nanoTime()} HTTP/1.1\r\nHost: example.com\r\nConnection: close\r\n\r\n"
                socket.outputStream.write(request.toByteArray(Charsets.US_ASCII))
                val response = socket.inputStream.bufferedReader(Charsets.US_ASCII).readText()
                check(response.startsWith("HTTP/1.1 200") && response.endsWith("a0-chain-ok")) { "Controlled HTTPS failed" }
            }
        }
    }

    private fun fdCount(): Int = checkNotNull(File("/proc/self/fd").list()).size

    private fun fdSnapshot(): Map<String, String> = checkNotNull(File("/proc/self/fd").list()).mapNotNull { fd ->
        runCatching { fd to Os.readlink("/proc/self/fd/$fd") }.getOrNull()
    }.toMap()

    private fun await(label: String, condition: () -> Boolean) {
        val deadline = SystemClock.elapsedRealtime() + 20_000
        while (SystemClock.elapsedRealtime() < deadline) {
            if (condition()) return
            SystemClock.sleep(100)
        }
        error("Timed out waiting for $label")
    }
}
