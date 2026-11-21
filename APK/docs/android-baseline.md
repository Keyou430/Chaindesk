# Android A0 baseline and evidence

Updated: 2026-11-20, Asia/Shanghai. Status: native-loading and VPN-runtime build
checkpoints plus supplemental emulator execution; A0 phone acceptance remains open. Tracking:
[android-local-issue.md](android-local-issue.md).

## Scope and source

The maintainer requested execution of the current v1.1 implementation plan.
The maintainer subsequently confirmed that no phone is available and requested
the buildable validation package first. That is the current delivery scope.
Work is on `codex/android-a0-core-bridge`, based on `f5f243d5`. The plan's existing
uncommitted dual-platform additions are preserved separately from this work.

The current runtime checkpoint is version 0.0.3-a0 under `dist/a0-runtime/`.
Version 2 and its original evidence are preserved under `dist/a0-runtime-v2/`.
The initial version 0.0.1 package and evidence below are historical and preserved
under `dist/a0/`. See the continued-runtime section for the added implementation.

The initial APK is a native-loading probe, not the product client. It contains
no VPN service, requests no Internet permission, and cannot route phone traffic.
It calls the fixed mihomo parser with a closed, local configuration and displays
build metadata. TUN integration and Android socket protection remain unverified.

| Input | Recorded value |
| --- | --- |
| mihomo | v1.19.33, commit `3f27ef39f8c96bbfbc8b0b9bb10f5c9ef60149d8` |
| Go module resolution | `go list -m -json ...@<commit>` resolved that commit to `v1.19.33` and the expected origin hash |
| Go module checksum | `github.com/metacubex/mihomo v1.19.33 h1:hwqYMItk3Jaeq8Sw2PBAtyHQh157ccUtWS7rXrBOMCU=` |
| Inspection archive | SHA-256 `030a128de8be6d70ddfd6fa4c229f1158d5c42a96451fffdea929331d3cbe5b8`; separately downloaded from the fixed GitHub commit |
| Android reference source | `MetaCubeX/ClashMetaForAndroid` commit `6e635124cc72f8c102b7e76d849c384deba28b3f`; read as reference, not imported as a whole client |
| Namespace | `dev.chaindesk.android`; the probe uses debug application ID `dev.chaindesk.android.debug` |
| Platform candidate | ARM64, min API 26, target API 37; supported-device claims await actual testing |

## Portable toolchain

Exact URLs and integrity hashes are in [toolchain.json](../toolchain.json).
The cache is `%USERPROFILE%\.cache\chaindesk-android` on C:, which had 300.2 GiB
free at pre-flight. E: had 33.9 GiB; no sealed desktop resources were removed.
The host had 15.7 GiB RAM, so Go and Gradle parallelism is limited to two workers.

| Component | Candidate / observed version | Current evidence |
| --- | --- | --- |
| Temurin JDK | 21.0.13+9 | Archive SHA-256 verified; `java -version` returned the expected runtime |
| Go | 1.26.11 Windows amd64 | Archive SHA-256 verified; `go version` returned the expected version |
| Gradle | 9.4.1 | Archive SHA-256 verified; `gradle --version` succeeded using the pinned JDK |
| Initial legacy bootstrap | Command-line tools 23.0 | Used during investigation; retired from the rebuild dependencies because its launcher downloads `latest` |
| Android CLI | 1.1.16919734 | Real executable frozen at a versioned Google URL, SHA-256 `4b7d9dc15b3609ac7c952edff6b26fc131dff531a864692903676a886ea9b5b2`; empty-cache download and execution passed |
| Platform tools | 37.0.1 / ADB 1.0.41 | Installed; `adb devices -l` returned an empty device list |
| SDK platform | Android 37.0, revision 2 | Installed using explicit Android CLI package `platforms/android-37.0` |
| AGP / Kotlin | 9.2.0 / bundled 2.2.10 | Both actual builds passed; Kotlin Gradle plugin 2.2.10 was observed in the dependency cache |
| Build tools | 36.0.0 | Installed; `source.properties` reports 36.0.0 |
| NDK / CMake | 28.2.13676358 / 3.31.6 | Installed; package files and actual revisions verified |

The NDK archive was downloaded in bounded parallel ranges after the single
installer connection proved slow. The assembled 748,118,221-byte archive matched
publisher SHA-1 `086bba43ff2f5eb0e387b15c8278bb4e0d89ba1d` and SHA-256
`6bec98ac2354d8a919760889a1a41d020132e5e8cfa1b1fe51610a72c36a466b`.
Android CLI then reused its content-addressed cache and performed normal package
installation. No unverified partial archive was used.

The real CLI executable has a valid Google LLC Authenticode signature. Its
embedded bundle includes its Java payload and runtime; no moving `latest`
launcher is used for rebuilding. SDK provisioning and Gradle builds set
process-local `ANDROID_USER_HOME` to the isolated cache. The initial bootstrap's
shared-user cache was preserved, not moved or deleted.

AGP 9.2 supports API 37.0 and requires Gradle 9.4.1 or later; its documented NDK
default matches this tuple. JDK 21 also satisfies its JDK 17 minimum.
[Official compatibility table](https://developer.android.com/build/releases/agp-9-2-0-release-notes)

The SDK repository now names this platform `android-37.0`; the Gradle project
uses `compileSdk { version = release(37) }`. This preserves the plan's API 37
target instead of silently building against an older API.

The deprecated `sdkmanager.bat` invocation returned exit code 0 while reporting
missing package names and installing only platform-tools. Direct Android CLI
with slash-separated package IDs successfully installed the platform. The
provisioning script therefore uses Android CLI directly and checks each expected
`package.xml`, rather than treating exit code 0 as complete provisioning.

## Native integration findings

- `config.Parse` temporarily mutates package globals while parsing. Probe calls
  are serialized; future live-session validation must share runtime ownership.
- Directly linking `config.Parse` without the executor failed with undefined
  `config.temporaryUpdateGeneral`. The probe now uses
  `executor.ParseWithBytes`, which links the owner of that internal hook.
- `executor.ApplyConfig` returns no error. Its TUN recreation path logs a start
  failure and disables the listener. Returning from ApplyConfig is insufficient
  evidence that a VPN is usable.
- The fixed source exposes `listener/sing_tun.New` with an error result and an
  externally supplied `FileDescriptor`. The Android reference also calls this
  constructor separately. Actual adoption, failure cleanup and descriptor
  lifetime remain device-validation work.
- The fixed source exposes `dialer.DefaultSocketHook`. The A0 loading probe
  attaches a gate that rejects all transports because no VpnService is attached.
  Host tests exercise actual TCP dial attempts; they do not prove Android
  `VpnService.protect` or underlying-network binding.
- `cmfa` excludes Android package-manager discovery in the fixed TUN source;
  `with_gvisor` matches the intended initial native build. Neither tag proves
  actual TUN stack behavior or real two-hop routing.

Sources: [fixed configuration parser](https://github.com/MetaCubeX/mihomo/blob/3f27ef39f8c96bbfbc8b0b9bb10f5c9ef60149d8/config/config.go),
[fixed executor](https://github.com/MetaCubeX/mihomo/blob/3f27ef39f8c96bbfbc8b0b9bb10f5c9ef60149d8/hub/executor/executor.go),
[fixed TUN constructor](https://github.com/MetaCubeX/mihomo/blob/3f27ef39f8c96bbfbc8b0b9bb10f5c9ef60149d8/listener/sing_tun/server.go),
[Android reference TUN wrapper](https://github.com/MetaCubeX/ClashMetaForAndroid/blob/6e635124cc72f8c102b7e76d849c384deba28b3f/core/src/main/golang/native/tun/tun.go).

## Checks already observed

- Host SocketProtector tests exercised real loopback TCP dials: unconfigured,
  denied and stopped gates prevented connections; an allowed gate connected.
  The tests were observed failing before the implementation was added.
- The native probe's blocked-configuration test reproduced an invalid YAML
  sequence (`MATCH` and `REJECT` were separate elements). Quoting the single
  `MATCH,REJECT` rule fixed it; the complete host Go suite then passed.
- `go test -p 2 ./... -count=1` passed and `go vet -p 2 ./...` returned 0 with
  the pinned toolchain and isolated module/build caches. These are host checks.
- The generated Gradle 9.4.1 wrapper ran successfully with the expected JDK and
  pinned distribution checksum, reusing the verified distribution archive.
- All three Android PowerShell scripts passed the PowerShell parser. The
  provisioning script completed with all expected package files present.

The first Android build failed on an unescaped drive separator in the generated
`local.properties`. The build script now uses process-local `ANDROID_HOME`,
preserves existing local properties and rejects conflicting explicit SDK paths.
Only the one-line file generated by this task was removed. No lint baseline or
error suppression was introduced.

## Initial native-loading checkpoint (0.0.1-a0)

| Result | Observed evidence |
| --- | --- |
| Delivered APK | `dist/a0/chaindesk-android-a0-arm64-v8a-debug.apk`, 59,481,711 bytes (56.7 MiB) |
| APK SHA-256 | `70e7ec87e3cdf590f850acbb0cc57eb5e03f99c1a8dc379d9ab17a52766cdade` |
| Identity | `dev.chaindesk.android.debug`, `versionCode = 1`, `versionName = 0.0.1-a0`, min API 26, target API 37 |
| Permissions | Packaged manifest has no requested permissions; no VPN service is declared |
| Signature | Debug signature verified; certificate SHA-256 `99e38fb95631a157b89bbd203d84f0c9d4fde2f08edda565d91d3439976b606a` |
| Android build | `:app:lintDebug :app:assembleDebug` completed successfully after the fixes |
| Lint | 0 errors, 5 non-blocking warnings: two AGP-update notices, ChromeOS ABI, backup extraction rules and application icon |
| Native ABI | Both packaged libraries are AArch64; every ELF LOAD alignment is 16,384 bytes |
| Native linkage | JNI requires `libchaindesk_core.so`; the Go library has the matching SONAME |
| Compiled dependencies | Packaged Go build info contains mihomo v1.19.33 and the upstream MetaCubeX protobuf replacement |
| APK alignment | Build Tools `zipalign -c -P 16 -v 4` passed on the final candidate before publication |
| Independent source build | Source ZIP extracted into a new directory on C: and rebuilt successfully using the already verified tool/dependency caches |
| Independent APK | SHA-256 `f6d398f9a1fd0336e198c1edb79d2bdfe6b54ebf65cee97d3906d5f97f865d34`; byte-identical reproduction is not claimed |

`dist/a0/native-inspection.json` records the packaged library hashes, LOAD
alignments, dependency names and actual Go build information. The source ZIP
contains the Android subproject and its lockfiles, not a complete public-release
redistribution of every dependency; full A5 release materials remain separate.

## Independent review and corrected boundaries

Five important findings were verified and corrected:

1. Go dependency replacements do not propagate from dependency modules. The
   initial binary used standard protobuf; the root module now mirrors the fixed
   mihomo replacement. Go tests/vet passed again and the final packaged binary
   was inspected for the fork's exact version.
2. The actual CLI executable is downloaded from its fixed versioned URL and
   verified by SHA-256. A new empty CLI cache downloaded that exact file and ran
   version 1.1.16919734 successfully.
3. SDK and Gradle user state now use the isolated Android user directory. The
   true CLI's embedded bundle was observed under that directory.
4. A failed build preserves existing local properties. A regression using a real
   PowerShell invocation with a failing external Gradle stub was observed RED
   before the fix and GREEN afterward; sentinel file bytes were preserved.
5. Candidate signature/alignment checks now precede copying the published APK.
   A deliberately invalid APK was rejected without changing the prior package
   or checksum; the same boundary failed before the correction.

One minor improvement remains deferred: provisioning verifies package-file
presence, while exact installed revisions were independently checked for this
checkpoint. A future cache-validation task can automate the revision comparison.

Two other review notes do not change this scope: `ConfigValidated` describes the
fixed parser probe, not a generic firewall/routing validator; `Enable(nil)`
rejects the invalid request without changing an existing callback. The current
probe never enables a platform callback. Future live-session code must use the
documented Block/lifetime boundary before releasing JNI references.

The five lint warnings remain visible. ARM64-only is intentional, and the probe
has no stored user credentials. Product icons, backup rules and any tool-version
change belong to their documented later stages rather than this internal probe.

## Open acceptance inputs

No ADB device was connected at pre-flight, and the maintainer confirmed no phone
is currently available. No dedicated Android two-hop fixture or device-network
observations have been supplied or fabricated.

ARM64 phone loading, TUN ownership, protected native sockets, real SS + SOCKS5
traffic, network changes, 50 start/stop cycles and 16 KiB runtime loading remain
incomplete. Static alignment checks do not substitute for them. A1–A5 are not
released by this document.

## Continued VPN-runtime checkpoint (0.0.2-a0)

The maintainer requested strict, continuous execution. Work stayed in A0;
no subscription/product expansion or A1-A5 acceptance occurred.

The native bridge now accepts a bounded dedicated test fixture, starts one
external-fd TUN/gVisor runtime, protects and binds outer TCP sockets to the
selected physical Network, and serializes native parsing/start/stop. JNI and
Go own copied descriptors while the Android service retains the original.
The runtime allows only TCP to the literal SS front; full-chain DoH uses the
fixed SOCKS5 exit, and other UDP is rejected. This is implementation, not an
observed Android flow. See [the device procedure](a0-device-validation.md).

A fresh runtime review found fd-close ordering, upstream eventfd cleanup,
in-flight generation overlap, startup-before-capture reporting, stale service
cleanup and SOCKS5 authentication downgrade issues. Corrections were applied:

- Drain TCP/DNS handlers before the next generation; close/join the stack before
  closing its device. A real concurrent host handler blocks a next-generation
  start until the old handler finishes.
- Scope platform blocking to a session ID; stale service cleanup cannot block
  the current protector. Keep an established TUN on core/network failure, but
  release setup callbacks and report no capture when establishment never occurs.
- Reject SOCKS5 method downgrade and overlong credentials. The downgrade test
  reproduced an unpatched CONNECT attempt; patched code rejects before CONNECT.
- Apply four hash-pinned source-file corrections to private generated copies of
  mihomo/sing-tun/gVisor, preserving the shared module cache. Go overlays over
  module-cache files were rejected by Go 1.26; temporary modfiles/local copies
  are the actual method. [Patch provenance](../core/patches/README.md) records
  the fd changes, rollback and remaining runtime evidence requirements.

Host checks use `scripts/testCore.ps1`: encrypted SS, authenticated SOCKS5 and
certificate-verified loopback HTTPS; wrong authentication, protect denial and
both hop failures; forbidden outer destinations; UDP socket controls; singleton,
stale stops, resource ownership, task drain, and 50 host lifecycle iterations.
They do not replace independent network isolation or a phone TUN.

An ARM64 test executable is built for the attached/unattached gVisor endpoint
fd regression. It has not run: this Windows host has no usable Linux/WSL or
Android device. The source-level eventfd fix and its compiled test therefore
remain pending device execution, alongside the original A0.3-A0.6 gates.

Android builds use one service-scoped Lint exception for the documented
systemExempted authorized-VPN alternative; no exact-alarm permission was added.
Actual foreground eligibility, JNI callbacks, TUN traffic, fd behavior, minimum
API support, 16 KiB loading and privacy observations are not accepted yet.

The runtime directory's build-manifest.json, native-inspection.json,
source-snapshot.json and SHA256SUMS describe the final delivered files. The old
native-loading directory is unchanged. No desktop source/lockfile, host proxy
setting, public issue, push, merge or public release was modified.

### Historical version-2 delivery evidence

- Preserved package: dist/a0-runtime-v2/chaindesk-android-a0-arm64-v8a-debug.apk,
  64,090,701 bytes, versionCode 2 / versionName 0.0.2-a0.
- APK SHA-256: f480499e8fae995f8bf2ff39894934f5e14ec8e4f969b30cf8f608312ba934ea.
- Final host checks: 14 top-level Go tests and their subcases passed; vet and
  original-cache module verification passed. Two script boundary regressions
  passed after the runtime-output path change.
- Android lint/build passed with 0 errors / 6 warnings. Three are fixed-tool
  update notices (Gradle and two AGP plugins); the others are ARM64-only
  ChromeOS coverage, future backup extraction rules and the missing probe icon.
  Kotlin also reports use of the deprecated allNetworks query. No unsupported
  device compatibility claim or unrelated dependency upgrade was added.
- Packaged ARM64 Go and JNI libraries have 16 KiB LOAD alignment; APK signature
  and ZIP alignment passed. Inspection verified the four patched source files
  against the local module paths recorded in the packaged Go build information,
  as well as the pinned protobuf fork. This is build provenance, not execution.
- Device regression executable: 118,833,984 bytes, SHA-256
  458aea392282c894cbdaa3d3a1a4ed8e795c406bf28dd13f013f1e8a3a575191;
  AArch64 with 16 KiB LOAD alignment, compiled but not executed.
- Source snapshot: 53 Android source/document files plus root LICENSE; no
  .kotlin/Gradle sessions, build directories, signing material or private fixture.
- Independent directory C:/Users/CYF/.cache/chaindesk-android/a0-runtime-rebuild-q6okwtjo
  rebuilt the snapshot using the verified existing caches. Its APK SHA-256 is
  c47ee6c9b26e7ac11c4a77ea7bf46090318f48fc88bcc8a77cfb7283471b63b6.
  Byte-identical output is not claimed. The original native-loading checkpoint's
  recorded checksums were rechecked and remain unchanged.

## Supplemental emulator execution, 2026-11-20

Version 3 selects and validates the physical `activeNetwork` before capture;
the previous `allNetworks` enumeration could choose cellular while Wi-Fi was
the actual default. An Android regression reproduced that mismatch and passed
after the correction. Debug-only local-network permission permits the controlled
host-alias fixture on Android 17; no release permission or host setting changed.

The pinned x86_64 Android 17 image actually reports 16 KiB pages. JNI loading,
the original-versus-patched eventfd regression, full two-hop TLS, 50 TUN cycles,
wrong-password rejection and retained-TUN network-loss behavior ran there.
The instrumentation now waits for VPN to become the app default before probing;
an assertion reproduced the earlier premature-request condition. Corrected
50-cycle counts: 51 authenticated target connections and 51 HTTPS requests;
fd 121 at baseline and 117 at completion. Full observations, failures and
isolation limits are in [the emulator report](a0-emulator-validation.md).

These observations do not execute the ARM64 binary or close phone/vendor,
minimum-API, independent-network, public exit-IP, IPv6-only/NAT64, Private DNS,
UDP/QUIC, process-death or resource-use gates. A0 is not accepted; A1–A5 remain
unstarted. Historical statements above describe their original checkpoints.
