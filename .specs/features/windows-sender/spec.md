# Windows Sender Specification

## Problem Statement

OpenDisplay's sender role (the machine whose desktop gets extended) exists today only on macOS. Windows users with a spare iPhone or iPad have no first-party way to use it as a second monitor through OpenDisplay — the wire protocol (`PROTOCOL.md`) was written transport- and platform-agnostic specifically so a Windows sender could exist, but nobody has built it. This spec defines a first-party Windows sender app that plays the same role `Mac/OpenSidecarMac` plays today, speaking the same `pv 3` wire protocol against the unmodified iOS/iPadOS receiver app.

## Goals

- [ ] A Windows 11 user can install the app, install a signed virtual-display driver once, and turn a paired iPhone/iPad into a true extended display — over WiFi or over USB.
- [ ] Touch input (tap-to-click, drag, two-finger scroll) and live cursor forwarding work from the phone to the Windows desktop, matching the current Mac↔iOS experience.
- [ ] The Windows sender interoperates with every receiver already in the field (any `pv` from 1 to 3) per the compatibility matrix in `COMPATIBILITY.md`, with zero changes to the iOS/iPadOS receiver app.
- [ ] The virtual-display driver installs and loads on stock Windows 11 (Secure Boot on) without the user disabling test-signing.

## Out of Scope

Explicitly excluded. Documented to prevent scope creep.

| Feature | Reason |
| --- | --- |
| Multiple simultaneous devices, each its own display | Mac already ships this (#8); Windows P1 is deliberately single-device, deferred to P2 (user decision). |
| Auto-update mechanism for the Windows app | Sparkle is macOS-only; no Windows equivalent wired in P1. Ships as a manual download, same as the Mac app before Sparkle landed. |
| Apple Pencil / proximity input | Still open on Mac itself (#4); Windows P1 mirrors Mac's *currently shipped* input feature set, not the Mac roadmap. |
| Right-click / multi-touch gestures | Still open on Mac itself (#5); same rationale. |
| Hardware keyboard passthrough | Still open on Mac itself (#6); same rationale. |
| Resolution / quality settings | Still open on Mac itself (#9); same rationale. |
| HEVC encoding | Still open on Mac itself (#10); same rationale. |
| Audio forwarding | Explicitly out of scope project-wide per README FAQ. |
| Encrypted WiFi transport / pairing code | Still open project-wide (#16); wire protocol `pv 3` has no auth by design — the Windows sender inherits this, it does not fix it. |
| Changes to the iOS/iPadOS receiver app or to `PROTOCOL.md` | This feature adds a new sender only; it consumes the existing spec, it does not evolve it. |
| CI/release pipeline, installer signing automation, code-signing certificate procurement | Implementation mechanics for Design/Tasks phases, not a functional requirement of this spec. |
| macOS-as-receiver or Linux-as-sender | Different, unrelated roadmap items (#17, community Linux sender already exists independently). |

---

## Assumptions & Open Questions

Every ambiguity is resolved or recorded here — nothing is left silently unclear.

| Assumption / decision | Chosen default | Rationale | Confirmed? |
| --- | --- | --- | --- |
| Transport scope for P1 | Both WiFi and USB ship in P1, not staged | User decision — full parity with the Mac sender's two transports from day one. | y |
| Virtual display mechanism | A first-party IddCx (Indirect Display Driver) kernel driver, built and shipped by this project | User decision — matches the "true extension" bar `CGVirtualDisplay` sets on Mac; mirror-only was rejected. | y |
| Driver signing | Microsoft attestation signing via Partner Center, using a purchased OV/EV code-signing certificate | User decision — installs cleanly on stock Windows 11 with Secure Boot on; rejected unsigned + testsigning due to end-user friction (reboot, watermark). | y |
| Sender application language/runtime | Rust for the application (capture orchestration, encode pipeline, networking, tray UI) | User decision. | y |
| IddCx driver toolchain | C++ against the WDK (Microsoft's own IddCx sample toolchain), separate from the Rust application, talking to it over a device-interface/IOCTL boundary | User decision — kernel-mode Rust (`windows-drivers-rs`) is still experimental and would jeopardize the already-chosen attestation-signing path, which assumes the standard WDK toolchain. | y |
| USB transport dependency | Bundle libimobiledevice's usbmuxd-compatible client instead of requiring Apple's own driver | User decision — LGPL-2.1, compatible with this project's GPL-3.0; avoids forcing users to install iTunes/Apple Devices first. Accepted trade-off: may lag brand-new iOS pairing/lockdown changes. | y |
| Multi-device support | Single connected device only in P1 | User decision — smaller MVP surface; multi-device becomes a P2 story. | y |
| Minimum Windows version | Windows 11 only | User decision — IddCx 1.x and Windows Graphics Capture are mature on 11; avoids the larger Win10 compatibility/signing matrix. | y |
| Repository layout | Same repository, new top-level `Windows/` directory (Rust workspace: app crate + a `protocol` crate reimplementing `Shared/Protocol.swift`'s constants) alongside the existing driver source tree | Mirrors the existing `Mac/` + `Shared/` layout; keeps one source of truth for wire-protocol compliance across platforms. `Shared/` itself stays Swift/Foundation-only and is not reused directly — Rust cannot link it, so its constants (protocol version, message-type strings) are ported, not shared, and must be kept in sync by hand until/unless a language-neutral protocol constants file is introduced. | n |
| Hardware H.264 encoder unavailable at runtime | Fall back to a software H.264 encoder rather than blocking the feature | No hardware-encoder inventory data exists yet for the Windows install base; failing outright would exclude older or low-end machines. Revisit if telemetry (none exists today — see Out of Scope) or user reports show this matters. | n |
| Wire protocol version used | Windows sender implements `pv 3` exactly as specified in `PROTOCOL.md`/`Shared/Protocol.swift`, introducing no new message types or fields | This feature is a new *implementation* of the existing spec, not a protocol change — per `COMPATIBILITY.md`, `pv` bumps only when the wire itself changes. | y |
| WiFi discovery mechanism | Windows sender advertises/browses the existing `_opensidecar._tcp` Bonjour/mDNS service type using a Windows-native or bundled mDNS implementation | Required for wire compatibility — the service type and TXT keys (`id`, `pv`) are fixed by `PROTOCOL.md` §2.1 regardless of platform. | y |
| Local session/connection logging | Keep a local rotating log file (mirroring `Mac/Log.swift` / `LogPolicies.swift` behavior: rotate, no upload, user-shareable) under the Windows per-user app-data path | Matches the project's "nothing is uploaded anywhere" privacy stance and existing bug-report workflow (README "Getting the logs"). | n |

**Open questions:** none — all resolved or logged above.

---

## User Stories

### P1: Extend the Windows desktop to an iPhone/iPad over WiFi ⭐ MVP

**User Story**: As a Windows 11 user with an iPhone or iPad on the same WiFi network, I want to pick my device from a list and have my Windows desktop extend onto it as a real second monitor, so I can use my spare device as a display without buying hardware.

**Why P1**: This is the smallest end-to-end vertical slice of the product's core value proposition; every other story builds on the same virtual-display/capture/encode pipeline.

**Acceptance Criteria**:

1. WHEN the user selects a discovered device from the WiFi connection list THEN the Windows sender SHALL dial TCP port 9000 on that device's advertised address and complete the `hello`/`welcome` handshake defined in `PROTOCOL.md` §6.
2. WHEN the handshake completes THEN the Windows sender SHALL create a virtual display sized from the device's reported panel dimensions at native pixel density (matching the device's `@2x`/`@3x` panel the way the Mac sender matches HiDPI today).
3. WHEN the virtual display is active THEN the Windows sender SHALL capture it, encode it as H.264 per `PROTOCOL.md` §5 (4-byte start codes, SPS/PPS on every IDR, one picture per frame), and stream it as length-prefixed frames per §3.
4. WHEN the device rotates between portrait and landscape THEN the Windows sender SHALL rebuild the virtual display at the new native resolution, matching the Mac sender's existing rotation behavior.
5. The Windows sender SHALL send a `ping` control message every 2 seconds while connected, per `PROTOCOL.md` §6 and Appendix A.
6. IF the TCP connection drops for any reason while a device was previously selected THEN the Windows sender SHALL retry the connection automatically until the device reappears or the user cancels, matching the Mac sender's existing retry behavior.
7. IF the receiver adopts a different sender's connection (protocol §1: "a receiver serves one sender at a time") THEN the Windows sender SHALL treat this as an ordinary disconnect, stop retrying that device automatically, and surface a status message indicating another sender is already connected.

**Independent Test**: On a Windows 11 PC and an iOS 16.4+ device on the same WiFi network, with the OpenDisplay receiver app open, pick the device from the Windows app's list and confirm a new display appears in Windows Display Settings that can be dragged windows onto, at the device's native resolution.

---

### P1: Extend the Windows desktop to an iPhone/iPad over USB ⭐ MVP

**User Story**: As a Windows 11 user, I want to plug my iPhone/iPad in with a data cable and have it work as a second display with lower latency than WiFi, without installing iTunes.

**Why P1**: USB is the project's flagship low-latency path on Mac; shipping it from day one (per user decision) means USB is not a second-class citizen.

**Acceptance Criteria**:

1. WHEN a data-capable USB cable connects a supported iPhone/iPad to the Windows PC THEN the Windows sender SHALL detect the device via the bundled libimobiledevice-compatible usbmuxd client, without requiring Apple's own driver to be installed.
2. WHEN a USB-attached device is detected THEN the Windows sender SHALL dial TCP port 9000 on that device through the usbmuxd tunnel and proceed with the same handshake, virtual-display, capture, and encode pipeline as the WiFi path (P1 WiFi story, ACs 1–5).
3. IF the connected USB cable does not carry a data line (charge-only) THEN the Windows sender SHALL fail to detect the device and SHALL NOT be required to distinguish this case from "no device attached" — the existing README guidance (use a data/sync cable) applies unchanged.
4. WHEN the same device is reachable over both a live USB connection and WiFi at once THEN the Windows sender SHALL prefer the USB path for the active session, matching the Mac sender's stated preference for lower latency over cable.
5. IF the device is unplugged mid-session THEN the Windows sender SHALL treat this as a connection drop and apply the same retry policy as WiFi story AC 6.

**Independent Test**: Connect an iOS 16.4+ device to the Windows PC with a known-good data cable, with no iTunes/Apple Devices app installed, and confirm the device is picked up and streams as a display without any WiFi network involved.

---

### P1: One-time signed driver install ⭐ MVP

**User Story**: As a Windows user running the app for the first time, I want the virtual-display driver installed with a single elevation prompt, without disabling any Windows security feature, so the app works out of the box.

**Why P1**: Every other P1 story depends on a working virtual display; without a clean, signed install path the product does not run on a default Windows 11 machine at all.

**Acceptance Criteria**:

1. WHEN the user runs the Windows sender for the first time and no compatible virtual-display driver is installed THEN the app SHALL prompt for one administrator elevation and install the attestation-signed IddCx driver package.
2. WHEN the driver package is installed THEN the app SHALL verify the driver loads (the virtual-display device interface becomes available) before offering any device connection in the UI.
3. IF the user declines the elevation prompt THEN the app SHALL remain open in a "setup incomplete" state, explain that a virtual display cannot be created yet, and re-offer the install on next launch rather than crashing or hanging.
4. IF the driver package fails to install or fails to load after installation THEN the app SHALL surface a specific, actionable error (not a generic failure) and SHALL NOT proceed to offer device connection.
5. The Windows sender SHALL NOT require Secure Boot to be disabled or Windows to be placed in test-signing mode at any point in this flow.
6. WHEN the app is uninstalled THEN the driver package SHALL be removable through the same Windows "Apps & features" surface as the app itself, leaving no orphaned virtual display behind.

**Independent Test**: On a clean Windows 11 VM with Secure Boot on and no prior OpenDisplay install, run the installer, accept the single elevation prompt, and confirm a new display adapter/device appears in Device Manager with no security warning blocking it — then uninstall and confirm it is gone.

---

### P1: Touch input and cursor forwarding ⭐ MVP

**User Story**: As a Windows user with a device connected as a display, I want to tap, drag, and two-finger-scroll on the phone and see the same effect on Windows, with the Windows cursor visible on the phone, so the extended display is actually usable and not just a passive screen.

**Why P1**: A display you cannot interact with does not satisfy the "true extension" goal; this is table-stakes for the vertical slice, not an enhancement.

**Acceptance Criteria**:

1. WHEN the Windows sender receives a `touch` control message from the connected device THEN it SHALL inject the equivalent mouse-down/move/up event at the corresponding point on the virtual display, producing tap-to-click and drag behavior.
2. WHEN the Windows sender receives a `scroll` control message THEN it SHALL inject the equivalent scroll-wheel input, producing two-finger-scroll-as-trackpad behavior matching the Mac sender.
3. WHILE the Windows cursor's position or image changes on the virtual display, the Windows sender SHALL send `cursor`/`cursorImg` control messages so the connected device renders the current cursor, per `PROTOCOL.md` baseline (`pv` 1) behavior.
4. IF a `pencil` or `proximity` control message arrives from a `pv ≥ 3` peer THEN the Windows sender SHALL accept the message without erroring, and MAY ignore it (Apple Pencil support is out of scope per this spec), since ignoring an optional control message it does not act on is protocol-conformant per Appendix A.

**Independent Test**: With a device connected as a display, tap and drag on the phone screen and confirm the Windows pointer moves and clicks/drags at the same normalized position; two-finger-scroll on the phone and confirm the focused Windows window scrolls; observe the Windows cursor icon rendered on the phone as it moves.

---

### P2: Multiple simultaneous devices

**User Story**: As a Windows user with more than one spare Apple device, I want to connect several at once, each becoming its own extended display, matching what the Mac sender already does.

**Why P2**: Valuable parity feature, but the single-device P1 stories already prove out the core pipeline; multi-device is additive scaling, not core value.

**Acceptance Criteria**:

1. WHEN more than one device is connected (any mix of USB and WiFi) THEN the Windows sender SHALL create one independent virtual display per connected device.
2. WHEN one of several connected devices disconnects THEN the Windows sender SHALL tear down only that device's virtual display and session, leaving the others streaming uninterrupted.

**Independent Test**: Connect two devices at once (e.g., one over USB, one over WiFi) and confirm two independent extended displays appear, each individually controllable and individually disconnectable.

---

### P2: Windows tray app and connection management UI

**User Story**: As a Windows user, I want a system-tray presence showing connection status, a device picker, and quick access to logs, mirroring the Mac app's menu-bar panel.

**Why P2**: P1's independent tests only require a minimal picker to prove the pipeline works; a polished tray/status UI is valuable but not required to validate the core feature.

**Acceptance Criteria**:

1. WHILE at least one device is connected, the Windows sender SHALL show connection status (device name, transport, basic stats) from the system tray, without requiring a maximized window.
2. WHEN the user requests logs from the tray UI THEN the app SHALL open the folder containing the rotating local log file described in the Assumptions table.

**Independent Test**: With the app running and a device connected, confirm tray status reflects the live connection and that a menu action opens the log folder.

---

### P3: Auto-update for the Windows app

**User Story**: As a Windows user, I want the app to notify me of new versions, similar to the Mac app's Sparkle-driven updates.

**Why P3**: Nice to have; manual download is an acceptable interim distribution model, and no Windows update framework has been chosen yet.

**Acceptance Criteria**:

1. WHEN a new version is published THEN the Windows sender SHALL be able to notify the user in-app, using a mechanism to be chosen in a future spec.

---

## Edge Cases

- IF the device advertises a `pv` below the Windows sender's `minSupportedPeer` (currently 1, i.e., never in practice) THEN the Windows sender SHALL send `updateRequired` and refuse to stream, per `COMPATIBILITY.md` §3.
- IF the connected device is locked or its screen sleeps mid-session THEN the Windows sender SHALL handle the `sleeping` control message by pausing the session and reconnecting automatically on wake, matching the Mac sender's documented behavior.
- IF the receiver app is quit by the user (`closing` control message) THEN the Windows sender SHALL end the session for good and SHALL NOT attempt to reconnect until the device is rediscovered fresh.
- IF the hardware H.264 encoder is unavailable on the Windows machine THEN the Windows sender SHALL fall back to a software H.264 encoder per the Assumptions table, rather than failing to stream.
- IF a video frame payload would exceed `2^20 - 1` bytes (the receiver-direction bound in `PROTOCOL.md` §3) THEN the Windows sender SHALL NOT emit it as a single frame — it SHALL respect the same framing bound the protocol imposes on any conformant sender.
- WHILE the P1 single-device constraint is in effect, IF a second device becomes available while one is already connected THEN the Windows sender SHALL list it as available but SHALL NOT auto-connect to it, requiring an explicit user action, and SHALL make clear that connecting it will end the first session (P1 has no multi-device support).

---

## Requirement Traceability

Each requirement gets a unique ID for tracking across design, tasks, and validation.

| Requirement ID | Story | Phase | Status |
| --- | --- | --- | --- |
| WSEND-01 | P1: WiFi extension | Tasks | In Tasks |
| WSEND-02 | P1: WiFi extension | Tasks | Implementing |
| WSEND-03 | P1: WiFi extension | Tasks | In Tasks |
| WSEND-04 | P1: WiFi extension | Tasks | In Tasks |
| WSEND-05 | P1: WiFi extension | Tasks | In Tasks |
| WSEND-06 | P1: WiFi extension | Tasks | In Tasks |
| WSEND-07 | P1: WiFi extension | Tasks | In Tasks |
| WSEND-08 | P1: USB extension | Tasks | In Tasks |
| WSEND-09 | P1: USB extension | Tasks | In Tasks |
| WSEND-10 | P1: USB extension | Tasks | In Tasks |
| WSEND-11 | P1: USB extension | Tasks | In Tasks |
| WSEND-12 | P1: USB extension | Tasks | In Tasks |
| WSEND-13 | P1: Driver install | Tasks | In Tasks |
| WSEND-14 | P1: Driver install | Tasks | Implementing |
| WSEND-15 | P1: Driver install | Tasks | In Tasks |
| WSEND-16 | P1: Driver install | Tasks | In Tasks |
| WSEND-17 | P1: Driver install | Tasks | In Tasks |
| WSEND-18 | P1: Driver install | Tasks | In Tasks |
| WSEND-19 | P1: Touch/cursor | Tasks | In Tasks |
| WSEND-20 | P1: Touch/cursor | Tasks | In Tasks |
| WSEND-21 | P1: Touch/cursor | Tasks | In Tasks |
| WSEND-22 | P1: Touch/cursor | Tasks | In Tasks |
| WSEND-23 | P2: Multi-device | - | Pending |
| WSEND-24 | P2: Multi-device | - | Pending |
| WSEND-25 | P2: Tray UI | - | Pending |
| WSEND-26 | P2: Tray UI | - | Pending |
| WSEND-27 | P3: Auto-update | - | Pending |

**ID format:** `WSEND-[NUMBER]`

**Status values:** Pending → In Design → In Tasks → Implementing → Verified

**Coverage:** 27 total, 22 mapped to tasks (all of P1), 5 unmapped ⚠️ (P2/P3 — Tasks phase intentionally scoped to P1 for this pass; see `tasks.md` "Scope of this pass")

---

## Success Criteria

How we know the feature is successful:

- [ ] A Windows 11 user with no prior setup can go from launching the installer to dragging a window onto their iPhone/iPad's screen in a single sitting, using only the in-app first-run flow (no manual driver-signing steps, no manual `bcdedit` commands, no iTunes install for USB).
- [ ] Both the WiFi and USB P1 stories pass their independent tests on real hardware (a physical Windows 11 PC and a physical iOS 16.4+ device), not only in emulation.
- [ ] The Windows sender interoperates with the current shipped iOS receiver app with zero receiver-side code changes and zero `PROTOCOL.md`/`Shared/Protocol.swift` changes.
- [ ] The signed driver installs and loads on a stock Windows 11 machine with Secure Boot enabled, with no security warning that blocks installation.
- [ ] Touch tap/drag, two-finger scroll, and cursor forwarding all function end-to-end on both transports.
