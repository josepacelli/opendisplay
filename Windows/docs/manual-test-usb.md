# Manual test: USB extension story (T34, WSEND-08..12)

Not performed — requires a real Windows 11 PC and a real iOS 16.4+ device
connected by a known-good USB data cable; this repository's
implementation work happened on macOS with no Rust toolchain available
(see `.specs/features/windows-sender/tasks.md`'s environment-limitation
note on every T1-T32 task). Record results here the first time this is
run on real hardware.

**Independent Test (spec.md, P1: Extend the Windows desktop to an
iPhone/iPad over USB):** Connect an iOS 16.4+ device to the Windows PC
with a known-good data cable, with no iTunes/Apple Devices app installed,
and confirm the device is picked up and streams as a display without any
WiFi network involved.

## Prerequisites

- `windows-installer` already run successfully on this machine.
- A known-good USB data (not charge-only) cable.
- No iTunes / Apple Devices app installed on the Windows PC — this is
  the whole point of the `idevice` crate choice (design.md Tech
  Decisions); if it is installed, uninstall it first so the test proves
  the intended no-dependency path.
- WiFi disabled on the test device, to isolate the USB path.

## Steps

1. Plug the device in with the data cable.
   - [ ] The device is detected via `idevice`'s usbmuxd client within a
     few seconds (WSEND-08), with no iTunes/Apple Devices install
     prompt.
2. Select the device in `windows-tray`.
   - [ ] The handshake completes through the usbmuxd tunnel to port 9000
     and streams as a display (WSEND-09), same pipeline as the WiFi
     story.
3. With the device still connected over USB, also join it to the same
   WiFi network as the Windows PC.
   - [ ] The active session stays on USB, not WiFi (WSEND-11 — USB
     preferred when both are available for the same device).
4. Swap in a charge-only (non-data) USB cable.
   - [ ] No device is detected — behaves identically to "nothing
     attached" (WSEND-10), matching the project's existing USB
     troubleshooting guidance.
5. Reconnect with the data cable, start a session, then physically
   unplug the cable mid-session.
   - [ ] `windows-core` treats this as a connection drop and applies the
     same retry policy as the WiFi story (WSEND-12).

## Result

Not run. Pending real Windows 11 hardware and a real iOS 16.4+ device.
