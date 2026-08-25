# Manual test: opendisplay-idd driver skeleton (T4)

Not performed — requires a real Windows 11 machine with the WDK
installed; this repository's implementation work happened on macOS with
no WDK/MSBuild available (see `.specs/features/windows-sender/tasks.md`
T4's environment-limitation note). Record results here the first time
this is run on real hardware.

## Prerequisites

- Windows 11 with the WDK and Visual Studio's driver workload installed.
- `bcdedit /set testsigning on`, then reboot (local dev only — the
  production install path uses attestation signing and does not require
  this; see `Windows/driver/install-dev.ps1`, T8).

## Steps

1. Build `Windows/driver/opendisplay-idd.vcxproj` (Debug|x64).
   - [ ] Build succeeds with no errors.
2. Run `Windows/driver/install-dev.ps1` to test-sign and install the
   package (T8; script is not implemented until Phase 2 finishes T8 —
   if run before then, install manually with `pnputil /add-driver
   opendisplay-idd.inf /install`).
   - [ ] Install reports success.
3. Open Device Manager.
   - [ ] "OpenDisplay Virtual Display" appears under Display adapters (or
     an equivalent node) with no yellow-bang load error.
4. Open Windows Display Settings.
   - [ ] No extra monitor is listed yet — the skeleton driver enumerates
     zero monitors until `IOCTL_OPENDISPLAY_CREATE_DISPLAY` (T5) is
     issued by `windows-core`.
5. Uninstall via `Windows/driver/install-dev.ps1 -Uninstall` (or
   `pnputil /delete-driver <oemNN.inf> /uninstall`).
   - [ ] Device Manager node is gone; no orphaned device remains.

## Result

Not run. Pending real Windows 11 hardware.
