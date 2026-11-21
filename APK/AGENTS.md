# Android development boundaries

The root `AGENTS.md` applies. Android work stays inside `APK/`; do not change
desktop lockfiles, Windows source, sealed evidence, or host proxy settings.

- Read `docs/android-local-issue.md` and the current implementation plan before
  changing scope. The maintainer subsequently authorized continuous feasible
  development across stages without intermediate confirmation. Implementation
  may proceed; actual-device, independent-network and elapsed-use gates must
  pass before their stages are accepted. See `docs/development-status.md`.
- `toolchain.json` pins the portable Windows toolchain and mihomo source.
  Keep caches under `%USERPROFILE%\.cache\chaindesk-android`; do not change
  persistent system PATH or require the desktop build environment.
- Provision with PowerShell 7: `./APK/scripts/prepareToolchain.ps1 -InstallSdk`.
  Android CLI 1.1 uses slash package names; the deprecated sdkmanager batch
  wrapper is not the build entry point.
- Build native code with `./APK/scripts/buildCore.ps1 -Abi arm64-v8a`. The
  Gradle `coreBridge` module invokes the same script before native compilation.
- Run Go checks with `./APK/scripts/testCore.ps1`; it prepares hash-checked
  private module copies containing the versioned A0 fixes, then runs test/vet
  and verifies the original module cache. Raw `go test` omits these fixes.
  Tests must exercise actual relevant
  behavior; version getters and generated JNI glue need no mirrored unit tests.
- Android commands run from `APK/android`: `./gradlew.bat :app:lintDebug
  :app:assembleDebug`. Use the pinned JDK and an isolated `GRADLE_USER_HOME`.
- `./APK/scripts/buildDevelopment.ps1` builds/lints/tests the current development
  package and writes a separate hash-named `dist/development/` directory with
  source and build records. It reuses the existing Windows proxy process-locally.
  `buildProbe.ps1` is for historical A0 source snapshots only; current source
  must be rejected before it can replace sealed A0 artifacts.
- The A0 runtime APK includes a minimal VPN test harness with dedicated IPv4
  SS-to-SOCKS5 fixture input. Supplemental emulator results are recorded in
  `docs/a0-emulator-validation.md`; no ARM64 phone acceptance has passed. It is not
  an accepted VPN client and must not be described as one. Current development
  adds subscription, encrypted profiles and product flows under the later
  continuous-development instruction; it does not satisfy missing acceptance.
- Preserve `dist/a0/`, `dist/a0-runtime-v2/`, `dist/a0-runtime/` and
  `dist/a0-emulator/`. See `docs/a0-device-validation.md` for fd ownership,
  routing limits and mandatory device/network evidence.
- `core/patches/android-a0.json` pins exact source hashes and minimal changes.
  Never mutate the shared Go module cache or desktop mihomo source. Compile the
  additional device test binary with `buildCore.ps1 -BuildDeviceTests`; its
  successful compilation is not a passed Android execution result.
- Record actual library/APK hashes and device observations. Do not substitute
  host Go tests, successful packaging, or emulator results for ARM64 phone VPN
  acceptance, TUN descriptor ownership, or Android socket-protection evidence.
