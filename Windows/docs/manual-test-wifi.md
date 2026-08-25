# Manual test: WiFi extension story (T33, WSEND-01..07)

Not performed — requires a real Windows 11 PC and a real iOS 16.4+ device
on the same WiFi network; this repository's implementation work happened
on macOS with no Rust toolchain available (see
`.specs/features/windows-sender/tasks.md`'s environment-limitation note
on every T1-T32 task). Record results here the first time this is run on
real hardware, after building `windows-core`/`windows-tray` and running
`windows-installer` on that hardware.

**Independent Test (spec.md, P1: Extend the Windows desktop to an
iPhone/iPad over WiFi):** On a Windows 11 PC and an iOS 16.4+ device on
the same WiFi network, with the OpenDisplay receiver app open, pick the
device from the Windows app's list and confirm a new display appears in
Windows Display Settings that can be dragged windows onto, at the
device's native resolution.

## Prerequisites

- `windows-installer` already run successfully on this machine (driver
  installed, `windows-core` and `windows-tray` registered — see
  `manual-test-driver-install.md`, T35).
- iPhone/iPad on iOS 16.4+ with the OpenDisplay receiver app open, on the
  same WiFi network as the Windows PC.

## Steps

1. Open the `windows-tray` picker.
   - [ ] The iOS device appears in the WiFi device list within a few
     seconds (WSEND-01's discovery path — `mdns-sd` browsing
     `_opensidecar._tcp`).
2. Select the device.
   - [ ] The `hello`/`welcome` handshake completes (WSEND-01) and a new
     display appears in Windows Display Settings.
   - [ ] The display's resolution matches the device's native panel size
     at its reported `@2x`/`@3x` scale (WSEND-02).
   - [ ] A window can be dragged onto the new display and is visible on
     the device's screen.
3. Rotate the device between portrait and landscape.
   - [ ] The virtual display rebuilds at the new native resolution with
     no crash (WSEND-04).
4. Disconnect WiFi on the device (or move out of range) mid-session.
   - [ ] `windows-core` detects the drop and retries automatically until
     the device reappears (WSEND-06).
5. From a second sender (or a second `windows-core` instance, if
   available), connect to the same device while the first session is
   still active.
   - [ ] The first session's `windows-tray` status shows "another sender
     is already connected" and does not keep retrying that device
     (WSEND-07).

## Result

Not run. Pending real Windows 11 hardware and a real iOS 16.4+ device.
