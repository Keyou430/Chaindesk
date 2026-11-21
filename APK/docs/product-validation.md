# Controlled product validation

This procedure is for a dedicated empty debug application on an explicitly
named emulator. It must not consume real subscriptions or wipe a user's app
data. All generated fixture credentials and certificate material stay in the
isolated task cache. Main APKs must contain none of the test CA/private keys.

## Executed checks

On Android17/API37 x86_64 with actual16384-byte pages, these framework
instrumentation groups passed:

| Argument | Actual behaviors checked |
| --- | --- |
| `coreDataChecks=true` | Chinese/supplementary UTF8 JNI roundtrip, generated configuration, sanitized failure codes, installed rule preparation, native CN parse, same-length private geosite tampering rejected before parse |
| `profileChecks=true` | Actual Keystore encryption, no plaintext secrets, tamper/schema/key-loss errors, atomic storage/failure retention, remember/forget/auth-required flags |
| `downloadChecks=true` | Local TLS, default untrusted-CA rejection, correct per-test trust and hostname, decoded gzip10MiB limit, corrupt gzip, bounded HTTPS redirects/retries, cancellation and actual20s total deadline |
| `diagnosticChecks=true` | Closed whitelist, synthetic secret/host/IP markers excluded, deterministic hash, changed profile/session/generation/native selection invalidates preview, tampering rejected |

Runner:
`dev.chaindesk.android.debug.test/dev.chaindesk.android.A0Instrumentation`.
The result must contain `passed=true`; a shell exit code alone is insufficient.

## Reproduce the product flow

1. Provision the optional emulator toolchain and boot the dedicated AVD.
   Build/install x86_64 app and test APKs plus `:trafficProbe:assembleDebug`.
   Keep an explicit `adb -s <serial>` for every device command.
2. Set process-local `GOTOOLCHAIN=local`, `GOPATH`, `GOMODCACHE`, `GOCACHE` to
   the isolated cache, as in `scripts/testCore.ps1`.
3. Generate a fresh private module file with `preparePatchedCore.ps1`, using
   absolute paths for `-ModuleCache` and `-OutputDirectory`.
4. Set process-local `CHAINDESK_PRODUCT_FIXTURE` to a new cache path such as
   `<cache>/product-lab-<id>/fixture.json`. If using the already available host
   transport, set `CHAINDESK_PRODUCT_HTTP_PROXY=127.0.0.1:7897`. No host proxy
   setting changes. Empty means host-network public-health transport.
5. Run the opt-in fixture with the pinned Go executable:

```powershell
& $goExe -C "$repoRoot/APK/core" test -modfile $moduleFile -mod=readonly -p 2 ./engine -run '^TestServeProductEmulatorFixture$' -count=1 -v -timeout 45m
```

6. Wait for the metadata/counters files, then run in another shell:

```powershell
pwsh -NoProfile -File ./APK/scripts/testAppFlow.ps1 -Serial emulator-5580 -Fixture '<cache>/product-lab-<id>/fixture.json' -OutputDirectory '<new-evidence-directory>'
```

`-LifecycleCycles 100` additionally performs100 product connect/verify/stop
iterations and records post-stop fd/thread samples. Every iteration requires
verified Connected, complete Disconnected, removed VPN route and no active
native session. Counter samples are observations, not automatic resource or
ARM64 acceptance. Public probes remain real verified-TLS requests.

`-NetworkSwitches 20` additionally alternates the dedicated guest's Wi-Fi and
cellular preference, requires a newer verified session for each actual
transport change, retains the original VPN network and finally checks an
independent app's TLS traffic. Periodic health monitoring is disabled in this
case so event-driven recovery is tested separately. Guest Wi-Fi/data are
restored enabled, matching this lab's baseline; host networking is untouched.

7. Stop the fixture by creating `<fixture.json>.stop`, wait for its Go test
   process to pass, and shut down only the emulator/ADB processes owned by the
   test. Retain tool caches. Remove no user-owned profiles or host settings.

The host runner verifies UDP delivery without VPN, then holds public health
while an independent ordinary UID tries the controlled TLS target. It releases
health, verifies both-hop/target counters, checks UDP did not arrive with VPN,
switches to a bad front and verifies rollback, cancels another pending probe,
and verifies wrong exit authentication retains a blocked TUN. Result JSON and
raw instrumentation outputs are written separately from synthetic credentials.

The product test checks an initially empty profile and resets its synthetic
profile on normal exit. If the process crashes, its `finally` cannot run.
Inspect/preserve the failure before resetting only that dedicated debug app;
never use this procedure against a profile containing real data.

## Interpretation limits

- Front/exit/target share host loopback. This is not the independent-network
  two-hop/public-source-identity gate.
- Controlled target TLS uses a per-invocation trust store only in the separate
  traffic probe. Product health retains system trust. No device-wide CA is
  installed and no product certificate verification is disabled.
- A local TCP handshake may complete inside the VPN stack before rejection.
  A blocked assertion requires zero target HTTP growth and failed TLS/HTTP.
- UDP `sent=true` is only a local send result. External receiver counter delta
  establishes actual delivery or non-delivery in this controlled run.
- No reachable IPv6 baseline, independent packet capture, ARM64 phone,
  manufacturer power behavior,8h screen-off or7-day use is implied.

## Signing, upgrade and recovery gates

Current builds are debug-signed. After A4 passes, use an owner-controlled
long-term key and verified backup. Record certificate fingerprints without
private-key passwords. With two increasing versionCodes and the same key,
disconnect first, install with replacement, confirm encrypted profile retention,
reenter nonremembered credentials and reverify the chain. A differently signed
package must be rejected by Android; do not replace this check with reinstall.

Recover old code by producing a higher-versionCode build with the same key.
Do not assume version downgrade is permitted. Cross-device credential recovery
is not supported by copying encrypted files. Formal v0.9 delivery and actual
7-day use remain pending the plan's release gates.
