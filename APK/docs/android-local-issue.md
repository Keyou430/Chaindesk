# Build a standalone Android fixed-exit proxy-chain client

Local tracking for the maintainer's 2026-11-19 instruction to execute
`APK/ChainDesk-Android-开发执行计划.md`, including its updated dual-platform
development rules. This follows the existing local tracking convention in
`.chaindesk/remaining-work/M2-M5-local-issue.md`. No GitHub issue or outbound
message is created.

## User outcome

Install a signed Android APK and connect phone applications through one
validated subscription front and one user-configured SOCKS5 exit, without a
computer or root. Delivery is for personal or small-group use; public publishing
and Google Play submission are outside this request.

## Observed limitation

The existing chain command entry points return `PLATFORM_UNSUPPORTED` outside
Windows. The current builder disables TUN and UDP. Windows acceptance does not
establish Android VPN, native library, or device compatibility.

## Authorized implementation scope

- Keep Android source, tests, build scripts and development records in `APK/`.
- Use `codex/android-a0-core-bridge` for the initial validation work; preserve the
  existing uncommitted v1.1 plan edits without attributing them to this task.
- Start with A0: freeze the source and toolchain, build an ARM64 native bridge
  and validation APK, and establish actual TUN and socket-protection behavior.
- Original stage acceptance still requires its documented evidence. The
  maintainer's later continuous-development instruction authorizes feasible
  A1–A5 implementation/preparation while missing hardware/network/time gates
  remain explicitly open; see `development-status.md`.
- Retain the two-hop direction, fixed exit, explicit routing exceptions,
  verified certificates, credential protection, and no implicit direct fallback.
- Do not modify desktop source, desktop lockfiles, sealed Windows evidence,
  existing host proxy settings, or unrelated services.

## Acceptance

The plan's A0–A5 gates define acceptance. A0 specifically requires ARM64 device
evidence for the TUN descriptor, protected native sockets, two real hops,
network changes, repeated start/stop, and supported native page sizes. A package
that merely compiles is not an accepted Android VPN client.

Device access, test-network prerequisites and release signing identity must be
recorded as actual inputs. Missing hardware or observations remain incomplete;
they must not be replaced with simulated success or skipped gates.

Assisted by: GPT-6 (Codex)
