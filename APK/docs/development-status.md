# Android development status

Status date: 2026-11-22 (Asia/Shanghai). Current development version:
`0.1.0-dev.1`; the default versionCode is `4`, with increasing overrides for
controlled upgrade validation. VersionCodes `5` and `6` have separate x86_64
development packages; versionCode `6` also has a completed ARM64 development
package and an independent rebuild from its archived source.

The maintainer requested continuous implementation and a buildable validation
package without a phone. Feasible development continues under that instruction;
stage acceptance still requires the original plan's evidence. The user-owned
plan is unchanged (SHA256
`673a598e8efa22fe0a751df914d963e9e58212d6b5c30d30e3300cc12c49b1f3`).

| Stage | Implementation/evidence available | Acceptance still missing |
| --- | --- | --- |
| A0 native integration | Pinned core/toolchain; JNI loading, protected/bound native sockets, retained TUN; actual x86_64 Android17/16KiB execution; prior endpoint and full-TUN lifecycle evidence | ARM64 execution, minimum API/vendor coverage, independent front/exit networks, remaining device/network/resource gates |
| A1 session/VPN | Application-owned generation controller; serialized worker; shared notification/UI state; current-session verification before traffic activation; network invalidation and retained capture | Full Android permission/network/process-death matrix on supported phones |
| A2 offline configuration | Bounded UTF8/YAML importer; stable node IDs; eight offline candidate parses; two closed routing presets; pinned geodata verified before parse; Keystore AES256-GCM atomic profiles; bounded HTTPS downloader; Chinese offline forms | Actual phone/provider compatibility, interruption/recovery and cross-device observations beyond the executed emulator cases |
| A3 product flow | Directed TLS/204 checks, two-source IPv4 observations/expected-IP checks; sequential candidates/rollback/cancellation; optional monitoring; diagnostic preview/hash/export; independent-UID controlled TCP/UDP flow executed | Full UI/SAF/accessibility and repeated network-recovery coverage; actual self-use acceptance |
| A4 reliability/privacy | Host failure-path tests; final v4 APK passed100 product lifecycle iterations and20 guest Wi-Fi/cellular changes with independent-UID TCP; SS128/256-GCM, SS-chacha20-ietf-poly1305 and VMess TCP controlled flows passed; unverified/wrong-auth traffic blocked | Remaining four TLS protocol combinations; both presets' complete DNS/IPv6/PrivateDNS matrix; physical-phone network switches;8h screen-off; throughput/power/vendor tests |
| A5 delivery preparation | ARM64/x86_64 development packages; independent archived-source ARM64 rebuild; signature/ABI/alignment/content/source checks; historical A0 output guard; actual debug-signed4→5→6 upgrades preserving encrypted profiles, chain/DNS revalidation, wrong-signer/downgrade rejection | Release gate; complete third-party redistribution review/materials; owner-controlled long-term key/backup and its upgrade matrix; actual7-day use and v0.9 acceptance |

No stage requiring these external observations is marked accepted. The debug
package remains an internal development candidate. Public publishing and
Google Play submission are outside this request.

## Current implementation

- Kotlin/Compose UI and an Application-owned model; one StateFlow session
  controller drives the Activity and foreground notification. Native start
  produces `Probing`, never `Connected`.
- One JNI/Go core and one retained original Android TUN; per-session owned fd
  duplication and probe cancellation/drain. All product traffic is suspended
  until the current exit probe verifies and native activation succeeds.
- A 30s total connection budget, at most three sequential candidates, a
  reserved rollback budget, and immediate generation invalidation on cancel.
  Monitoring is optional and targets60s intervals; Android sleep can delay it.
- HTTPS204 and IPv4 probes use the selected native proxy directly with TLS
  verification and no redirects/system HTTP proxy fallback. A public IP-service
  failure without an expected-IP requirement is an observation failure only.
- Import strips subscription routing/providers/scripts; unknown/ineligible
  nodes cannot inject chain references. File/YAML/decompressed download limits
  are10MiB,5000 proxies and32 YAML nesting levels.
- `chinaDirect` requires immutable rules version
  `meta-rules-dat-19f81c0fda5b505c0b186bcbc67cc5f8226aafd3`. Missing or altered
  private rule files fail before the native parser can download replacements.
- Encrypted profiles and last-verified configuration use separate app-owned
  Keystore aliases in no-backup storage. Forgetting or not remembering secrets
  preserves the authentication-required flag. UI drafts survive rotation in
  memory, without secrets in saved instance state.
- Diagnostic JSON is generated from a closed whitelist. Export requires the
  current preview/hash and matching profile/session revision. Unknown errors
  cannot carry arbitrary text into the export.

## Observed regressions and corrections

| Observed failure | Correction and evidence |
| --- | --- |
| TUN reader could start before chain health verification | Product starts suspended; only a verified current native session activates. Host regression RED→GREEN; actual independent-UID request while Probing failed TLS with no target-counter increase. |
| Stopping while a canceled probe finished deadlocked on coreMutex | Per-session atomic verification token removes the finishing probe's mutex dependency; stop drains owned probes. Actual isolated regression RED→GREEN; full host checks passed. |
| Instrumentation read Application.model before Application.onCreate | Test runner waits for main-thread idle before reading model; controlled flow then passed. This was a harness race. |
| Rapid disconnect/reconnect crashed after one lifecycle iteration | Android reported ForegroundServiceDidNotStartInTimeException. Stop had published Disconnected before the old service exited. Completion now publishes after service destruction/resource cleanup;100 iterations subsequently passed. |
| Light status icons were unreadable over the light UI | Activity uses edge-to-edge system-bar appearance. Actual connected screenshot reviewed. |

A fresh integrated review found additional ownership/persistence defects.
Actual pre-fix emulator regressions reproduced excluded historical rollback,
same-generation invalid commits, retained historical credentials and cancel
before CONNECT handling. All four then passed, together with staged cancel,
deadline, network-loss, activation-failure and current-policy persistence cases.
Native activation and Verified state now share one controller operation;
both encrypted slots restore when the commit aborts. Historical fronts must
remain eligible in the current subscription and never replace current policy.

A same-process chinaDirect→allProxy host regression also reproduced inherited
split-routing socket permissions. Explicit geodata-mode in every generated
preset fixed it; full host test/vet/module verification passed. Queued network
work now checks its generation, native session and physical binding before
stopping a core. Network events trigger bounded recovery independently of the
optional monitor; Degraded has no periodic reconnect loop. Required identity
failures block; optional IP observations alone do not switch the exit. The
final10 controller JVM tests and final v4 network execution passed; see the
artifact-bound observations below.

The final v4 APK SHA256 is
`3da3e2650a2991dcd1de344ffe77a2271b448a723bfc62e9bec6b580e396ab0c`.
It passed100 verified connect/stop cycles and20 guest Wi-Fi/cellular transport
changes with monitoring disabled, newer verified sessions, retained VPN and
independent-client TLS after recovery. Native goroutines were4–5 (idle4),
FDs151–157 (idle154), and threads34→41, largely Android Binder-pool names.
These are observed emulator samples, not8h resource or physical-device acceptance.

Controlled SS aes-128-gcm, SS aes-256-gcm, SS chacha20-ietf-poly1305 and VMess
TCP/AEAD auto/alterId0 passed on this same APK. Each covered the independent-UID
flow, paused/wrong-auth blocking, UDP baseline/blocking, rollback and cancellation.
Other TLS combinations remain open; no test CA is added to the product.

## Debug upgrade execution on 2026-11-22

The original v4 encrypted profile and history still matched their saved SHA256
markers when this continuation began. The old lab server had timed out. A
cache-only Go test overlay restored its original front/exit ports and synthetic
credentials, leaving repository sources, encrypted profile bytes and Android
Keystore identity intact. The restored v4 baseline reverified the chain and an
independent-UID DNS query before any installation.

Android then rejected a valid APK signed by a different temporary key with
`INSTALL_FAILED_UPDATE_INCOMPATIBLE`; version and UID remained unchanged.
Same-signature4→5 and5→6 installations passed. Each retained the exact encrypted
profile/history bytes, decrypted the profile, reverified the native chain and
passed ordinary-UID DNS through the VPN. An attempted5→4 installation returned
`INSTALL_FAILED_VERSION_DOWNGRADE`. The final v6 verification cleared only the
owned synthetic profile/marker. Temporary negative-test signing secrets were
deleted. These are debug-identity observations, not release-key acceptance.

Package hashes:

- v5 x86_64: `94e257b296add6754eeed4533e582fac61461d9182a6955fda1d222c7fe33df3`.
- v6 x86_64: `d55ce05417d19d9f0f4d454559e8cfc1efff8946afa9e39778955aedfa95c390`.

The first negative-signing attempt failed in the test helper before installing
anything: apksigner consumed a one-line password file twice. Omitting the
redundant PKCS12 key-password argument corrected the helper; the actual
wrong-signature Android rejection was then observed. Failed output is retained
with the other local evidence, rather than attributed to a product defect.

## ARM64 development delivery on 2026-11-22

Local directory: `APK/dist/development/v6-arm64-v8a-f94f312a4db9/`.
The75.47MiB APK has SHA256
`f94f312a4db904368fd671242b1add026359f714e97897a81ec9ba332c10ac9c`.
Its as-built source ZIP has SHA256
`a44b8dc4481dac8405f1462c8f5c9d5e23dd86d665e3437b57a837e9b4881f55`.
The build passed10 controller JVM tests, Lint with0 errors and20 warnings,
signature, expected native-library/ABI checks, ZIP/ELF16KiB alignment and
pre/post-build source-hash comparison. Remaining warnings were retained;
the build is not described as warning-free.

All147 archived source files matched the source-input manifest after extraction
to a fresh cache directory. A build from that directory, reusing the pinned
tool/dependency caches, also passed the same packaging checks. Its APK SHA256 is
`2dcd29a696d7319499511f481e943aa2f56b161ebebd0d339916a8fab10b5e75`.
Binary hashes differ; byte-identical reproduction is not claimed.

Fresh host checks passed38 top-level tests plus subcases, vet and original
module-cache verification. Two opt-in server tests were skipped by that general
suite; the restored upgrade fixture was executed separately and stopped
gracefully with a passing Go test. The installed v6 x86 APK was pulled and its
hash matched the tested upgrade candidate. The dedicated emulator was shut
down; no device remained attached. Pre-existing ADB service/cache were retained.

Landscape profile/navigation was visually inspected. The actual injected
synthetic draft survived Activity recreation; the attempted full URL injection
was incomplete, so no full-URL entry claim is made. A transient null accessibility
root during rotation required a later dump. Rotation mode was restored to
`free`, and the unsaved draft was discarded by stopping the owned debug app.
This smoke observation does not establish TalkBack or complete UI acceptance.

As-built archives/checksums remain immutable. Final delivery records and these
documentation-only status updates supplement those snapshots. Prior sealed
A0 packages and the original plan hash were checked unchanged. No desktop
source, host proxy setting, commit, push or public release was changed.

Known deferred minor: duplicate variants with the same canonical ID can retain
an ineligible first occurrence and discard a later eligible occurrence. An
affected import may have fewer candidates; it does not add a bypass route.

## Evidence boundaries

The controlled product fixture places front, exit, TLS target and UDP collector
on host loopback. The target uses a documentation-only IP alias to avoid the
declared private-LAN DIRECT rule. VPN UID10231 and traffic UID10233 are distinct.
Successful traffic correlates an independent app's TLS/HTTP result with both
controlled-hop counters and the target request counter.

The public-health fixture permits only the fixed four HTTPS probe hosts and
the fixed DoH IP; it uses the existing loopback HTTP CONNECT transport. Native
TLS verification stays enabled. This additional lab transport means public
egress identity and independent network isolation are not accepted.

The blocked TUN can complete a local TCP handshake inside gVisor before closing
the flow. Blocking observations therefore require failed target TLS/HTTP and
zero target-counter increase; a local `connected=true` is not delivery.

An early guest SystemUI service ANR and a launcher crash are retained separately
from the observed ChainDesk foreground-service crash. Emulator results cannot
substitute for ARM64/vendor/resource acceptance.

## Source synchronization checkpoint

The maintainer requested remote source synchronization on2026-11-22 and deferred
physical-device validation until the required equipment is ready. The Android
checkpoint and supplied v1.1 plan updates belong to
`codex/android-a0-core-bridge` in `Keyou430/Chaindesk`; stage acceptance remains
open. APKs, generated outputs, private caches and raw local evidence remain
outside Git. Keep the local delivery directory and plan ledger for resumption.

Android Git attributes preserve the exact bundled rule/license and upstream
provenance bytes across checkouts. Generated rule databases are binary diffs;
unchanged upstream provenance copies retain their original whitespace.

## Remaining execution order

1. Use the [Chinese installation and upgrade guide](install-upgrade.zh-CN.md)
   with a supported ARM64 test phone; finish permission, process-death,
   accessibility and vendor observations against its actual installed APK.
2. Finish the third-party source/license materials before redistribution;
   current native and Android dependency reports are inventories, not closure
   of the release gate.
3. With real hardware and controlled network/TLS resources, execute the open
   A0–A4 matrix. Remove unverified protocols/devices from any release support
   statement rather than calling them supported.
4. After release gates pass, establish the owner-controlled signing identity,
   verify increasing-version same-signature upgrades and recovery, then record
   actual7-day use. No build script or report can replace elapsed observations.
