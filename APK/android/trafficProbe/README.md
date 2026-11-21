# Ordinary-UID lab traffic probe

Independent application ID `dev.chaindesk.trafficprobe.debug`; no shared UID,
VPN service, native code, UI, network binding or socket-protection API. The
framework instrumentation targets its own application. The coordinator adds
`:trafficProbe` to settings, builds the debug APK, and verifies its installed
UID differs from the VPN application's UID. This module is a lab tool.

Arguments: `mode=tcp|udp|ipv6`, literal `host`, `port=1..65535`, optional
`certificate=<base64 DER>`, `requireVpn=true`, and `expectBlocked=true`.
Booleans accept only `true`/`false`; omitted values default to false. Scoped,
bracketed, mapped and ambiguous IP forms are rejected. No DNS lookup is used
for the target.

- TCP connects to the literal host, then verifies TLS for `example.com`, sends
  `GET /a0?nonce=<32 hex characters>` and requires status 200 plus the exact
  `a0-chain-ok` body. Certificate trust is system-default unless a supplied DER
  certificate creates a per-invocation trust store. Hostname verification stays
  enabled. No CA is packaged in this APK or written into product assets.
- UDP sends ASCII `chaindesk-udp:<nonce>` once. `sent=true` means the local
  socket accepted the datagram; delivery/blocking requires host-side counters.
- IPv6 performs a raw TCP connection to the supplied IPv6 literal. An observed
  connection failure establishes no blocking claim without a successful
  separate baseline to that same reachable endpoint.

Result keys are `mode`, `uid`, `vpnActive`, `success`, `expectBlocked`,
`exception` (class only), `connected` for TCP/IPv6, `sent` for UDP, and `nonce`
for TCP/UDP. TCP/IPv6 also return `expectationMet`: with `expectBlocked=true`,
only a failure before TCP connect matches the expectation. TLS/HTTP failure
after connection does not count as a connection block. UDP omits
`expectationMet`; external delivery evidence is required. Instrumentation
finishes with `RESULT_OK` when the TCP/IPv6 expectation matches or the UDP send
call completes. Check the observation keys, not the finish code alone.

TCP connect, TLS handshake and complete HTTP response read each have a maximum
five-second timeout. Headers are limited to 8192 bytes; a single Content-Length
must match the expected body length. Chunked, redirected, malformed, oversized
or truncated responses fail. The reader returns after the complete declared
body instead of waiting for EOF. Logs/results contain no host, URL, certificate
or credential values.

## Explicit-device commands

Examples use `emulator-5554`; substitute the intended serial in every command.
Use actual controlled fixture ports and the fixture's exported DER certificate.
Do not point tests at user subscriptions or credentials.

```powershell
adb -s emulator-5554 install -r -g APK/android/trafficProbe/build/outputs/apk/debug/trafficProbe-debug.apk
adb -s emulator-5554 shell pm grant dev.chaindesk.trafficprobe.debug android.permission.ACCESS_LOCAL_NETWORK
adb -s emulator-5554 shell cmd package list packages -U dev.chaindesk

# Runtime fixture input only; this DER is not an APK asset.
$certificate = [Convert]::ToBase64String([IO.File]::ReadAllBytes('E:/controlled-fixture/target.der'))
adb -s emulator-5554 shell am instrument -w -r -e mode tcp -e host 10.0.2.2 -e port 19443 -e certificate $certificate -e requireVpn true dev.chaindesk.trafficprobe.debug/dev.chaindesk.trafficprobe.TrafficInstrumentation

# Run once without VPN, then again with the controlled VPN active; correlate
# each returned nonce with the UDP receiver's separate counters.
adb -s emulator-5554 shell am instrument -w -r -e mode udp -e host 10.0.2.2 -e port 19053 dev.chaindesk.trafficprobe.debug/dev.chaindesk.trafficprobe.TrafficInstrumentation
adb -s emulator-5554 shell am instrument -w -r -e mode udp -e host 10.0.2.2 -e port 19053 -e requireVpn true dev.chaindesk.trafficprobe.debug/dev.chaindesk.trafficprobe.TrafficInstrumentation

# Set this to a reachable controlled IPv6 endpoint. The baseline must connect
# before the VPN run can support an IPv6-blocking observation.
$ipv6Host = '<controlled-reachable-ipv6>'
adb -s emulator-5554 shell am instrument -w -r -e mode ipv6 -e host $ipv6Host -e port 443 dev.chaindesk.trafficprobe.debug/dev.chaindesk.trafficprobe.TrafficInstrumentation
adb -s emulator-5554 shell am instrument -w -r -e mode ipv6 -e host $ipv6Host -e port 443 -e requireVpn true -e expectBlocked true dev.chaindesk.trafficprobe.debug/dev.chaindesk.trafficprobe.TrafficInstrumentation
```

`ACCESS_LOCAL_NETWORK` appears only in the debug manifest for the emulator
gateway UDP baseline. No product manifest or permission is changed.

## Host checks

`src/test/java/dev/chaindesk/trafficprobe/TrafficClientChecks.java` is a plain
Java check runner with no test-framework dependency. Compile it together with
`TrafficClient.java` using the pinned JDK and `--release 17 -Xlint:all -Werror`,
then run its main class with two temporary PKCS12 paths: a self-signed fixture
whose SAN is `example.com`, and another whose SAN is `wrong.example`. Both use
alias `fixture` and password `traffic-fixture`. These generated host-test keys
stay under ignored `build/` and do not enter the application.

The checks exercise real loopback TCP/TLS/UDP/IPv6, strict target validation,
system-trust rejection, wrong-host rejection, malformed HTTP, and a server that
keeps the connection open after the complete response body. They do not replace
Android, VPN, UID, emulator namespace, or physical-phone acceptance.
