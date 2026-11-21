package dev.chaindesk.android.profile

data class ProfileData(
    val subscription: String = "",
    val sourceUrl: String = "",
    val selectedFrontId: String = "",
    val excludedFrontIds: Set<String> = emptySet(),
    val exitHost: String = "",
    val exitPort: Int = 1080,
    val exitUsername: String = "",
    val exitPassword: String = "",
    val rememberCredentials: Boolean = false,
    val preset: String = "allProxy",
    val expectedIpv4: String = "",
    val automaticFront: Boolean = false,
    val healthMonitoring: Boolean = false,
    val exitAuthenticationRequired: Boolean = false,
)
