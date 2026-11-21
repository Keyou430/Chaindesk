# Synthetic downloader TLS fixtures

These PKCS12 files contain disposable, public test keys. They belong exclusively
to the instrumentation APK and must never be packaged in the application APK.
The fixture password is `download-checks-only`; it protects no real credential.

- `localhost.p12`: self-signed RSA-2048/SHA-256 server certificate with SANs
  `DNS:localhost` and `IP:127.0.0.1`.
- `wrong-host.p12`: self-signed RSA-2048/SHA-256 server certificate with only
  `DNS:wrong-host.invalid`.

Both are valid from 2020-01-01 through 2039-12-27, avoiding dependence on the
fixture generation time. They were generated with the repository-pinned Temurin
21.0.13+9 keytool, using `-J-Dkeystore.pkcs12.legacy` for Android PKCS12 support,
`-startdate 2020/01/01`, `-validity 7300`, `-ext BC=ca:false`,
`-ext KU=digitalSignature,keyEncipherment`, and `-ext EKU=serverAuth`.

DownloaderChecks loads these certificates only into its own SSL contexts and
sets their socket factory only on test-owned HttpsURLConnection instances.
Platform hostname validation remains active. A separate call through the public
downloader API proves the default trust store rejects this synthetic identity.

The local SSLServerSocket checks cover successful HTTPS, three accepted
redirects and a rejected fourth, HTTP downgrade, a 10 MiB decoded gzip boundary
and one-byte excess, malformed gzip, 401 without retry, 503 recovery and retry
exhaustion with observed 1/2-second backoff, trusted-but-wrong hostname rejection,
body cancellation/socket teardown, and a shared 20-second budget spanning two
503 retries plus a continuously streaming body. This suite does not establish
external DNS timing, cross-host credential forwarding, minimum-API compatibility,
OEM behavior, VPN routing, or ARM64 phone acceptance. Root owns actual Gradle
and device execution; generating these assets alone is not a passed check.
