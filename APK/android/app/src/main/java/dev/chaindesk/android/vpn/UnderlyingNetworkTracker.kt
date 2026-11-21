package dev.chaindesk.android.vpn

import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.os.Build
import android.os.Handler

class UnderlyingNetworkTracker(
    private val connectivity: ConnectivityManager,
    private val handler: Handler,
    private val changed: (Network?) -> Unit,
) {
    private val candidates = mutableMapOf<Network, NetworkCapabilities>()
    @Volatile var network: Network? = null
        private set
    @Volatile private var closed = false

    private val callback = object : ConnectivityManager.NetworkCallback() {
        override fun onCapabilitiesChanged(candidate: Network, capabilities: NetworkCapabilities) {
            if (closed) return
            candidates[candidate] = capabilities
            if (Build.VERSION.SDK_INT >= 31) {
                publish(candidate.takeIf { usable(capabilities) })
            } else {
                chooseFallback()
            }
        }

        override fun onLost(candidate: Network) {
            if (closed) return
            candidates.remove(candidate)
            if (Build.VERSION.SDK_INT >= 31) {
                if (network == candidate) publish(null)
            } else {
                chooseFallback()
            }
        }
    }

    fun start(initial: Network) {
        network = initial
        connectivity.getNetworkCapabilities(initial)?.let { candidates[initial] = it }
        val request = NetworkRequest.Builder()
            .addCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
            .addCapability(NetworkCapabilities.NET_CAPABILITY_NOT_VPN).build()
        if (Build.VERSION.SDK_INT >= 31) {
            connectivity.registerBestMatchingNetworkCallback(request, callback, handler)
        } else {
            connectivity.registerNetworkCallback(request, callback, handler)
        }
    }

    fun stop() {
        closed = true
        connectivity.unregisterNetworkCallback(callback)
        network = null
    }

    private fun chooseFallback() {
        val actual = connectivity.activeNetwork?.takeIf { usable(connectivity.getNetworkCapabilities(it)) }
        val preferred = actual ?: candidates.filterValues(::usable).entries.minByOrNull { (_, capabilities) ->
            when {
                capabilities.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET) -> 0
                capabilities.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) -> 1
                else -> 2
            }
        }?.key
        publish(preferred)
    }

    private fun publish(candidate: Network?) {
        if (network != candidate) {
            network = candidate
            changed(candidate)
        }
    }

    companion object {
        fun usable(capabilities: NetworkCapabilities?): Boolean = capabilities != null &&
            capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET) &&
            capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_VPN) &&
            capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_VALIDATED)
    }
}
