package dev.chaindesk.android.vpn

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.Manifest
import android.content.Intent
import android.content.pm.ServiceInfo
import android.content.pm.PackageManager
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.net.VpnService
import android.os.Build
import android.os.Handler
import android.os.HandlerThread
import android.os.Looper
import android.os.ParcelFileDescriptor
import dev.chaindesk.android.MainActivity
import dev.chaindesk.android.R
import dev.chaindesk.android.ChainDeskApplication
import dev.chaindesk.android.session.SessionIntent
import dev.chaindesk.android.session.SessionPhase
import dev.chaindesk.android.ui.SessionLabels
import dev.chaindesk.core.NativeBridge
import dev.chaindesk.core.PlatformCallbacks
import dev.chaindesk.core.CoreFailure
import java.util.concurrent.Executors

class ChainVpnService : VpnService(), PlatformCallbacks {
    private val workerThread = HandlerThread("a0-vpn").apply { start() }
    private val worker = Handler(workerThread.looper)
    private val mainHandler = Handler(Looper.getMainLooper())
    private lateinit var connectivity: ConnectivityManager
    @Volatile private var tun: ParcelFileDescriptor? = null
    @Volatile private var sessionId = 0L
    private var foregroundStarted = false
    private var networkCallback: ConnectivityManager.NetworkCallback? = null
    @Volatile private var boundNetwork: Network? = null
    @Volatile private var networkAvailable = false
    @Volatile internal var stopping = false
        private set
    private var profileMode = false
    private var stopGeneration: Long? = null
    private var physicalTracker: UnderlyingNetworkTracker? = null
    private val cancellation = Executors.newSingleThreadExecutor { Thread(it, "chain-cancel").apply { isDaemon = true } }
    private val profileSession by lazy { ProfileSession(this, worker) }
    internal val model get() = (application as ChainDeskApplication).model
    internal val hasTun get() = tun != null
    internal val nativeSessionId get() = sessionId

    override fun onCreate() {
        super.onCreate()
        connectivity = getSystemService(ConnectivityManager::class.java)
        getSystemService(NotificationManager::class.java).createNotificationChannel(
            NotificationChannel(CHANNEL, getString(R.string.vpn_channel), NotificationManager.IMPORTANCE_LOW),
        )
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_STOP) {
            stopping = true
            if (model.sessions.state.value.phase == SessionPhase.Disconnecting) profileMode = true
            stopGeneration = if (profileMode) model.sessions.submit(SessionIntent.Disconnect).generation else null
            if (profileMode) profileSession.cancelPending() else blockSockets()
            worker.post {
                release()
                finishService()
            }
            return START_NOT_STICKY
        }
        if (intent?.action == ACTION_CONNECT) return startProfile(intent, startId)
        val fixture = intent?.getByteArrayExtra(EXTRA_FIXTURE)
        if (stopping || intent?.action != ACTION_START || fixture == null || prepare(this) != null ||
            (Build.VERSION.SDK_INT >= 33 && checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED)) {
            fixture?.fill(0)
            if (!foregroundStarted) stopSelf(startId)
            return START_NOT_STICKY
        }
        try {
            val notification = notification(getString(R.string.vpn_starting))
            if (Build.VERSION.SDK_INT >= 34) {
                startForeground(NOTIFICATION_ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_SYSTEM_EXEMPTED)
            } else {
                startForeground(NOTIFICATION_ID, notification)
            }
            foregroundStarted = true
            getSystemService(NotificationManager::class.java).cancel(FAILURE_NOTIFICATION_ID)
        } catch (_: RuntimeException) {
            fixture.fill(0)
            stopSelf(startId)
            return START_NOT_STICKY
        }
        worker.post {
            try {
                if (!stopping && tun == null) startProbe(fixture)
            } catch (_: Exception) {
                blocked()
            } catch (_: LinkageError) {
                blocked()
            } finally {
                fixture.fill(0)
            }
        }
        return START_NOT_STICKY
    }

    private fun startProfile(intent: Intent, startId: Int): Int {
        val token = intent.getLongExtra(EXTRA_GENERATION, -1)
        val state = model.sessions.state.value
        if (stopping || state.generation != token || state.phase != SessionPhase.Connecting || fixtureActive()) {
            if (state.phase == SessionPhase.Disconnecting && !fixtureActive()) {
                profileMode = true
                stopGeneration = state.generation
            }
            if (!foregroundStarted) stopSelf(startId)
            return START_NOT_STICKY
        }
        profileMode = true
        val rejection = when {
            prepare(this) != null -> "VPN_PERMISSION_DENIED"
            Build.VERSION.SDK_INT >= 33 && checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED -> "NOTIFICATION_REQUIRED"
            else -> null
        }
        if (rejection != null) {
            profileSession.cancelPending()
            worker.post {
                stopProductCore()
                model.sessions.submit(SessionIntent.Failed(token, rejection, hasTun))
                if (!hasTun) finishService() else refreshProfileNotification()
            }
            return START_NOT_STICKY
        }
        try {
            val notification = notification(SessionLabels.title(state))
            if (Build.VERSION.SDK_INT >= 34) startForeground(NOTIFICATION_ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_SYSTEM_EXEMPTED)
            else startForeground(NOTIFICATION_ID, notification)
            foregroundStarted = true
        } catch (_: RuntimeException) {
            model.sessions.submit(SessionIntent.Failed(token, "FOREGROUND_SERVICE_UNAVAILABLE", hasTun))
            if (!hasTun) stopSelf(startId)
            return START_NOT_STICKY
        }
        profileSession.cancelPending()
        worker.post { profileSession.connect(token, intent.getStringExtra(EXTRA_FRONT_ID)) }
        return START_NOT_STICKY
    }

    private fun fixtureActive() = !profileMode && tun != null

    internal fun productNetwork(): Network {
        val candidate = physicalTracker?.network ?: if (tun == null) connectivity.activeNetwork else null
        return candidate?.takeIf { UnderlyingNetworkTracker.usable(connectivity.getNetworkCapabilities(it)) }
            ?: throw CoreFailure("NETWORK_UNAVAILABLE")
    }

    internal fun bindingIsCurrent(network: Network, id: Long): Boolean = !stopping && networkAvailable &&
        boundNetwork == network && physicalTracker?.network == network && sessionId == id

    internal fun boundTo(network: Network?): Boolean = boundNetwork == network

    internal fun captureProductTun(network: Network) {
        if (stopping || !UnderlyingNetworkTracker.usable(connectivity.getNetworkCapabilities(network))) throw CoreFailure("NETWORK_UNAVAILABLE")
        boundNetwork = network
        networkAvailable = true
        if (physicalTracker == null) {
            physicalTracker = UnderlyingNetworkTracker(connectivity, mainHandler) {
                if (!stopping && tun != null && (it != boundNetwork || !networkAvailable)) {
                    val previousBinding = boundNetwork
                    networkAvailable = false
                    profileSession.networkChanged(previousBinding)
                }
            }.also { it.start(network) }
        }
        if (tun == null) {
            tun = Builder().setSession("ChainDesk").setMtu(1500).addAddress("172.29.0.1", 30)
                .addRoute("0.0.0.0", 0).addDnsServer("172.29.0.2").setUnderlyingNetworks(arrayOf(network))
                .setBlocking(false).establish() ?: throw CoreFailure("VPN_PERMISSION_DENIED")
        } else if (!setUnderlyingNetworks(arrayOf(network))) {
            throw CoreFailure("NETWORK_BIND_FAILED")
        }
        if (stopping || !networkAvailable) throw CoreFailure("NETWORK_LOST")
    }

    internal fun startProductCore(data: ByteArray, network: Network): Long {
        if (stopping || !networkAvailable || boundNetwork != network) throw CoreFailure("NETWORK_LOST")
        val id = NativeBridge.startChain(data, checkNotNull(tun).fd, network.networkHandle, this)
        if (id == 0L) throw CoreFailure("CORE_START_FAILED")
        sessionId = id
        return id
    }

    internal fun stopProductCore() {
        val id = sessionId
        if (id != 0L) {
            NativeBridge.blockSockets(id)
            if (!NativeBridge.stop(id) && profileMode) model.reportFailure("CORE_STOP_FAILED")
            sessionId = 0L
        }
    }

    internal fun cancelNativeAsync(id: Long = sessionId) {
        if (id != 0L && !cancellation.isShutdown) cancellation.execute {
            NativeBridge.blockSockets(id)
            NativeBridge.cancelProbe(id)
        }
    }

    internal fun refreshProfileNotification() {
        if (Build.VERSION.SDK_INT >= 33 && checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) return
        if (foregroundStarted) getSystemService(NotificationManager::class.java).notify(NOTIFICATION_ID, notification(SessionLabels.title(model.sessions.state.value)))
    }

    internal fun finishWithoutCapture() {
        release()
        finishService()
    }

    private fun startProbe(fixture: ByteArray) {
        check(NativeBridge.validateFixture(fixture))
        val network = connectivity.activeNetwork?.takeIf { candidate ->
            connectivity.getNetworkCapabilities(candidate)?.let {
                it.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET) &&
                    it.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_VPN) &&
                    it.hasCapability(NetworkCapabilities.NET_CAPABILITY_VALIDATED)
            } == true
        } ?: error("No validated underlying network")
        boundNetwork = network
        networkAvailable = true
        val callback = object : ConnectivityManager.NetworkCallback() {
            override fun onLost(lost: Network) {
                if (stopping || lost != boundNetwork) return
                networkAvailable = false
                blockSockets()
                worker.post { if (!stopping) blocked() }
            }
        }
        connectivity.registerNetworkCallback(
            NetworkRequest.Builder()
                .addCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
                .addCapability(NetworkCapabilities.NET_CAPABILITY_NOT_VPN)
                .build(),
            callback,
        )
        networkCallback = callback
        check(!stopping && networkAvailable)
        tun = Builder()
            .setSession("ChainDesk A0")
            .setMtu(1500)
            .addAddress("172.29.0.1", 30)
            .addRoute("0.0.0.0", 0)
            .addDnsServer("172.29.0.2")
            .setUnderlyingNetworks(arrayOf(network))
            .setBlocking(false)
            .establish() ?: error("VPN permission revoked")
        val id = NativeBridge.start(fixture, checkNotNull(tun).fd, network.networkHandle, this)
        check(id != 0L)
        sessionId = id
        if (stopping || !networkAvailable) {
            blocked()
        } else {
            updateNotification(R.string.vpn_tun_ready)
        }
    }

    override fun protectSocket(fd: Int, networkHandle: Long): Boolean {
        val network = boundNetwork ?: return false
        if (stopping || !networkAvailable || network.networkHandle != networkHandle) return false
        return try {
            val capabilities = connectivity.getNetworkCapabilities(network) ?: return false
            if (!capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_VPN) || !protect(fd)) return false
            // fromFd duplicates the socket; adoptFd would close Go's original fd.
            ParcelFileDescriptor.fromFd(fd).use { network.bindSocket(it.fileDescriptor) }
            networkAvailable && boundNetwork == network
        } catch (_: Exception) {
            false
        }
    }

    private fun blockSockets() {
        try { NativeBridge.blockSockets(sessionId) } catch (_: LinkageError) { }
    }

    private fun blocked() {
        blockSockets()
        if (sessionId != 0L) NativeBridge.stop(sessionId)
        sessionId = 0
        if (tun == null) {
            stopping = true
            release()
            if (Build.VERSION.SDK_INT < 33 || checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED) {
                getSystemService(NotificationManager::class.java).notify(
                    FAILURE_NOTIFICATION_ID, notification(getString(R.string.vpn_not_established), false),
                )
            }
            finishService()
            return
        }
        // Keep the original TUN until explicit stop; a start failure cannot open a direct route.
        updateNotification(R.string.vpn_blocked)
    }

    private fun release() {
        networkAvailable = false
        blockSockets()
        if (sessionId != 0L) NativeBridge.stop(sessionId)
        sessionId = 0
        tun?.close()
        tun = null
        networkCallback?.let { connectivity.unregisterNetworkCallback(it) }
        networkCallback = null
        physicalTracker?.stop()
        physicalTracker = null
        boundNetwork = null
    }

    private fun finishService() {
        mainHandler.post {
            stopForeground(STOP_FOREGROUND_REMOVE)
            foregroundStarted = false
            stopSelf()
        }
    }

    override fun onRevoke() {
        stopping = true
        networkAvailable = false
        stopGeneration = if (profileMode) {
            model.reportFailure("VPN_PERMISSION_REVOKED")
            profileSession.cancelPending()
            model.sessions.submit(SessionIntent.Disconnect).generation
        } else null
        if (!profileMode) blockSockets()
        worker.post { release(); finishService() }
    }

    override fun onDestroy() {
        stopping = true
        networkAvailable = false
        if (profileMode) {
            profileSession.close()
            val state = model.sessions.state.value
            if (state.active && state.phase != SessionPhase.Disconnecting) model.sessions.submit(SessionIntent.Failed(state.generation, "SERVICE_STOPPED", hasTun))
        } else blockSockets()
        worker.post {
            release()
            if (profileMode) {
                stopGeneration?.let { model.sessions.submit(SessionIntent.Stopped(it)) }
                val state = model.sessions.state.value
                if (state.phase == SessionPhase.Degraded && state.errorCode == "SERVICE_STOPPED") model.sessions.submit(SessionIntent.Failed(state.generation, "SERVICE_STOPPED", false))
            }
            cancellation.shutdown()
        }
        workerThread.quitSafely()
        super.onDestroy()
    }

    private fun updateNotification(message: Int) {
        if (Build.VERSION.SDK_INT >= 33 && checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) return
        getSystemService(NotificationManager::class.java).notify(NOTIFICATION_ID, notification(getString(message)))
    }

    private fun notification(message: String, ongoing: Boolean = true): Notification {
        val open = PendingIntent.getActivity(this, 0, Intent(this, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE)
        val stop = PendingIntent.getService(this, 1, Intent(this, ChainVpnService::class.java).setAction(ACTION_STOP), PendingIntent.FLAG_IMMUTABLE)
        val builder = Notification.Builder(this, CHANNEL)
            .setSmallIcon(android.R.drawable.stat_sys_warning)
            .setContentTitle(getString(R.string.app_name))
            .setContentText(message)
            .setContentIntent(open)
            .setOngoing(ongoing)
        if (ongoing) builder.addAction(Notification.Action.Builder(null, getString(R.string.vpn_stop), stop).build())
        return builder.build()
    }

    companion object {
        const val ACTION_START = "dev.chaindesk.android.A0_START"
        const val ACTION_STOP = "dev.chaindesk.android.A0_STOP"
        const val EXTRA_FIXTURE = "fixture"
        const val ACTION_CONNECT = "dev.chaindesk.android.CONNECT"
        const val EXTRA_GENERATION = "generation"
        const val EXTRA_FRONT_ID = "frontId"
        private const val CHANNEL = "a0-vpn"
        private const val NOTIFICATION_ID = 1
        private const val FAILURE_NOTIFICATION_ID = 2
    }
}
