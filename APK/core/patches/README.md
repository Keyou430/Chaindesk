# Pinned Android A0 corrections

`android-a0.json` describes exact source SHA-256 values and unique textual
changes for mihomo v1.19.33, sing-tun v0.4.28 and gVisor
v0.0.0-20260922041103-e2cbcd6e7400. Original versions remain pinned in go.mod.
This is a locally patched build, not an unmodified upstream binary.

`preparePatchedCore.ps1` verifies each original source file, copies the three
affected modules into a private generated build directory, applies only the
listed edits and creates a temporary modfile with local replacements. Original
module-cache files and desktop sources are never edited. Original licenses
remain in the private module copies. Source material for a public release is
still an A5 obligation.

Go 1.26 rejects overlays replacing module-cache files. The initial overlay
experiment was rejected by the compiler; it is not the build method. Builds
and checks use the same temporary-modfile preparation path.

Changes:

- SOCKS5 authentication: reject a server selecting no-authentication when the
  client offered username/password only. A real net.Pipe handshake reproduced
  the downgrade and now returns ErrAuth before writing CONNECT. Fixture
  validation separately enforces the protocol's 255-byte credential limits.
- gVisor readv dispatcher: release the stop eventfd exactly once. Serialize
  signaling with a stop-once guard so a later endpoint detach cannot write to
  the released/reused fd. Release buffers and processors once as well.
- gVisor endpoint: detach, wait for readers, then release dispatchers even when
  the endpoint was constructed but never attached. Repeated Close is safe for
  the A0 readv TUN endpoint.
- sing-tun: close a constructed endpoint if gVisor stack setup fails.

The application's own stop sequence drains its TCP/DNS handler group, closes
the stack (which joins readers), then closes the TUN device. Native copies and
the service's original fd have separate owners. The real runtime socket gate
accepts only TCP to the literal SS front; it is not test-only isolation.

Host tests cover the authentication, destination and handler-generation
boundaries. `fd_linux_test.go` supplies real Linux/Android eventfd/pipe endpoint
checks for unattached and attached lifetimes, repeated 50 times. An ARM64 test
binary is compiled for device execution, but these Linux/Android tests have
not run on the Windows host. No leak-free Android claim is made without that
execution and the plan's actual TUN cycles.
