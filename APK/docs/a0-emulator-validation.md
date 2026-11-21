# A0 supplemental Android emulator evidence

Date: 2026-11-20, Asia/Shanghai. This is supplemental x86_64 execution,
not ARM64 phone acceptance. A0 remains open and A1–A5 have not started.

## Executed environment

| Input | Observed value |
| --- | --- |
| Emulator | 37.2.13, build 16740729; WHPX; dedicated headless AVD |
| System image | `system-images/android-37.0/google_apis_ps16k/x86_64@8` |
| Android / ABI | Android 17 / API 37, x86_64 |
| Actual page size | `getconf PAGE_SIZE`: 16384 |
| Fingerprint | `google/sdk_gphone16k_x86_64/emu64xa16k:17/CE2A.260420.051.A3/16571990:userdebug/dev-keys` |
| Application | Ordinary app UID; no root; debug signature |
| Native core | mihomo v1.19.33; Go 1.26.11; `android/amd64` |

The downloaded image was 2,081,411,720 bytes. Publisher SHA-1:
`3e04f6c168a573640e63fdd8afd71a3c4d947f64`; observed SHA-256:
`94936d2fb7d8bd1d0c2411510ca7e06ce93184695cad8a43fea036660ffb972d`.
Optional provisioning pins are in `testFixtures/emulator-toolchain.json`.
No host virtualization, proxy or firewall settings were changed.

## Fixture and observation method

An opt-in host Go fixture creates a Shadowsocks front, authenticated SOCKS5
exit and certificate-verified HTTPS target. Each service has a separate
listener, but all use the host loopback network. This is not independent
network isolation and does not establish public exit identity.

The front accepts only the configured exit destination; the exit accepts only
the controlled target. Counters distinguish accepted front connections,
decrypted exit destinations, authenticated target connections, HTTPS requests
and actual `socks5.ErrAuth` rejections. Background Android traffic may increase
front counters but is rejected by the fixture's destination restrictions.
The instrumentation validates the target certificate and hostname; it does not
disable TLS verification. Passwords are generated in the task cache and are
not packaged in the APK or source snapshot.

Android 17 target-37 local-network restrictions initially caused TCP timeouts
to the emulator's host alias `10.0.2.2`. Only the debug manifest declares
`ACCESS_LOCAL_NETWORK`; the owned emulator explicitly grants it. Platform
protection was not disabled. This is a fixture requirement, not a release
permission flow. See the [official local-network permission guide](https://developer.android.com/privacy-and-security/local-network-permission).

## Results and corrections

- The original gVisor descriptor regression failed cycle 0 with 9 descriptors
  versus baseline 8. The pinned corrections passed 50 attached and detached
  cycles on Android. These pipe/eventfd tests are distinct from real TUN tests.
- JNI loading and closed native configuration validation passed on the actual
  16 KiB guest. Malformed fixture input was rejected.
- With Wi-Fi as the default and validated cellular also present, enumerating
  `allNetworks` selected cellular. The Android regression failed because VPN
  physical transport differed from the pre-VPN default. Selecting and validating
  `activeNetwork` fixed this; the controlled two-hop HTTPS regression passed
  with process fd count 121 before and after the cycle.
- An incorrect exit password produced 22 explicit authentication rejections,
  no additional target connections and no additional HTTPS requests.
- After one healthy HTTPS request, disabling guest Wi-Fi and cellular stopped
  the native session while retaining the original VPN/TUN. Restoring Wi-Fi did
  not resume forwarding; HTTPS remained blocked until explicit stop. The
  target count increased only for the initial healthy control request.
- An earlier version-2 real-TUN run completed a warmup and 50 cycles: 51 target
  requests, 51 exit-to-target connections; fd count 157 initially and 156 at
  completion. Its broad handshake-error counter was subsequently corrected;
  that historical counter does not establish an authentication rejection.
- Later fast lifecycle runs reset connections after 34 and 29 successful
  requests. A diagnostic assertion reproduced HTTPS starting while the app's
  default network was still physical, despite the native session being active.
  The test now waits for the app default to become VPN and retains the assertion
  before every HTTPS attempt. Final corrected-run results are recorded below.

The corrected version-3 run completed a warmup plus 50 full TUN cycles:
front-to-exit connections +168, authenticated exit-to-target connections +51,
HTTPS requests +51, authentication rejections +0, front errors +0. Process fd
count started at 121 and ended at 117; no surviving descriptor target changed.
Each HTTPS attempt first required the app default network to be VPN. This
corrects the premature-probe condition without adding retries or weakening the
fd assertion. Historical reset runs are retained as failed runs, not passes.

## Final clean version-3 package

Tested APK SHA-256:
`fb1e87fada1aaec8dd7840f5a66122212c067095112212c5107fc4c3cc81368d`.
The clean package passed lint (0 errors, 6 warnings), debug signature, ZIP
alignment and native 16 KiB LOAD alignment checks. Packaged module provenance
matched all four pinned source corrections; temporary diagnostic code was absent.

| Final APK scenario | Service observations | Instrumentation result |
| --- | --- | --- |
| Warmup + 50 TUN cycles | Front-to-exit +181; exit-to-target +51; HTTPS +51; auth rejections 0; front errors 0 | Pass; fd121 ->117; no changed surviving descriptor targets |
| Default Wi-Fi with cellular also available | Exit-to-target +2; HTTPS +2 | Pass; VPN physical transport matches pre-VPN default; fd121 ->121 |
| Incorrect exit password | Actual authentication rejections +22; exit-to-target 0; HTTPS 0 | Pass; HTTPS blocked |
| Network loss and restoration | Exit-to-target +1; HTTPS +1, both from healthy control | Pass; native stopped; original TUN retained; restored network still blocked until explicit stop |

Raw result bundles and counters are retained in `dist/a0-emulator/evidence/`.
The installed package hash, image, page size and instrumentation hash are bound
to those results in `dist/a0-emulator/emulator-execution.json`.

## Repeating the controlled test

Use PowerShell 7 and the pinned isolated toolchain. Provision the optional
emulator with `scripts/prepareEmulator.ps1`, then launch its dedicated AVD with
`ANDROID_AVD_HOME` set to the returned directory. Do not modify host features
if acceleration is unavailable. Select its explicit ADB serial and verify
`getconf PAGE_SIZE` before recording any 16 KiB execution claim.

Build the application with `scripts/buildProbe.ps1 -Abi x86_64`. To build the
framework-only test APK, use the same process-local JDK/SDK/Gradle directories:

```powershell
$cacheRoot = Join-Path $env:USERPROFILE '.cache/chaindesk-android'
$env:JAVA_HOME = Join-Path $cacheRoot 'unpacked/jdk-21.0.13+9/jdk-21.0.13+9'
$env:ANDROID_HOME = Join-Path $cacheRoot 'sdk'
$env:ANDROID_USER_HOME = Join-Path $cacheRoot 'android-user'
$env:GRADLE_USER_HOME = Join-Path $cacheRoot 'gradle-cache'
& ./APK/android/gradlew.bat -p ./APK/android --no-daemon -PprobeAbi=x86_64 :app:assembleDebugAndroidTest
```

In a separate PowerShell process, start the opt-in fixture. Retain the generated
metadata path locally; it contains synthetic credentials and must not be shared:

```powershell
$cacheRoot = Join-Path $env:USERPROFILE '.cache/chaindesk-android'
$env:GOTOOLCHAIN = 'local'
$env:GOPATH = Join-Path $cacheRoot 'go-path'
$env:GOMODCACHE = Join-Path $cacheRoot 'go-mod'
$env:GOCACHE = Join-Path $cacheRoot 'go-build'
$labDirectory = Join-Path $cacheRoot ('emulator-lab-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $labDirectory | Out-Null
$env:CHAINDESK_A0_FIXTURE = Join-Path $labDirectory 'fixture.json'
$env:CHAINDESK_A0_FIXTURE
$moduleFile = & ./APK/scripts/preparePatchedCore.ps1 -ModuleCache $env:GOMODCACHE -OutputDirectory (Join-Path (Get-Location) 'APK/android/coreBridge/build/patchedCore')
& (Join-Path $cacheRoot 'unpacked/go-1.26.11/go/bin/go.exe') -C ./APK/core test -modfile $moduleFile -mod=readonly -p 2 ./engine -run '^TestServeA0EmulatorFixture$' -count=1 -v -timeout 45m
```

Wait for both metadata and its `.counts` file. Install the app and test APK
with the explicit serial; grant the debug LAN permission as documented in
[the device procedure](a0-device-validation.md). Load the metadata locally and
pass base64-encoded UTF-8 JSON for its `fixture` object, its `certificate` and
numeric `targetPort` as instrumentation arguments. Example after assigning
`$adb`, `$serial` and `$metadataPath` to the dedicated inputs:

```powershell
$metadata = Get-Content -LiteralPath $metadataPath -Encoding UTF8 -Raw | ConvertFrom-Json
$fixtureJson = $metadata.fixture | ConvertTo-Json -Depth 8 -Compress
$fixtureArgument = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($fixtureJson))
& $adb -s $serial shell am instrument -w -e cycles 50 -e fixture $fixtureArgument -e certificate $metadata.certificate -e targetPort $metadata.targetPort dev.chaindesk.android.debug.test/dev.chaindesk.android.A0Instrumentation
```

Require `passed=true` and correlate the `.counts` before/after delta: a warmup
plus 50 cycles must add exactly 51 authenticated target connections and 51
HTTPS requests. `checkDefaultNetwork=true` adds the physical-transport assertion.
For the negative case, change only the exit password in the local metadata and
use `expectHttpsFailure=true`; require actual authentication rejections and zero
target increases. `networkLoss=true` temporarily toggles Wi-Fi/data only on a
`ranchu` emulator and restores both in `finally`; require exactly one healthy
target request. Stop the fixture by creating its `.stop` file, and stop only the
owned emulator after evidence collection.

## Remaining acceptance limits

ARM64 phones, vendor behavior, minimum supported Android, independently isolated
front/exit/target networks, public exit identity, IPv6-only/NAT64, Private DNS,
non-DNS UDP/QUIC, resource use and process-death protection remain unaccepted.
Static ARM64 ELF alignment and x86_64 16 KiB execution are separate evidence.
No successful emulator run closes these plan gates.
