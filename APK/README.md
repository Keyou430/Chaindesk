# ChainDesk Android development

Standalone Kotlin/Compose + JNI/Go Android application, currently
`0.1.0-dev.1` / versionCode `4`. Imports a bounded Clash YAML subscription,
stores profiles with Android Keystore encryption, and builds a fixed
subscription-front → authenticated SOCKS5-exit chain. The UI, foreground
notification and connection worker share one generation-based session state.
Product traffic starts only after the current chain's directed HTTPS check.

This is a development candidate. ARM64 phone, vendor/minimum-API, independent
network, full protocol/privacy and elapsed-use acceptance remain open. See
[development status](docs/development-status.md) for actual evidence and gaps.
The eight protocol combinations are offline compiler candidates; this is not
a claim that all eight passed Android traffic tests.

## Build on Windows

Use PowerShell 7. All pinned tools and caches are isolated under
`%USERPROFILE%/.cache/chaindesk-android`. The scripts reuse the existing proxy
for dependency downloads without changing host settings.

```powershell
pwsh -NoProfile -File ./APK/scripts/prepareToolchain.ps1 -InstallSdk
pwsh -NoProfile -File ./APK/scripts/testCore.ps1
pwsh -NoProfile -File ./APK/scripts/buildDevelopment.ps1 -Abi arm64-v8a
```

The final command builds, runs JVM tests and Lint, verifies the signature,
ZIP/ELF 16 KiB alignment and packaged ABI, checks for lab credential files,
and creates a hash-named directory under `APK/dist/development/`. It includes
the APK, source ZIP, source-input hashes, build record and SHA256SUMS. An
existing distribution is preserved. An explicit conflicting SDK in
`android/local.properties` is rejected without rewriting the file.

Packages use Android's debug signing identity and application ID
`dev.chaindesk.android.debug`. They are installable validation packages, not
long-term release-signed builds. `minSdk=26` and `targetSdk=37` are build
settings; successful compilation does not establish support on every such
system. The ARM64 build is not evidence of ARM64 execution.

## Controlled Android validation

The optional emulator is provisioned separately; this does not start it or
change Windows virtualization:

```powershell
pwsh -NoProfile -File ./APK/scripts/prepareEmulator.ps1
pwsh -NoProfile -File ./APK/scripts/buildDevelopment.ps1 -Abi x86_64
```

The test APK contains framework instrumentation for native data, resource
tampering, encrypted storage, bounded HTTPS downloads, diagnostic transactions
and controlled product sessions. Test certificates/private keys exist only in
`androidTest` assets and source fixtures. They must never enter the app APK.

`android/trafficProbe` is a separate ordinary-UID lab application. Its controlled
TLS request and external server counters distinguish a local TCP handshake
from successful target forwarding. See its [usage](android/trafficProbe/README.md)
and the [product validation procedure](docs/product-validation.md).

## Use and protection boundaries

For installation, first connection, troubleshooting and updates in Chinese,
see the [development user guide](docs/install-upgrade.zh-CN.md).

1. Import a local YAML file or HTTPS subscription while disconnected. Review
   rejected candidates and select an eligible front.
2. Set the fixed SOCKS5 exit, authentication and routing preset; save offline.
   Passwords are hidden by default. With credentials not remembered, process
   restart requires reentry.
3. Connect, grant notification/VPN authorization, and grant local-network
   access when using private endpoints or declared LAN exceptions. A native
   start alone is not reported as connected: directed HTTPS must pass, plus
   consistent expected IPv4 verification when configured.
4. Disconnect explicitly before changing subscription/exit/routing fields.
   Switching a front retains the captured TUN and attempts a previously
   verified configuration on failure within the same connection budget.
5. Diagnostic export requires a current preview/hash. It excludes subscription
   URLs, credentials, raw configuration, endpoint hosts and actual IP values.

`allProxy` includes explicit localhost/private-IPv4 TCP exceptions.
`chinaDirect` additionally uses the bundled, hash-verified CN rules and declared
DNS infrastructure. Neither preset adds a direct or front-only fallback for
proxy traffic. Non-DNS UDP is unsupported. IPv6-only/NAT64/Private DNS behavior
still requires the plan's device/network validation.

Retaining a TUN after a connection failure is limited to the living VPN
service. Force-stop, process/native crash, authorization revocation or another
VPN can remove Android's VPN routes. This candidate does not implement an
independent process-death kill switch or claim protection after VPN teardown.

## Historical A0 evidence

`dist/a0/`, `dist/a0-runtime-v2/`, `dist/a0-runtime/` and `dist/a0-emulator/`
remain sealed checkpoints. Current source must not overwrite them.
`buildProbe.ps1` rejects current versions and is only usable from historical
A0 source snapshots. Standalone device-test compilation now writes generated
output to `android/coreBridge/build/deviceTests/`.

Historical device/network observations are in
[the A0 procedure](docs/a0-device-validation.md),
[the A0 emulator report](docs/a0-emulator-validation.md), and
[the original baseline](docs/android-baseline.md).
The maintainer's [implementation plan](ChainDesk-Android-开发执行计划.md) remains
unchanged. Current development evidence supplements it and does not mark
missing A0–A5 acceptance gates as passed.
