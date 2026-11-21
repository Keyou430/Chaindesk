# A0 VPN runtime validation

Status: implementation/build checkpoint with supplemental emulator execution.
No ARM64 phone acceptance has
been observed. A0 is not accepted and A1-A5 remain gated.

## Package and input

Build with `pwsh -NoProfile -File ./APK/scripts/buildProbe.ps1` from the repository
root. The runtime checkpoint is written to `APK/dist/a0-runtime/`; the earlier
native-loading checkpoint in `APK/dist/a0/` is preserved. Version 0.0.3-a0 uses
the same debug application ID and development signing identity, with versionCode 3.
Version 2 is preserved in `dist/a0-runtime-v2/`. Supplemental x86_64/16 KiB
execution and its limits are in [the emulator report](a0-emulator-validation.md).

Use `testFixtures/a0.example.json` as an input shape. Its documentation addresses
are placeholders, not working servers. Replace them with dedicated test
endpoints; never commit the resulting credentials. A0 supports one SS TCP
aes-128-gcm front and one username/password SOCKS5 exit. Endpoint addresses must
be literal IPv4; hostname bootstrap and subscription import are not implemented.

For a controlled local-network fixture on Android 17, explicitly grant the
debug-only LAN permission before starting the VPN:

```powershell
adb -s <dedicated-device> shell pm grant dev.chaindesk.android.debug android.permission.ACCESS_LOCAL_NETWORK
```

This debug grant does not implement a release local-network permission flow.
Use an explicit device serial for every command when multiple devices exist.
Files larger than 16 KiB, unknown fields, additional JSON values, missing
credentials and invalid ports are rejected. Test fixture files are supplied by
the tester; this probe keeps the imported configuration in memory and does not
persist URI grants or write a profile to app storage.

## Runtime boundaries

- Authorization comes from `VpnService.prepare`; notification permission is
  requested on API 33+. Foreground startup uses `systemExempted` on API 34+.
- The service holds the original nonblocking TUN at 172.29.0.1/30, MTU 1500,
  IPv4 default route and VPN DNS 172.29.0.2. It does not call allowBypass,
  allowFamily(AF_INET6), or exclude the app from the VPN.
- JNI duplicates the borrowed service fd with F_DUPFD_CLOEXEC. Go owns that
  copy on every result. The stack receives a second explicitly owned duplicate;
  stack rollback closes it before the owner copy is closed. Stop releases both
  native copies; only the service releases the original.
- The pinned sing-tun external-fd constructor adopts without post-adoption
  errors. Direct stack construction reuses mihomo's public TUN/DNS handler and
  avoids the full listener constructor's split ownership and platform monitors.
- Every hooked outer socket must pass VpnService.protect and binding to the
  pinned underlying Network. A lost network blocks further native sockets,
  stops the native session and retains an established original TUN. A0 requires
  explicit stop/start afterward; automatic network migration is not implemented.
- The runtime socket gate permits only TCP to the configured literal SS front.
  Stale session IDs cannot block a newer protector. Stop cancels DNS work,
  closes inbound TCP connections and waits for their handlers to finish before
  enabling the next generation; the original core's pending TCP dial timeout
  can delay this drain. It does not run on Android's main thread.
- IPv4 TCP routes to CHAIN-EXIT with dialer-proxy CHAIN-FRONT. Port 53 is
  hijacked into the core DNS handler; DoH uses 1.1.1.1 through CHAIN-EXIT. Other
  UDP is rejected. ICMP forwarding is disabled; ICMP reachability is not evidence
  of a healthy chain. No unmanaged control/listening port is enabled.
- The notification says TUN ready, not connected/verified. No app traffic has
  been accepted merely because the TUN constructor returned successfully.
- An error after successful capture retains the original TUN until explicit
  stop. Revocation, process death and force-stop can release the TUN and restore
  ordinary system routing. This probe does not provide a system kill switch.
- Setup failure before TUN establishment releases its network callback, ends
  the service and reports that the attempt did not capture traffic. It cannot
  truthfully claim blocking before there is a TUN.

The fixed upstream versions need the recorded [minimal A0 patches](../core/patches/README.md)
for required SOCKS5 authentication and gVisor fd cleanup. Use `testCore.ps1`,
not an unpatched raw Go command, for host regressions. The original module cache
remains verifiable. Build the device-only regression executable with:

```powershell
pwsh -NoProfile -File ./APK/scripts/buildCore.ps1 -BuildDeviceTests
```

On an ARM64 Android test device, run the endpoint regression separately:

```text
adb push APK/dist/a0-runtime/chaindesk-core-tests-arm64-v8a /data/local/tmp/chaindesk-core-tests
adb shell chmod 700 /data/local/tmp/chaindesk-core-tests
adb shell /data/local/tmp/chaindesk-core-tests -test.run TestGVisorFiftyEndpointLifetimesDoNotLeakFDs -test.v
```

This executable does not establish a system VPN. Passing its pipe/eventfd
regression would still leave real TUN cycles and phone traffic acceptance open.

The systemExempted manifest uses one scoped ForegroundServicePermission Lint
suppression. AGP 9.2 requires an exact-alarm alternative in its static check;
the documented authorized-VPN alternative requires no alarm permission. The
service checks VPN authorization before starting. Runtime eligibility on the
target device remains unverified. Sources:
[foreground-service eligibility](https://developer.android.com/develop/background-work/services/fgs/service-types#system-exempted),
[VPN authorization and socket protection](https://developer.android.com/develop/connectivity/vpn).

## Required controlled network

Use distinct client, front, exit and controlled HTTPS target environments.
Record addresses and server builds without passwords. Enforce isolation at the
network boundary: client can reach the SS front but cannot reach exit or target
directly; front can reach exit but cannot reach target; exit can reach target.
Give the HTTPS target a valid certificate and a unique, noncached request path.
Use a dedicated test phone/profile because establishing this VPN replaces any
existing VPN in that Android user/profile.

The host Go chain test uses actual SS encryption, authenticated SOCKS5 and
certificate-verified HTTPS with process-local dial restrictions and counters.
Its loopback peers share a host. It does not provide the device/network isolation
or distinct public source-IP evidence required by this section.

## Device execution and evidence

1. Record model, Android/API, ABI and `adb shell getconf PAGE_SIZE`. Install the
   debug APK, load the dedicated fixture, grant notification and VPN permission.
   Observe native self-check output and foreground startup without assuming
   Android 17 or a 16 KiB runtime passed from successful packaging alone.
2. From a separate test app/browser, request the controlled HTTPS target.
   Correlate timestamp/request ID, SS connection/byte counters, authenticated
   SOCKS5 counters, target log/source IP and gateway capture. Both hops must
   increase and the target must see only the exit.
3. Refuse SOCKS5 authentication, stop either hop, deny protect in a dedicated
   fault-injection build, and lose the bound network. For each case, observe no
   new target request, no direct DNS fallback and no unprotected native send.
   The established TUN remains until the stop control is used.
4. Exercise UDP/TCP 53, ordinary UDP/QUIC, dual-stack IPv6 attempts, strict
   Private DNS and app DoH. Capture both gateway and target observations.
   Record IPv6-only/NAT64/464XLAT separately; it is not a supported claim yet.
5. Repeat 50 complete starts/stops and at least one revoke/other-VPN takeover.
   Observe /proc process fd counts, threads, logs and the Android VPN interface
   before/after; every cycle must settle without a growing baseline or residual
   interface. Host pipe tests cannot satisfy this step.
6. Run native loading and the VPN flow on an actual 16 KiB page environment;
   preserve installation/load errors if any. Static ELF/APK alignment is only
   a build prerequisite. Minimum-API coverage also remains required before
   claiming support for API 26.

Store only redacted real observations under `APK/docs/`. A0 can be released
only after the plan's actual-device gates pass. Do not manufacture evidence
rows or promote this checkpoint to the A1 product skeleton.
