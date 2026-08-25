; OpenDisplay Windows sender installer (Inno Setup 6).
;
; Packages the release binaries this workspace already builds
; (Windows/target/release/{core,tray,installer}.exe) into a single
; Setup.exe for Windows 11 64-bit: copies files to Program Files, then
; elevates once to run windows-installer.exe (this repo's own composed
; install() — driver/Scheduled Task/autostart registration, see
; Windows/installer/src/main.rs), and reverses it on uninstall.
;
; The driver package (opendisplay-idd, needs the WDK) isn't built yet, so
; it is not packaged here — windows-installer.exe skips that step
; gracefully when its .inf is absent (see run_install's inf_path.exists()
; check) rather than failing the whole install over it. Add
; "Source: ...\opendisplay-idd.*; DestDir: {app}" once it exists; no other
; change needed here.
;
; Build: run this file with Inno Setup's ISCC.exe after
; `cargo build --workspace --release` in Windows/. Output lands in
; Windows/packaging/Output/OpenDisplay-Setup.exe (gitignored).

#define MyAppName "OpenDisplay"
#define MyAppVersion "0.1.0"
#define MyAppPublisher "OpenDisplay"
#define MyAppURL "https://github.com/josepacelli/opendisplay"
#define ReleaseDir "..\target\release"

[Setup]
AppId={{7F6C8B0B-6B5B-4A6C-9F0D-6D3A6D8B6C10}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
DefaultDirName={autopf}\{#MyAppName}
DefaultGroupName={#MyAppName}
; Windows 11 64-bit only, per the ask — this is a native x64 build with
; no 32-bit fallback.
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
; windows-installer.exe needs elevation to register the Scheduled Task /
; driver / write HKCU\...\Run for the target user, and installing into
; Program Files needs it too.
PrivilegesRequired=admin
OutputDir=Output
OutputBaseFilename=OpenDisplay-Setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
DisableProgramGroupPage=yes
UninstallDisplayIcon={app}\tray.exe
; No signing certificate configured for this build; Setup.exe and its
; installed binaries are unsigned, per this environment's current state
; (see Windows/docs/windows-hardware-runbook.md for the real production
; signing story).
SetupIconFile={#ReleaseDir}\..\..\tray\assets\app.ico

[Files]
Source: "{#ReleaseDir}\core.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#ReleaseDir}\tray.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#ReleaseDir}\installer.exe"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\tray.exe"
Name: "{group}\Uninstall {#MyAppName}"; Filename: "{uninstallexe}"

[Run]
; windows-installer.exe with no argument = install() (see main.rs): driver
; install (skipped gracefully if not packaged), Scheduled Task
; registration for core.exe, and HKCU autostart registration for tray.exe.
Filename: "{app}\installer.exe"; Parameters: ""; Flags: runhidden waituntilterminated; StatusMsg: "Setting up OpenDisplay..."
; Optional immediate launch. windows-core itself only starts elevated via
; the Scheduled Task at next logon (by design, see T30/FIX1's doc
; comments), so tray.exe alone may show "core not running" until then —
; expected, not a bug.
Filename: "{app}\tray.exe"; Description: "Launch OpenDisplay now"; Flags: nowait postinstall skipifsilent unchecked

[UninstallRun]
; Best-effort: stop both processes before removing files/registration so
; uninstall doesn't leave a locked binary or a stale running instance.
Filename: "{cmd}"; Parameters: "/C taskkill /F /IM core.exe /T"; Flags: runhidden waituntilterminated; RunOnceId: "KillCore"
Filename: "{cmd}"; Parameters: "/C taskkill /F /IM tray.exe /T"; Flags: runhidden waituntilterminated; RunOnceId: "KillTray"
Filename: "{app}\installer.exe"; Parameters: "uninstall"; Flags: runhidden waituntilterminated; RunOnceId: "RunUninstall"
