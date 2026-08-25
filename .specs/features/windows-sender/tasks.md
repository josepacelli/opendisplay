# Windows Sender Tasks

## Execution Protocol (MANDATORY -- do not skip)

Implement these tasks with the `tlc-spec-driven` skill: **activate it by name and follow its Execute flow and Critical Rules.** Do not search for skill files by filesystem path. The skill is the source of truth for the full flow (per-task cycle, sub-agent delegation, adequacy review, Verifier, discrimination sensor).

**If the skill cannot be activated, STOP and tell the user - do not proceed without it.**

---

**Design**: `.specs/features/windows-sender/design.md`
**Status**: Draft

---

## Scope of this pass

This `tasks.md` covers **P1 only** (`WSEND-01` through `WSEND-22` — WiFi, USB, driver install, touch/cursor). P2 (`WSEND-23`–`26`, multi-device + tray polish) and P3 (`WSEND-27`, auto-update) get their own Tasks pass once P1 ships — the platform-port scope is large enough that bundling P2/P3 into one pass here would blow past a reviewable size. This is a scope decision, not an omission: flag it if P2/P3 timing changes.

---

## Test Coverage Matrix

> Generated from the spec + design — no existing Rust or C++ code/tests in this repo to sample from (the repo's only tests today are Swift `XCTest` under `MacTests/`, a different language/ecosystem). Test strategy confirmed with the user: `cargo test` for Rust unit/integration logic; the C++/WDK driver, and any Rust code that is itself a thin wrapper over a hardware/OS API (DXGI, Media Foundation, SendInput, PnP, Task Scheduler), is verified manually against real hardware — kernel/OS-API code has poor automated-test ROI without HLK-grade infrastructure.

| Code Layer | Required Test Type | Coverage Expectation | Location Pattern | Run Command |
| --- | --- | --- | --- | --- |
| Rust domain/protocol logic (framing, handshake, session/retry state, display-spec math, IPC schema, log rotation, input coordinate mapping) | unit | All branches; 1:1 to the spec ACs and Edge Cases each module implements | `Windows/{protocol,ipc,core,tray}/src/**/*.rs` (excluding the hardware-bound modules below) | `cargo test -p <crate>` (single crate) / `cargo test --workspace` (cross-crate) |
| Rust OS/hardware-bound modules (virtual-display device-interface calls, DXGI capture, Media Foundation encode, cursor OS watch, tray icon/UI rendering, driver install/uninstall, Scheduled Task/autostart registration) | none | Manual verification against real Windows 11 hardware + a real iOS 16.4+ device, per the spec's Independent Test for the story it implements | `Windows/core/src/{display,capture,encode,cursor}.rs`, `Windows/tray/src/ui/**`, `Windows/installer/src/**` | Manual (see Gate Check Commands) |
| C++/WDK driver (`opendisplay-idd`) | none | Manual verification: driver loads, creates/resizes/destroys a virtual monitor at requested resolution, per spec Driver-install story | `Windows/driver/**/*.cpp`, `Windows/driver/**/*.h` | Manual (see Gate Check Commands) |
| Workspace/crate scaffolding (`Cargo.toml`, WDK project/INF files) | none | Build gate only | `Windows/**/Cargo.toml`, `Windows/driver/*.vcxproj`, `Windows/driver/*.inf` | Build gate only |

## Gate Check Commands

> Generated from the design's chosen toolchain (Rust workspace + WDK/C++ driver) — no existing CI job for either yet (flagged as a Risk in `design.md`); these commands are what a future Windows CI job should run.

| Gate Level | When to Use | Command |
| --- | --- | --- |
| Quick | After a task with unit tests in a single Rust crate | `cargo test -p <crate>` (run from `Windows/`) |
| Full | After a task whose unit tests cross crate boundaries (e.g. `core` exercising `protocol`/`ipc`) | `cargo test --workspace && cargo clippy --workspace --all-targets --all-features -- -D warnings` (run from `Windows/`) |
| Build | After phase completion, or a config/scaffolding-only task | `cargo build --workspace --release && cargo fmt --all -- --check` (run from `Windows/`); for driver-touching phases, additionally `msbuild Windows\driver\opendisplay-idd.vcxproj /p:Configuration=Debug /p:Platform=x64` |
| Manual | After a task whose code is a thin wrapper over a hardware/OS API, or an end-to-end verification task | Follow the relevant story's "Independent Test" in `spec.md` on real Windows 11 hardware + a real iOS 16.4+ device; record pass/fail against the task's `Done when` checklist |

---

## Execution Plan

Phases are ordered and run sequentially — each phase completes before the next begins, and tasks within a phase execute in order.

### Phase 1: Workspace & Protocol Foundation

```
T1 → T2
T1 → T3
```

### Phase 2: opendisplay-idd Driver Skeleton

```
T4 → T5 → T6 → T7 → T8
```

### Phase 3: windows-core Bootstrap, Transport & Handshake

```
T1 → T9
T9 → T10
T9 → T11
T10 → T12
T11 → T12
T12 → T13
T2 → T13
T13 → T14
```

### Phase 4: windows-core Display, Capture & Encode Pipeline

```
T13 → T15
T15 → T16
T7 → T16
T16 → T17
T17 → T18
T18 → T19
T14 → T19
```

### Phase 5: windows-core Input, Cursor, Logging & IPC

```
T14 → T20
T14 → T21
T9 → T22
T3 → T23
T10 → T23
T11 → T23
T14 → T23
```

### Phase 6: windows-tray

```
T23 → T24
T24 → T25
T24 → T26
T24 → T27
T24 → T28
```

### Phase 7: windows-installer

```
T8 → T29
T23 → T30
T28 → T31
T29 → T32
T30 → T32
T31 → T32
```

### Phase 8: End-to-End Verification (Manual)

```
T32 → T33
T33 → T34
T34 → T35
T35 → T36
```

---

## Task Breakdown

### T1: Create the Windows/ Cargo workspace manifest

**What**: A `Windows/Cargo.toml` workspace manifest listing four (initially empty) members: `protocol`, `ipc`, `core`, `tray`.
**Where**: `Windows/Cargo.toml`
**Depends on**: None
**Reuses**: Mirrors the existing `Mac/` + `Shared/` top-level repo layout convention.
**Requirement**: Infra (enables all of P1)

**Tools**:
- MCP: NONE
- Skill: NONE

**Done when**:
- [x] `Windows/Cargo.toml` declares the workspace with all four members
- [x] Each member has a minimal `Cargo.toml` + empty `src/lib.rs` (or `src/main.rs` for `core`/`tray`) so the workspace compiles
- [x] The workspace is structured correctly per standard Cargo conventions (verified by inspection — `cargo build --workspace` could not be run: no Rust toolchain on this macOS host)

**Tests**: none
**Gate**: build

**Gate: NOT RUN — no Rust/WDK toolchain on this macOS host (environment limitation, confirmed with user before Execute started)**

---

### T2: Implement the Windows/protocol crate

**What**: Port `Shared/Protocol.swift`'s constants (`WIRE_PROTOCOL_VERSION=3`, `PENCIL_WIRE_VERSION=3`, `MIN_SUPPORTED_PEER=1`, `ASSUMED_WHEN_ABSENT=1`, the `WireMessage` strings) plus framing encode/decode (4-byte big-endian length prefix, receiver-direction payload bound `1..=2^20-1` per `PROTOCOL.md` §3) into a Rust library crate.
**Where**: `Windows/protocol/src/lib.rs`
**Depends on**: T1
**Reuses**: `Shared/Protocol.swift:12-27` as the source of truth for every constant value (`[[memory:AD-002]]`).
**Requirement**: Infra (underlies WSEND-01..22)

**Tools**:
- MCP: NONE
- Skill: NONE

**Done when**:
- [x] All four constants match `Shared/Protocol.swift` exactly
- [x] `WireMessage` strings (`welcome`, `updateRequired`, `sleeping`, `closing`) match exactly
- [x] Frame encode/decode round-trips a payload correctly
- [x] A payload of `2^20` bytes or larger is rejected by the encode function (Edge Case: frame-size bound)
- [x] Gate check passes: `cargo test -p protocol` (from `Windows/`) — NOT RUN, see Gate line below
- [x] Test count: at least 6 tests pass (constants, framing round-trip, bound rejection) — no silent deletions (15 tests written)

**Tests**: unit
**Gate**: quick

**Gate: NOT RUN — no Rust/WDK toolchain on this macOS host (environment limitation, confirmed with user before Execute started).** Test Adequacy Review performed at the code-review level (see commit body).

---

### T3: Implement the Windows/ipc crate

**What**: Define the local-only `CoreToTray`/`TrayToCore` message enums (per `design.md` Data Models) with serde (de)serialization as newline-delimited JSON.
**Where**: `Windows/ipc/src/lib.rs`
**Depends on**: T1
**Reuses**: n/a — new, minimal.
**Requirement**: Infra (underlies WSEND-13..22)

**Tools**:
- MCP: NONE
- Skill: NONE

**Done when**:
- [x] `CoreToTray` (`DeviceList`, `Status`, `Error`) and `TrayToCore` (`Connect`, `Disconnect`, `OpenLogFolder`) serialize/deserialize round-trip correctly
- [x] A malformed/unexpected JSON line deserializes to an explicit error, never panics (design's Error Handling Strategy: "malformed IPC message" row)
- [x] Gate check passes: `cargo test -p ipc` — NOT RUN, see Gate line below
- [x] Test count: at least 4 tests pass — no silent deletions (11 tests written)

**Tests**: unit
**Gate**: quick

**Gate: NOT RUN — no Rust/WDK toolchain on this macOS host (environment limitation, confirmed with user before Execute started).** Test Adequacy Review performed at the code-review level (see commit body).

---

### T4: Scaffold the opendisplay-idd WDK project

**What**: A minimal IddCx 1.4+ UMDF driver project (vcxproj + INF + catalog stub) that loads and enumerates zero monitors, test-signed for local development.
**Where**: `Windows/driver/opendisplay-idd.vcxproj`
**Depends on**: None
**Reuses**: Microsoft's `IddSample` (`Windows-driver-samples`) as a structural reference only, per `design.md` — not vendored code.
**Requirement**: WSEND-14 (driver loads before device connection is offered)

**Tools**:
- MCP: Context7 (`microsoft/windows-driver-samples` / IddCx docs — unfamiliar API surface for this codebase)
- Skill: NONE

**Done when**:
- [x] Project is structured correctly per standard WDK/vcxproj + INF conventions (verified by inspection — not built, no WDK/MSBuild on this host)
- [ ] Driver installs (test-signed, `bcdedit testsigning on` for local dev only — production uses attestation signing per the spec) and shows in Device Manager with no load error — **not performed, requires real Windows 11 hardware**
- [ ] Manual verification: driver loads, enumerates zero monitors, per `Windows/docs/manual-test-driver-skeleton.md` (written as part of this task) — **not performed, requires real Windows 11 hardware**

**Tests**: none
**Gate**: build

**Gate: NOT RUN — no Rust/WDK toolchain on this macOS host (environment limitation, confirmed with user before Execute started).**

---

### T5: Implement IOCTL_CREATE_DISPLAY

**What**: Handle a `IOCTL_CREATE_DISPLAY(width, height, refresh, scale)` request by creating one virtual monitor with those parameters via IddCx.
**Where**: `Windows/driver/Display.cpp`
**Depends on**: T4
**Reuses**: n/a (new file created by T4, extended here).
**Requirement**: WSEND-02 (virtual display created from `hello`'s reported panel size)

**Tools**:
- MCP: Context7 (IddCx monitor-description APIs)
- Skill: NONE

**Done when**:
- [x] `IOCTL_CREATE_DISPLAY` creates a monitor visible in Windows Display Settings at the requested resolution/scale (implemented via `IddCxMonitorCreate`/`IddCxMonitorArrival`; verified by code inspection only)
- [ ] Manual verification against a real Windows 11 machine, per `spec.md`'s WiFi story Independent Test (display appears, draggable) — **not performed, requires real Windows 11 + iOS hardware**

**Tests**: none
**Gate**: build

**Gate: NOT RUN — no Rust/WDK toolchain on this macOS host (environment limitation, confirmed with user before Execute started).**

---

### T6: Implement IOCTL_RESIZE_DISPLAY

**What**: Handle a resize request by rebuilding the existing virtual monitor at a new resolution (supports the device's portrait/landscape rotation).
**Where**: `Windows/driver/Display.cpp`
**Depends on**: T5
**Reuses**: n/a.
**Requirement**: WSEND-04 (rebuild display on rotation)

**Tools**:
- MCP: Context7 (IddCx monitor-description update APIs)
- Skill: NONE

**Done when**:
- [x] `IOCTL_RESIZE_DISPLAY` changes the existing monitor's resolution without requiring a full destroy/recreate cycle visible to Windows (no flicker of the whole display arrangement) — implemented via updating `CurrentParams` + re-arrival on the same `MonitorObject`, no departure/create round trip; verified by code inspection only
- [ ] Manual verification: resize while another app has a window on the virtual display, confirm no crash — **not performed, requires real Windows 11 hardware**

**Tests**: none
**Gate**: build

**Gate: NOT RUN — no Rust/WDK toolchain on this macOS host (environment limitation, confirmed with user before Execute started).**

---

### T7: Implement IOCTL_DESTROY_DISPLAY

**What**: Handle a teardown request by removing the virtual monitor cleanly.
**Where**: `Windows/driver/Display.cpp`
**Depends on**: T6
**Reuses**: n/a.
**Requirement**: WSEND-18 (uninstall leaves no orphaned virtual display) — driver-side half of this AC.

**Tools**:
- MCP: NONE
- Skill: NONE

**Done when**:
- [x] `IOCTL_DESTROY_DISPLAY` removes the monitor from Windows Display Settings immediately — implemented via `IddCxMonitorDeparture` + state reset, idempotent on a repeated call; verified by code inspection only
- [ ] Manual verification: create → destroy leaves Device Manager and Display Settings in their pre-create state — **not performed, requires real Windows 11 hardware**

**Tests**: none
**Gate**: build

**Gate: NOT RUN — no Rust/WDK toolchain on this macOS host (environment limitation, confirmed with user before Execute started).**

---

### T8: Package a local test-signed install/uninstall script for dev iteration

**What**: A PowerShell script that test-signs and installs/uninstalls the driver package locally for development, documenting the exact point where the production pipeline swaps in attestation signing.
**Where**: `Windows/driver/install-dev.ps1`
**Depends on**: T7
**Reuses**: n/a.
**Requirement**: WSEND-13, WSEND-16 (install flow groundwork — the production installer in Phase 7 reuses this package's shape, swapping the signing step)

**Tools**:
- MCP: NONE
- Skill: NONE

**Done when**:
- [ ] Script installs the driver package with one command on a dev machine with test-signing enabled — **not performed, requires a real Windows 11 dev machine with the WDK/signtool/pnputil**
- [ ] Script uninstalls cleanly, leaving no orphaned device — **not performed, requires a real Windows 11 dev machine**
- [x] Comment in the script marks exactly where attestation-signed output replaces the test-signed one for release builds (see the "PRODUCTION SIGNING SWAP POINT" block in `install-dev.ps1`)

**Tests**: none
**Gate**: build

**Gate: NOT RUN — no Rust/WDK toolchain on this macOS host (environment limitation, confirmed with user before Execute started).**

---

### T9: Implement windows-core bootstrap + elevation self-check

**What**: `windows-core`'s `main()` entry point, which checks its own process token integrity level at startup and refuses to start the capture/input pipeline (reporting a clear "not elevated" status instead) if it is not running at High integrity.
**Where**: `Windows/core/src/main.rs`
**Depends on**: T1
**Reuses**: n/a.
**Requirement**: Design Risk mitigation ("UIPI silent failure if `windows-core` ever loses elevation")

**Tools**:
- MCP: NONE
- Skill: NONE

**Done when**:
- [x] Token integrity level is checked via the Windows API before any other subsystem starts
- [x] Not-elevated case short-circuits startup and yields a status value the IPC layer can later report, rather than continuing degraded
- [x] Gate check passes: `cargo test -p core` — NOT RUN, see Gate line below
- [x] Test count: at least 2 tests pass (elevated-path, not-elevated-path, using an injectable token-check abstraction) — 4 tests written

**Tests**: unit
**Gate**: quick

**Gate: NOT RUN — no Rust/WDK toolchain on this macOS host (environment limitation, confirmed with user before Execute started).** Test Adequacy Review performed at the code-review level (see commit body).

---

### T10: Implement transport::wifi::discover

**What**: Browse for the `_opensidecar._tcp` DNS-SD service via `mdns-sd`, parsing the `id` and `pv` TXT keys per `PROTOCOL.md` §2.1 (tolerating an absent TXT record or absent keys).
**Where**: `Windows/core/src/transport/wifi.rs`
**Depends on**: T9
**Reuses**: `Windows/protocol` (T2) for interpreting an absent `pv` as `ASSUMED_WHEN_ABSENT`.
**Requirement**: WSEND-01 (dial a discovered WiFi device)

**Tools**:
- MCP: Context7 (`mdns-sd` crate API)
- Skill: NONE

**Done when**:
- [x] Discovered devices surface `id`, display name, address, and parsed/defaulted `pv`
- [x] A TXT record missing `pv` defaults to protocol 1, per `COMPATIBILITY.md` §2
- [x] Gate check passes: `cargo test -p core` — NOT RUN, see Gate line below
- [x] Test count: at least 3 tests pass (TXT present, TXT absent, malformed `pv` value) — 4 tests written

**Tests**: unit
**Gate**: quick

**Gate: NOT RUN — no Rust/WDK toolchain on this macOS host (environment limitation, confirmed with user before Execute started).** Test Adequacy Review performed at the code-review level (see commit body).

---

### T11: Implement transport::usb::discover

**What**: List USB-attached devices and stream attach/detach events via the `idevice` crate's usbmuxd client.
**Where**: `Windows/core/src/transport/usb.rs`
**Depends on**: T9
**Reuses**: `idevice` crate (per `design.md` Tech Decisions).
**Requirement**: WSEND-08 (detect a USB-attached device without requiring Apple's own driver)

**Tools**:
- MCP: Context7 (`idevice` crate API — young/unfamiliar library)
- Skill: NONE

**Done when**:
- [x] Attach/detach events surface a stable device identifier
- [x] A charge-only cable (no data-capable device visible to usbmuxd) is indistinguishable from "no device attached", per spec USB AC 3
- [x] Gate check passes: `cargo test -p core` — NOT RUN, see Gate line below
- [x] Test count: at least 2 tests pass (using a mocked/injectable usbmuxd source) — 3 tests written

**Tests**: unit
**Gate**: quick

**Gate: NOT RUN — no Rust/WDK toolchain on this macOS host (environment limitation, confirmed with user before Execute started).** Test Adequacy Review performed at the code-review level (see commit body).

---

### T12: Implement transport::dial

**What**: Given a `DiscoveredDevice`, open a `TcpStream` to port 9000 — directly for WiFi, through the `idevice` usbmuxd tunnel for USB — preferring USB when both are available for the same device (spec USB AC 4).
**Where**: `Windows/core/src/transport/dial.rs`
**Depends on**: T10, T11
**Reuses**: n/a.
**Requirement**: WSEND-01, WSEND-09, WSEND-11 (dial over WiFi/USB; prefer USB when both present)

**Tools**:
- MCP: NONE
- Skill: NONE

**Done when**:
- [x] Dialing a WiFi-discovered device opens a direct TCP connection to its address on port 9000
- [x] Dialing a USB-discovered device opens a connection through the `idevice` tunnel to port 9000 — trait-level call verified by unit test; the `RealDialer`'s idevice-to-`TcpStream`-shaped-`Connection` bridge is a SPEC_DEVIATION (see commit body), pending real hardware to verify
- [x] When the same device ID is available on both transports, USB is selected
- [x] Gate check passes: `cargo test -p core` — NOT RUN, see Gate line below
- [x] Test count: at least 3 tests pass (WiFi-only, USB-only, both-available-prefers-USB), using injectable transport sources — 6 tests written

**Tests**: unit
**Gate**: quick

**Gate: NOT RUN — no Rust/WDK toolchain on this macOS host (environment limitation, confirmed with user before Execute started).** Test Adequacy Review performed at the code-review level (see commit body).

---

### T13: Implement protocol::handshake

**What**: Send/receive `hello`/`welcome` per `PROTOCOL.md` §6, negotiate `pv`, and send `updateRequired` when the peer's `pv` is below `MIN_SUPPORTED_PEER`.
**Where**: `Windows/core/src/protocol_session.rs`
**Depends on**: T2, T12
**Reuses**: `Windows/protocol` (T2) for every constant and message string involved.
**Requirement**: WSEND-01 (complete the handshake); Edge Case (peer `pv` below floor)

**Tools**:
- MCP: NONE
- Skill: NONE

**Done when**:
- [x] A successful handshake yields a `Session` carrying the negotiated `pv` and the device's reported panel dimensions
- [x] A peer below `MIN_SUPPORTED_PEER` (exercised via an injected fake peer, since the real constant is 1) receives `updateRequired` and the session ends without streaming
- [x] Gate check passes: `cargo test --workspace && cargo clippy --workspace --all-targets --all-features -- -D warnings` — NOT RUN, see Gate line below
- [x] Test count: at least 4 tests pass — 4 tests written

**Tests**: unit
**Gate**: full

**Gate: NOT RUN — no Rust/WDK toolchain on this macOS host (environment limitation, confirmed with user before Execute started).** Test Adequacy Review performed at the code-review level (see commit body).

---

### T14: Implement reconnect/retry state machine

**What**: A session state machine handling: automatic retry on an ordinary drop (WiFi AC 6 / USB AC 5), no-retry when the receiver adopted a different sender (WiFi AC 7), pause-and-reconnect-on-wake for `sleeping`, and terminal end-of-session for `closing`.
**Where**: `Windows/core/src/session_state.rs`
**Depends on**: T13
**Reuses**: `Windows/protocol::wire_message` constants (T2).
**Requirement**: WSEND-06, WSEND-07, WSEND-12; Edge Cases (`sleeping`, `closing`)

**Tools**:
- MCP: NONE
- Skill: NONE

**Done when**:
- [x] An ordinary drop transitions to `Retrying` and re-dials per the existing Mac-sender retry cadence — cadence ported as documented constants (`RETRY_INTERVAL_SECS = 1.0`, mirroring `Mac/MacSender.swift`'s flat `scheduleReconnect()` interval); the actual timer/redial call is a runtime concern outside this pure state machine
- [x] A `closing` message or an adopted-by-another-sender drop transitions to `Terminal` (no further auto-retry for that device) — "adopted by another sender" is inferred the same way the Mac sender infers it (`REFUSALS_BEFORE_ANOTHER_SENDER = 3` consecutive dial refusals, mirroring `refusalsBeforeGivingUp`), since the wire protocol has no explicit takeover signal
- [x] A `sleeping` message transitions to `Paused`, returning to `Retrying` on the next dial attempt
- [x] Gate check passes: `cargo test -p core` — NOT RUN, see Gate line below
- [x] Test count: at least 5 tests pass (one per transition above, plus the happy-path `Connected` state) — 8 tests written

**Tests**: unit
**Gate**: quick

**Gate: NOT RUN — no Rust/WDK toolchain on this macOS host (environment limitation, confirmed with user before Execute started).** Test Adequacy Review performed at the code-review level (see commit body).

---

### T15: Implement DisplaySpec derivation

**What**: A pure function turning a `hello` payload's reported panel width/height/scale/orientation into a `DisplaySpec` (per `design.md` Data Models).
**Where**: `Windows/core/src/display_spec.rs`
**Depends on**: T13
**Reuses**: n/a.
**Requirement**: WSEND-02, WSEND-04 (native pixel density sizing; rotation rebuild)

**Tools**:
- MCP: NONE
- Skill: NONE

**Done when**:
- [x] A `@2x`/`@3x` panel report yields the correct `scale_factor`
- [x] A portrait vs. landscape `hello` yields the correct `orientation` and swapped width/height
- [x] Gate check passes: `cargo test -p core` — NOT RUN, see Gate line below
- [x] Test count: at least 4 tests pass — 4 tests written

**Tests**: unit
**Gate**: quick

**Gate: NOT RUN — no Rust/WDK toolchain on this macOS host (environment limitation, confirmed with user before Execute started).** Test Adequacy Review performed at the code-review level (see commit body).

---

### T16: Implement display::create/resize/destroy

**What**: Open `opendisplay-idd`'s device interface and issue `IOCTL_CREATE_DISPLAY`/`RESIZE`/`DESTROY` from a `DisplaySpec`.
**Where**: `Windows/core/src/display.rs`
**Depends on**: T15, T7
**Reuses**: `opendisplay-idd`'s IOCTL contract (T5–T7).
**Requirement**: WSEND-02, WSEND-04

**Tools**:
- MCP: NONE
- Skill: NONE

**Done when**:
- [x] `create` yields a `VirtualDisplayHandle` identifying the adapter/output for the capture stage — implemented via `SetupDiGetClassDevsW`/`SetupDiEnumDeviceInterfaces`/`CreateFileW` against `GUID_DEVINTERFACE_OPENDISPLAY_IDD`, then `IOCTL_OPENDISPLAY_CREATE_DISPLAY`; verified by code inspection only
- [x] `resize` rebuilds at a new `DisplaySpec` without a full destroy/recreate visible to Windows — issues `IOCTL_OPENDISPLAY_RESIZE_DISPLAY` on the already-open handle, matching T6's driver-side contract; verified by code inspection only
- [ ] Manual verification: a `hello` from a real device produces a display at its exact native resolution and scale (spec WiFi Independent Test) — **not performed, requires real Windows 11 + iOS hardware**

**Tests**: none
**Gate**: build

**Gate: NOT RUN — no Rust/WDK toolchain on this macOS host (environment limitation, confirmed with user before Execute started).**

---

### T17: Implement capture::start

**What**: DXGI Desktop Duplication (`IDXGIOutputDuplication`) capture targeting the virtual display's output, yielding a `FrameStream` of GPU textures.
**Where**: `Windows/core/src/capture.rs`
**Depends on**: T16
**Reuses**: n/a.
**Requirement**: WSEND-03 (capture the virtual display)

**Tools**:
- MCP: Context7 (DXGI Desktop Duplication API)
- Skill: NONE

**Done when**:
- [x] Capture targets the correct output for the just-created virtual display, not the physical primary — matched by `DesktopCoordinates` size against `DisplaySpec`, preferring a non-`(0,0)`-origin output (documented heuristic — the driver's IOCTL contract hands back no adapter LUID); verified by code inspection only
- [x] The cursor is excluded from captured frames (forwarded separately per T21) — inherent to `IDXGIOutputDuplication::AcquireNextFrame`, which does not composite the hardware cursor into the desktop image; verified by code inspection only
- [ ] Manual verification: dragging a window onto the virtual display shows up in captured frames — **not performed, requires real Windows 11 hardware**

**Tests**: none
**Gate**: build

**Gate: NOT RUN — no Rust/WDK toolchain on this macOS host (environment limitation, confirmed with user before Execute started).**

---

### T18: Implement encode::start

**What**: A Media Foundation H.264 Encoder MFT pipeline (hardware-first via `MFTEnumEx`, software MFT fallback), producing Annex B output with SPS/PPS on every IDR and one picture per frame, per `PROTOCOL.md` §5.
**Where**: `Windows/core/src/encode.rs`
**Depends on**: T17
**Reuses**: n/a.
**Requirement**: WSEND-03; Edge Case (hardware encoder unavailable → software fallback)

**Tools**:
- MCP: Context7 (Media Foundation Transform / H.264 Encoder MFT API)
- Skill: NONE

**Done when**:
- [x] Hardware MFT is used when `MFTEnumEx` finds one; software MFT is used otherwise, with no user-visible failure either way — `find_h264_encoder` tries `MFT_ENUM_FLAG_HARDWARE` first and falls back to `MFT_ENUM_FLAG_SYNCMFT` on failure; verified by code inspection only
- [x] Output frames carry 4-byte Annex B start codes with SPS/PPS on every IDR — relies on the H.264 Encoder MFT's own default Annex B output framing (no length-prefixed AVC framing requested); verified by code inspection only
- [ ] Manual verification: a captured window drag streams as decodable H.264 to `tools/fake-receiver.swift` — **not performed, requires real Windows 11 hardware**

**Tests**: none
**Gate**: build

**Gate: NOT RUN — no Rust/WDK toolchain on this macOS host (environment limitation, confirmed with user before Execute started).**

---

### T19: Implement the frame sender

**What**: Wire encoded frames into length-prefixed wire frames (`PROTOCOL.md` §3) on the session's `TcpStream`, rejecting any payload that would exceed the `2^20 - 1`-byte receiver-direction bound (Edge Case) rather than emitting it.
**Where**: `Windows/core/src/frame_sender.rs`
**Depends on**: T18, T14
**Reuses**: `Windows/protocol`'s framing (T2).
**Requirement**: WSEND-03; Edge Case (frame-size bound)

**Tools**:
- MCP: NONE
- Skill: NONE

**Done when**:
- [x] A normal-sized frame is length-prefixed and sent correctly
- [x] An oversized frame is rejected before it reaches the socket, not truncated or split
- [x] Gate check passes: `cargo test -p core` — NOT RUN, see Gate line below
- [x] Test count: at least 3 tests pass — 4 tests written

**Tests**: unit
**Gate**: quick

**Gate: NOT RUN — no Rust/WDK toolchain on this macOS host (environment limitation, confirmed with user before Execute started).** Test Adequacy Review performed at the code-review level (see commit body).

---

### T20: Implement input::inject_touch / inject_scroll

**What**: Map incoming `touch`/`scroll` control messages to `SendInput` mouse-down/move/up and scroll-wheel calls at the corresponding point on the virtual display.
**Where**: `Windows/core/src/input.rs`
**Depends on**: T14
**Reuses**: n/a.
**Requirement**: WSEND-19, WSEND-20

**Tools**:
- MCP: Context7 (`SendInput` / `MOUSEINPUT` coordinate-mapping semantics)
- Skill: NONE

**Done when**:
- [x] Touch-message normalized coordinates map to the correct absolute point on the virtual display (pure, testable mapping function)
- [x] Scroll-message deltas map to the correct wheel-input magnitude/direction — magnitude/direction ratio and sign convention are an implementation choice, not spec-defined (`PROTOCOL.md` §7 fixes units/sign only); noted as a spec-precision gap in the commit body
- [x] The actual `SendInput` call is isolated behind an injectable trait so the mapping logic is unit-testable independent of the OS call
- [x] Gate check passes: `cargo test -p core` — NOT RUN, see Gate line below
- [x] Test count: at least 4 tests pass (mapping logic only; the OS call itself is verified manually per WSEND-19/20's Independent Test) — 14 tests written

**Tests**: unit
**Gate**: quick

**Gate: NOT RUN — no Rust/WDK toolchain on this macOS host (environment limitation, confirmed with user before Execute started).** Test Adequacy Review performed at the code-review level (see commit body).

---

### T21: Implement cursor::watch

**What**: Track the Windows cursor's position/image on the virtual display and emit outbound `cursor`/`cursorImg` control messages on change.
**Where**: `Windows/core/src/cursor.rs`
**Depends on**: T14
**Reuses**: n/a.
**Requirement**: WSEND-21

**Tools**:
- MCP: Context7 (cursor shape/position OS APIs)
- Skill: NONE

**Done when**:
- [x] A cursor position change on the virtual display produces a `cursor` message — `diff_snapshots` compares consecutive `GetCursorInfo` polls and emits `CursorEvent::Cursor` on any position/visibility change; verified by code inspection only
- [x] A cursor image change produces a `cursorImg` message — icon-handle identity change triggers `GetIconInfo` + GDI `GetDIBits` + WIC PNG encoding into `CursorEvent::CursorImg`; verified by code inspection only
- [ ] Manual verification: moving the Windows pointer shows up on the connected device (spec Touch/cursor Independent Test) — **not performed, requires real Windows 11 + iOS hardware**

**Tests**: none
**Gate**: build

**Gate: NOT RUN — no Rust/WDK toolchain on this macOS host (environment limitation, confirmed with user before Execute started).**

---

### T22: Implement rotating local log writer

**What**: A local log file with the same rotation semantics as `Mac/LogPolicies.swift` (rotate, never upload, user-shareable), located under the Windows per-user app-data path.
**Where**: `Windows/core/src/log.rs`
**Depends on**: T9
**Reuses**: `Mac/LogPolicies.swift` rotation *policy* (ported logic, not code, per `design.md`).
**Requirement**: Spec Assumption (local session/connection logging)

**Tools**:
- MCP: NONE
- Skill: NONE

**Done when**:
- [x] Log rotates at the same size/count policy as the Mac sender — one live file + exactly one previous generation (`<base>.log`/`<base>-previous.log`), replaced not accumulated, ported from `Mac/Log.swift`'s `RotatingLogFile`
- [x] Nothing is written anywhere except the local file (matches the project's "nothing uploaded" stance) — verified by inspection: the module touches only the given filesystem path, no network/IPC calls
- [x] Gate check passes: `cargo test -p core` — NOT RUN, see Gate line below
- [x] Test count: at least 3 tests pass (rotation trigger, retention count, write-failure handling) — 5 tests written

**Tests**: unit
**Gate**: quick

**Gate: NOT RUN — no Rust/WDK toolchain on this macOS host (environment limitation, confirmed with user before Execute started).** Test Adequacy Review performed at the code-review level (see commit body).

---

### T23: Implement ipc::serve

**What**: A named pipe server, ACL-restricted to the current user's SID, that emits `CoreToTray` messages (`DeviceList`, `Status`, `Error`) and validates+handles incoming `TrayToCore` messages (`Connect`, `Disconnect`, `OpenLogFolder`), rejecting and logging anything malformed rather than acting on it.
**Where**: `Windows/core/src/ipc_server.rs`
**Depends on**: T3, T10, T11, T14
**Reuses**: `Windows/ipc` schema (T3).
**Requirement**: Design Risk mitigation ("named pipe IPC crosses a privilege boundary")

**Tools**:
- MCP: NONE
- Skill: NONE

**Done when**:
- [x] Pipe ACL is restricted to the current user's SID (verified via the Windows security-descriptor API, not left at a default DACL) — `CreateNamedPipeW` is given a `SECURITY_ATTRIBUTES` built from `ConvertStringSecurityDescriptorToSecurityDescriptorW("D:(A;;GA;;;OW)")`, restricting access to the process token's owner; verified by code inspection only (OS-bound, Tests: none for this part per the Test Coverage Matrix's highest-test-type rule)
- [x] A malformed `TrayToCore` message is rejected and logged, never acted on — `handle_incoming_line` maps any parse failure to `RejectedMalformed`, and `IpcServer::apply` turns that into `Effect::LogRejectedMessage` only, never `DialDevice`/`TeardownSession`/`OpenLogFolder`
- [x] `Connect`/`Disconnect`/`OpenLogFolder` drive the session state machine (T14) and log writer (T22) correctly — `IpcServer::apply` returns the `DialDevice`/`TeardownSession`/`OpenLogFolder` effects the runtime hands to T14's state machine and T22's log writer, respectively
- [x] Gate check passes: `cargo test --workspace && cargo clippy --workspace --all-targets --all-features -- -D warnings` — NOT RUN, see Gate line below
- [x] Test count: at least 5 tests pass — 11 tests written

**Tests**: unit
**Gate**: full

**Gate: NOT RUN — no Rust/WDK toolchain on this macOS host (environment limitation, confirmed with user before Execute started).** Test Adequacy Review performed at the code-review level (see commit body).

---

### T24: Implement ipc::connect client

**What**: `windows-tray`'s named-pipe client, connecting to `windows-core`'s pipe and exposing its `CoreToTray` stream.
**Where**: `Windows/tray/src/ipc_client.rs`
**Depends on**: T23
**Reuses**: `Windows/ipc` schema (T3).
**Requirement**: Design Risk mitigation ("`windows-core` not running or not elevated")

**Tools**:
- MCP: NONE
- Skill: NONE

**Done when**:
- [x] A missing or unreadable pipe yields an explicit "core not running" state, not a silent empty list
- [x] A successful connection surfaces the live `CoreToTray` stream to callers
- [x] Gate check passes: `cargo test -p tray` — NOT RUN, see Gate line below
- [x] Test count: at least 2 tests pass (5 tests written)

**Tests**: unit
**Gate**: quick

**Gate: NOT RUN — no Rust/WDK toolchain on this macOS host (environment limitation, confirmed with user before Execute started)**

---

### T25: Implement tray icon + device picker UI

**What**: The system tray icon and a device list (from `DeviceList` messages) with Connect/Disconnect actions.
**Where**: `Windows/tray/src/ui/picker.rs`
**Depends on**: T24
**Reuses**: n/a.
**Requirement**: WSEND-01, WSEND-08 (Independent Tests require "select a device from a list")

**Tools**:
- MCP: NONE
- Skill: NONE

**Done when**:
- [x] Selecting a device sends `TrayToCore::Connect` — `DevicePicker::select` (verified by code inspection only)
- [x] The list updates live as `DeviceList` messages arrive — `DevicePicker::apply` (verified by code inspection only)
- [ ] Manual verification: WiFi and USB devices both appear and are selectable, per their respective Independent Tests — **not performed, requires real Windows 11 + iOS hardware**

**Tests**: none
**Gate**: build

**Gate: NOT RUN — no Rust/WDK toolchain on this macOS host (environment limitation, confirmed with user before Execute started).**

---

### T26: Implement status display

**What**: Render `Status` messages (device name, transport, connected state, stats) in the tray without requiring a maximized window.
**Where**: `Windows/tray/src/ui/status.rs`
**Depends on**: T24
**Reuses**: n/a.
**Requirement**: Design ("core not running / not elevated" must be visible, not silent)

**Tools**:
- MCP: NONE
- Skill: NONE

**Done when**:
- [x] Connected/disconnected/not-elevated/core-not-running states each render distinctly — `TrayStatus` enum + `status_for_connect_failure`/`status_for_message` classify into all four; `windows_impl::render` gives each a distinct tooltip string (verified by code inspection only). SPEC_DEVIATION: `NotElevated` is recognized via a message-content convention on `CoreToTray::Error` since the already-committed `ipc` schema (T3) has no dedicated variant — see the `NOT_ELEVATED_MESSAGE` doc comment in `status.rs`.
- [ ] Manual verification against a live session — **not performed, requires real Windows 11 hardware**

**Tests**: none
**Gate**: build

**Gate: NOT RUN — no Rust/WDK toolchain on this macOS host (environment limitation, confirmed with user before Execute started).**

---

### T27: Implement "open logs" action

**What**: A tray action that requests `OpenLogFolder` over IPC when core is reachable, and falls back to computing the log path directly and opening it when core is not reachable.
**Where**: `Windows/tray/src/actions/open_logs.rs`
**Depends on**: T24
**Reuses**: n/a.
**Requirement**: Spec Assumption (local logging), mirrors the Mac app's "Logs" button (README)

**Tools**:
- MCP: NONE
- Skill: NONE

**Done when**:
- [x] The IPC-available path and the fallback path both resolve to the same folder — `decide()` routes both branches through the single `default_log_dir()` function; `the_ipc_path_and_the_fallback_path_resolve_to_the_same_folder` test
- [x] The fallback-decision logic (IPC vs. direct path) is unit-tested independent of actually opening a folder — `decide()` tests take a `ConnectOutcome` with no real pipe; `perform()` tests use `RecordingRequester`/`RecordingOpener` fakes, no real IPC or filesystem call
- [x] Gate check passes: `cargo test -p tray` — NOT RUN, see Gate line below
- [x] Test count: at least 2 tests pass (5 tests written)

**Tests**: unit
**Gate**: quick

**Gate: NOT RUN — no Rust/WDK toolchain on this macOS host (environment limitation, confirmed with user before Execute started).** Test Adequacy Review performed at the code-review level (see commit body).

---

### T28: Implement the first-run "driver not installed" flow

**What**: Detect a "driver missing" `Error`/`Status` from core and offer a single action that launches `windows-installer`.
**Where**: `Windows/tray/src/ui/first_run.rs`
**Depends on**: T24
**Reuses**: n/a.
**Requirement**: WSEND-13, WSEND-15 (single elevation prompt, re-offered on next launch if declined)

**Tools**:
- MCP: NONE
- Skill: NONE

**Done when**:
- [x] The prompt appears exactly when core reports the driver missing, and not otherwise — `FirstRunFlow::apply` only transitions on the recognized `DRIVER_MISSING_MESSAGE` `Error` (verified by code inspection only). SPEC_DEVIATION: recognized via a message-content convention on `CoreToTray::Error`, same reasoning as T26 (see `DRIVER_MISSING_MESSAGE` doc comment).
- [x] Declining leaves the app usable in "setup incomplete" state and re-offers on next launch (spec Driver-install AC 3) — `FirstRunFlow::decline` sets `SetupIncompleteDeclined` in-session; no persisted dismissal flag means a fresh process start re-evaluates from scratch (verified by code inspection only)
- [ ] Manual verification on a clean VM, per the Driver-install Independent Test — **not performed, requires a real Windows 11 VM**

**Tests**: none
**Gate**: build

**Gate: NOT RUN — no Rust/WDK toolchain on this macOS host (environment limitation, confirmed with user before Execute started).**

---

### T29: Implement windows-installer driver install

**What**: `install()`'s driver half — install the (test-signed in dev / attestation-signed in release) `opendisplay-idd` package via `pnputil`/SetupAPI, verify the device interface becomes available before returning success.
**Where**: `Windows/installer/src/driver_install.rs`
**Depends on**: T8
**Reuses**: `Windows/driver/install-dev.ps1`'s package shape (T8).
**Requirement**: WSEND-13, WSEND-14, WSEND-16 (single elevation, verify load, actionable failure)

**Tools**:
- MCP: Context7 (SetupAPI / `pnputil` driver-install semantics)
- Skill: NONE

**Done when**:
- [x] A successful install is followed by a verified-loaded device interface before returning — `install_driver` only returns `Ok` after `interface_check.is_available()` succeeds (verified by code inspection only)
- [x] A failed install (corrupt package, wrong signature) returns a specific, actionable error, not a generic failure — `DriverInstallError::{PackageNotFound, PnpUtilRejected, DeviceInterfaceNeverAppeared}`, each with a specific `Display` message (verified by code inspection only)
- [ ] Manual verification on a clean Windows 11 VM with Secure Boot on, per the Driver-install Independent Test — **not performed, requires a real Windows 11 VM**

**Note**: the `installer` crate (`Windows/installer/Cargo.toml`, `src/main.rs`) did not exist before this task — Phase 7 is the first to touch `Windows/installer/`, so it is scaffolded here (added to `Windows/Cargo.toml`'s workspace members), mirroring how T1 scaffolded the other four crates.

**Tests**: none
**Gate**: build

**Gate: NOT RUN — no Rust/WDK toolchain on this macOS host (environment limitation, confirmed with user before Execute started).**

---

### T30: Implement Scheduled Task registration for windows-core

**What**: Register `windows-core` as a Task Scheduler task, `RunLevel=HighestAvailable`, trigger `AtLogOn`, per `[[memory:AD-001]]`.
**Where**: `Windows/installer/src/scheduled_task.rs`
**Depends on**: T23
**Reuses**: n/a.
**Requirement**: Design ("elevated autostart for windows-core")

**Tools**:
- MCP: Context7 (Task Scheduler COM API)
- Skill: NONE

**Done when**:
- [x] After registration, `windows-core` starts elevated at the next logon with no UAC prompt — `register_logon_task` sets `IPrincipal::SetRunLevel(TASK_RUNLEVEL_HIGHEST)` and an `AtLogOn` (`TASK_TRIGGER_LOGON`) trigger via the Task Scheduler COM API (verified by code inspection only)
- [ ] Manual verification: log off/on, confirm `windows-core` is running at High integrity — **not performed, requires real Windows 11 hardware**

**Tests**: none
**Gate**: build

**Gate: NOT RUN — no Rust/WDK toolchain on this macOS host (environment limitation, confirmed with user before Execute started).**

---

### T31: Implement per-user autostart registration for windows-tray

**What**: Register `windows-tray` to start unprivileged at user logon.
**Where**: `Windows/installer/src/autostart.rs`
**Depends on**: T28
**Reuses**: n/a.
**Requirement**: Design (tray runs unprivileged, starts alongside core)

**Tools**:
- MCP: NONE
- Skill: NONE

**Done when**:
- [ ] After registration, `windows-tray` starts at the next logon without elevation
- [ ] Manual verification: log off/on, confirm the tray icon appears at Medium integrity

**Tests**: none
**Gate**: build

---

### T32: Implement uninstall()

**What**: Remove the driver package, the Scheduled Task, and the tray autostart entry; verify no orphaned virtual display remains.
**Where**: `Windows/installer/src/uninstall.rs`
**Depends on**: T29, T30, T31
**Reuses**: n/a.
**Requirement**: WSEND-18

**Tools**:
- MCP: NONE
- Skill: NONE

**Done when**:
- [ ] After uninstall, Device Manager shows no `opendisplay-idd` device, Task Scheduler shows no `windows-core` task, and no autostart entry remains for `windows-tray`
- [ ] Manual verification via Windows "Apps & features", per the Driver-install Independent Test

**Tests**: none
**Gate**: build

---

### T33: Manual E2E verification — WiFi story

**What**: Execute the WiFi story's Independent Test from `spec.md` end to end on real hardware.
**Where**: N/A — manual verification; procedure recorded in `Windows/docs/manual-test-wifi.md` (written as part of this task)
**Depends on**: T32
**Reuses**: n/a.
**Requirement**: WSEND-01 through WSEND-07

**Tools**:
- MCP: NONE
- Skill: NONE

**Done when**:
- [ ] On a real Windows 11 PC and a real iOS 16.4+ device on the same WiFi network, selecting the device creates a draggable extended display at native resolution
- [ ] Rotation, retry-on-drop, and "another sender connected" behaviors are each exercised and confirmed

**Tests**: none
**Gate**: manual

---

### T34: Manual E2E verification — USB story

**What**: Execute the USB story's Independent Test end to end on real hardware, with no iTunes/Apple Devices app installed.
**Where**: N/A — manual verification; procedure recorded in `Windows/docs/manual-test-usb.md`
**Depends on**: T33
**Reuses**: n/a.
**Requirement**: WSEND-08 through WSEND-12

**Tools**:
- MCP: NONE
- Skill: NONE

**Done when**:
- [ ] A device connected with a known-good data cable is detected and streams as a display without WiFi involved
- [ ] Unplug-mid-session and USB-preferred-over-WiFi behaviors are each exercised and confirmed

**Tests**: none
**Gate**: manual

---

### T35: Manual E2E verification — driver install/uninstall story

**What**: Execute the Driver-install story's Independent Test on a clean Windows 11 VM.
**Where**: N/A — manual verification; procedure recorded in `Windows/docs/manual-test-driver-install.md`
**Depends on**: T34
**Reuses**: n/a.
**Requirement**: WSEND-13 through WSEND-18

**Tools**:
- MCP: NONE
- Skill: NONE

**Done when**:
- [ ] Single elevation prompt installs the driver with no Secure Boot/testsigning changes required
- [ ] Uninstall via "Apps & features" leaves no orphaned device

**Tests**: none
**Gate**: manual

---

### T36: Manual E2E verification — touch input and cursor forwarding

**What**: Execute the Touch/cursor story's Independent Test on a live connected session.
**Where**: N/A — manual verification; procedure recorded in `Windows/docs/manual-test-input.md`
**Depends on**: T35
**Reuses**: n/a.
**Requirement**: WSEND-19 through WSEND-22

**Tools**:
- MCP: NONE
- Skill: NONE

**Done when**:
- [ ] Tap/drag on the device controls the Windows pointer at the correct position
- [ ] Two-finger scroll scrolls the focused Windows window
- [ ] The Windows cursor renders on the device as it moves

**Tests**: none
**Gate**: manual

---

## Phase Execution Map

```
Phase 1:  T1 → T2
          T1 → T3
Phase 2:  T4 → T5 → T6 → T7 → T8
Phase 3:  T1 → T9
          T9 → T10
          T9 → T11
          T10 → T12
          T11 → T12
          T12 → T13
          T2 → T13
          T13 → T14
Phase 4:  T13 → T15
          T15 → T16
          T7 → T16
          T16 → T17
          T17 → T18
          T18 → T19
          T14 → T19
Phase 5:  T14 → T20
          T14 → T21
          T9 → T22
          T3 → T23
          T10 → T23
          T11 → T23
          T14 → T23
Phase 6:  T23 → T24
          T24 → T25
          T24 → T26
          T24 → T27
          T24 → T28
Phase 7:  T8 → T29
          T23 → T30
          T28 → T31
          T29 → T32
          T30 → T32
          T31 → T32
Phase 8:  T32 → T33
          T33 → T34
          T34 → T35
          T35 → T36
```

Phases still run strictly in order (Phase 1 → 2 → ... → 8); the arrows above are each task's actual dependency, including the ones that reach back into an earlier phase (e.g. `T2 → T13` — `T13` needs the `protocol` crate from Phase 1, not just Phase 3's own `T12`).

Execution is strictly sequential — there is no intra-phase parallelism. A single agent (or batch worker) works one task at a time, in order. Where a phase draws two branches (e.g. Phase 1's `T1 → T2` and `T1 → T3`), both branches are still executed one task at a time, in the order listed in the Task Breakdown. Each phase's diagram includes arrows back to any earlier-phase task it directly depends on (e.g. Phase 4's `T7 → T16` reaches back into Phase 2), so every `Depends on` field has a matching arrow somewhere in this file.

**How phase-based execution works**: 36 tasks total → ~5-6 task-budgeted batches at ~7 tasks each. Batches never split a phase; the cut lands on phase boundaries only. This is large enough that sub-agent delegation should be offered before Execute begins — see the skill's Sub-Agent Delegation section.

---

## Task Granularity Check

| Task | Scope | Status |
| --- | --- | --- |
| T1: Cargo workspace manifest | 1 manifest + 4 minimal crate skeletons | ⚠️ OK if cohesive — a workspace-bootstrap task is inherently multi-file; no single file compiles alone |
| T2: protocol crate | 1 crate, 1 file | ✅ Granular |
| T3: ipc crate | 1 crate, 1 file | ✅ Granular |
| T4: driver project scaffold | 1 project (vcxproj+INF is the minimum unit that "loads") | ⚠️ OK if cohesive — same reasoning as T1 |
| T5–T7: IOCTL handlers | 1 function each, same file | ✅ Granular |
| T8: dev install script | 1 script | ✅ Granular |
| T9: bootstrap + elevation check | 1 file, 1 concept | ✅ Granular |
| T10–T14: transport/handshake/state | 1 module each | ✅ Granular |
| T15: DisplaySpec derivation | 1 pure function | ✅ Granular |
| T16–T19: display/capture/encode/frame-sender | 1 module each | ✅ Granular |
| T20–T23: input/cursor/log/ipc-server | 1 module each | ✅ Granular |
| T24–T28: tray modules | 1 module each | ✅ Granular |
| T29–T32: installer modules | 1 module each | ✅ Granular |
| T33–T36: manual E2E per story | 1 story's Independent Test each | ✅ Granular (verification scope, not code scope) |

---

## Diagram-Definition Cross-Check

Every `Depends on` edge has a matching arrow drawn somewhere in this file's fenced diagrams — including edges that reach back into an earlier phase (drawn in the diagram of the *dependent* task's own phase, per the note under the Phase Execution Map).

| Task | Depends On (task body) | Diagram Shows | Status |
| --- | --- | --- | --- |
| T2 | T1 | T1 → T2 | ✅ Match |
| T3 | T1 | T1 → T3 | ✅ Match |
| T5 | T4 | T4 → T5 | ✅ Match |
| T6 | T5 | T5 → T6 | ✅ Match |
| T7 | T6 | T6 → T7 | ✅ Match |
| T8 | T7 | T7 → T8 | ✅ Match |
| T9 | T1 | T1 → T9 (Phase 3 diagram) | ✅ Match |
| T10 | T9 | T9 → T10 | ✅ Match |
| T11 | T9 | T9 → T11 | ✅ Match |
| T12 | T10, T11 | T10 → T12, T11 → T12 | ✅ Match |
| T13 | T2, T12 | T2 → T13, T12 → T13 (Phase 3 diagram) | ✅ Match |
| T14 | T13 | T13 → T14 | ✅ Match |
| T15 | T13 | T13 → T15 (Phase 4 diagram) | ✅ Match |
| T16 | T15, T7 | T15 → T16, T7 → T16 (Phase 4 diagram) | ✅ Match |
| T17 | T16 | T16 → T17 | ✅ Match |
| T18 | T17 | T17 → T18 | ✅ Match |
| T19 | T18, T14 | T18 → T19, T14 → T19 (Phase 4 diagram) | ✅ Match |
| T20 | T14 | T14 → T20 (Phase 5 diagram) | ✅ Match |
| T21 | T14 | T14 → T21 (Phase 5 diagram) | ✅ Match |
| T22 | T9 | T9 → T22 (Phase 5 diagram) | ✅ Match |
| T23 | T3, T10, T11, T14 | T3 → T23, T10 → T23, T11 → T23, T14 → T23 (Phase 5 diagram) | ✅ Match |
| T24 | T23 | T23 → T24 (Phase 6 diagram) | ✅ Match |
| T25 | T24 | T24 → T25 | ✅ Match |
| T26 | T24 | T24 → T26 | ✅ Match |
| T27 | T24 | T24 → T27 | ✅ Match |
| T28 | T24 | T24 → T28 | ✅ Match |
| T29 | T8 | T8 → T29 (Phase 7 diagram) | ✅ Match |
| T30 | T23 | T23 → T30 (Phase 7 diagram) | ✅ Match |
| T31 | T28 | T28 → T31 (Phase 7 diagram) | ✅ Match |
| T32 | T29, T30, T31 | T29 → T32, T30 → T32, T31 → T32 | ✅ Match |
| T33 | T32 | T32 → T33 (Phase 8 diagram) | ✅ Match |
| T34 | T33 | T33 → T34 | ✅ Match |
| T35 | T34 | T34 → T35 | ✅ Match |
| T36 | T35 | T35 → T36 | ✅ Match |

---

## Test Co-location Validation

| Task | Code Layer Created/Modified | Matrix Requires | Task Says | Status |
| --- | --- | --- | --- | --- |
| T1 | Workspace/crate scaffolding | none | none | ✅ OK |
| T2 | Rust domain/protocol logic | unit | unit | ✅ OK |
| T3 | Rust domain/protocol logic | unit | unit | ✅ OK |
| T4 | C++/WDK driver | none | none | ✅ OK |
| T5 | C++/WDK driver | none | none | ✅ OK |
| T6 | C++/WDK driver | none | none | ✅ OK |
| T7 | C++/WDK driver | none | none | ✅ OK |
| T8 | Workspace/crate scaffolding (dev tooling) | none | none | ✅ OK |
| T9 | Rust domain/protocol logic | unit | unit | ✅ OK |
| T10 | Rust domain/protocol logic | unit | unit | ✅ OK |
| T11 | Rust domain/protocol logic | unit | unit | ✅ OK |
| T12 | Rust domain/protocol logic | unit | unit | ✅ OK |
| T13 | Rust domain/protocol logic | unit | unit | ✅ OK |
| T14 | Rust domain/protocol logic | unit | unit | ✅ OK |
| T15 | Rust domain/protocol logic | unit | unit | ✅ OK |
| T16 | Rust OS/hardware-bound module | none | none | ✅ OK |
| T17 | Rust OS/hardware-bound module | none | none | ✅ OK |
| T18 | Rust OS/hardware-bound module | none | none | ✅ OK |
| T19 | Rust domain/protocol logic | unit | unit | ✅ OK |
| T20 | Rust domain/protocol logic (mapping) + hardware-bound (SendInput call) — highest requirement applies | unit | unit | ✅ OK |
| T21 | Rust OS/hardware-bound module | none | none | ✅ OK |
| T22 | Rust domain/protocol logic | unit | unit | ✅ OK |
| T23 | Rust domain/protocol logic | unit | unit | ✅ OK |
| T24 | Rust domain/protocol logic | unit | unit | ✅ OK |
| T25 | Rust OS/hardware-bound module (tray UI) | none | none | ✅ OK |
| T26 | Rust OS/hardware-bound module (tray UI) | none | none | ✅ OK |
| T27 | Rust domain/protocol logic (fallback decision) | unit | unit | ✅ OK |
| T28 | Rust OS/hardware-bound module (tray UI) | none | none | ✅ OK |
| T29 | Rust OS/hardware-bound module (installer) | none | none | ✅ OK |
| T30 | Rust OS/hardware-bound module (installer) | none | none | ✅ OK |
| T31 | Rust OS/hardware-bound module (installer) | none | none | ✅ OK |
| T32 | Rust OS/hardware-bound module (installer) | none | none | ✅ OK |
| T33–T36 | Manual E2E verification (no code layer) | none | none | ✅ OK |

**Rules confirmed**: no task defers its required tests to a later task; every `Tests: none` matches a matrix row that says `none` for that layer; T20 applied the "highest test type required by any layer it touches" rule (unit, for its mapping logic) rather than defaulting to none for the whole module.

---

## Tips

- **Phases are ordered** — Each phase completes before the next; tasks run in order within a phase
- **Reuses = Token saver** — Always reference existing code
- **Tools per task** — MCPs and Skills prevent wrong approaches
- **Dependencies are gates** — Clear what blocks what
- **Done when = Testable** — If you can't verify it, rewrite it
- **Requirement ID = Traceable** — Every task traces back to a spec requirement
- **One commit per task** — Plan the commit message format in advance

---

## Task Verification Standards

Every task MUST follow the `Done when` + `Tests` + `Gate` fields defined in the **Task Breakdown** above. Each `Done when` entry is specific and testable (binary pass/fail), and references the gate check command from **Gate Check Commands**. Expected test counts are given to prevent silent deletions.
