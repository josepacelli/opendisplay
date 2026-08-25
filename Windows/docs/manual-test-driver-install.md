# Manual test: driver install/uninstall story (T35, WSEND-13..18)

Not performed — requires a clean Windows 11 VM with Secure Boot on; this
repository's implementation work happened on macOS with no WDK/MSBuild
or Rust toolchain available (see
`.specs/features/windows-sender/tasks.md`'s environment-limitation note
on every T1-T32 task, and `manual-test-driver-skeleton.md` for the raw
driver-only check from T4). This test exercises the full
`windows-installer` flow, not just the driver skeleton.

**Independent Test (spec.md, P1: One-time signed driver install):** On a
clean Windows 11 VM with Secure Boot on and no prior OpenDisplay install,
run the installer, accept the single elevation prompt, and confirm a new
display adapter/device appears in Device Manager with no security
warning blocking it — then uninstall and confirm it is gone.

## Prerequisites

- A clean Windows 11 VM, Secure Boot **on**, no prior OpenDisplay
  install, no test-signing mode enabled.
- The attestation-signed release build of `opendisplay-idd` (not the
  test-signed dev build from `Windows/driver/install-dev.ps1`, T8) — or,
  until signing is set up, note that this run is against the test-signed
  build and flag the result accordingly.

## Steps

1. Run the `windows-installer` binary.
   - [ ] Exactly one UAC elevation prompt appears (WSEND-13).
   - [ ] Accepting it installs the driver package with no Secure
     Boot/testsigning changes required (WSEND-13, WSEND-16).
2. Check Device Manager.
   - [ ] `opendisplay-idd`'s device interface is present and loaded,
     with no yellow-bang error, before `windows-tray` offers any device
     connection (WSEND-14).
3. Re-run the installer on a machine where the driver package is
   deliberately corrupted (e.g. truncate the `.cat` file).
   - [ ] Install fails with a specific, actionable error message, not a
     generic failure, and does not proceed to offer device connection
     (WSEND-16).
4. Run the installer again, but decline the UAC prompt this time.
   - [ ] The app stays open in a "setup incomplete" state, explains a
     virtual display cannot be created yet, and does not crash or hang
     (spec Driver-install AC 3).
   - [ ] Relaunching the app re-offers the install prompt.
5. Log off and log back on.
   - [ ] `windows-core` starts elevated with no UAC prompt (the
     Scheduled Task registered by T30) and `windows-tray` starts
     unprivileged (T31's autostart).
6. Uninstall via Windows "Apps & features".
   - [ ] Device Manager shows no `opendisplay-idd` device, Task
     Scheduler shows no `windows-core` task, and no autostart entry
     remains for `windows-tray` (WSEND-18).

## Result

Not run. Pending a clean Windows 11 VM with Secure Boot on.
