# Windows sender — runbook for picking this up on real Windows hardware

Everything under `Windows/` was written on macOS, with **no Rust toolchain and no WDK/MSBuild available on that host**. Every task and commit on the `windows-sender` branch says so explicitly — none of this code has ever been compiled, and no automated test has ever actually run. This doc is the checklist for the first real pass on a Windows machine: get the toolchain in place, actually run the gates, then work through the Verifier's open gaps.

Source of truth for everything below: `.specs/features/windows-sender/` (`spec.md`, `design.md`, `tasks.md`, `validation.md`) and `PROTOCOL.md` / `COMPATIBILITY.md` at the repo root.

## 0. Get the code

```powershell
git clone <this repo's remote> opendisplay
cd opendisplay
git checkout windows-sender
git pull
```

Check `.specs/features/windows-sender/validation.md` and the bottom of `tasks.md` (any `## Verifier Fix Round N` sections) for the latest state — a fix round may have landed after this runbook was written.

## 1. Install the toolchain

- **Rust**: install via [rustup](https://rustup.rs). Default stable toolchain, `x86_64-pc-windows-msvc` target (rustup's Windows default already matches).
- **WDK + Visual Studio** (for the driver only): install Visual Studio 2022 with the "Desktop development with C++" workload, then the [Windows Driver Kit (WDK)](https://learn.microsoft.com/en-us/windows-hardware/drivers/download-the-wdk) matching your Windows SDK version. You need this only to touch `Windows/driver/`.

Verify:

```powershell
cargo --version
rustc --version
```

## 2. Build and actually run the gates

From `Windows/`:

```powershell
cd Windows
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

This is the first time these commands have ever run. Expect real compile errors — the code was written and reasoned about carefully, but never checked by a compiler. Work through them file by file; `tasks.md`'s Task Breakdown section tells you which file each task touched and what it was supposed to do, so you can cross-reference intent vs. what's actually there.

Once `cargo build --workspace` and `cargo test --workspace` are clean, go back through `tasks.md` and replace every

```
Gate: NOT RUN — no Rust/WDK toolchain on this macOS host (environment limitation, confirmed with user before Execute started)
```

line with the real result (pass, with the actual test count). Do the same in `.specs/features/windows-sender/spec.md`'s Requirement Traceability table — bump `Status` from `Implementing` to `Verified` for any `WSEND-NN` row whose tests now genuinely pass.

## 3. Build the driver

```powershell
# Open Windows/driver/opendisplay-idd.vcxproj in Visual Studio, or:
msbuild Windows\driver\opendisplay-idd.vcxproj /p:Configuration=Debug /p:Platform=x64
```

Then, for local dev iteration (test-signed, not the production attestation-signed path):

```powershell
bcdedit /set testsigning on   # then reboot
Windows\driver\install-dev.ps1
```

Work through `Windows/docs/manual-test-driver-skeleton.md` first (the raw driver-only check from task T4) before the full install-flow check in step 5 below.

## 4. Run the app

Once it builds:

```powershell
cargo run -p installer   # first-run setup: installs the driver, registers windows-core (elevated, Scheduled Task) and windows-tray (autostart)
```

or run `windows-core`/`windows-tray` directly during development (`cargo run -p core`, `cargo run -p tray`) without going through the installer, if you're iterating on app logic rather than the install flow itself.

## 5. Work through the manual E2E procedures

These were written as the deliverable for tasks T33-T36 but never executed (no hardware). Each doc is a checklist tied to a spec.md story's Independent Test and its `WSEND-NN` requirement IDs — check boxes as you confirm them, and fill in each doc's "Result" section at the bottom:

- `Windows/docs/manual-test-wifi.md` — WSEND-01..07
- `Windows/docs/manual-test-usb.md` — WSEND-08..12
- `Windows/docs/manual-test-driver-install.md` — WSEND-13..18 (needs a clean Windows 11 VM, Secure Boot on)
- `Windows/docs/manual-test-input.md` — WSEND-19..22

## 6. Known open items to check first

From `.specs/features/windows-sender/validation.md` (the Verifier's report) and the commits that reference a `SPEC_DEVIATION`:

- `Windows/core/src/transport/dial.rs` — `RealDialer::dial_usb` couldn't bridge `idevice`'s async connection into a `std::net::TcpStream`-shaped type without a toolchain to check it against; it returns an explicit `io::Error` there today. This needs a real fix once you can compile and test against an actual device.
- The `ipc` crate has no dedicated "not elevated" / "driver missing" message variants — the tray recognizes these via a message-content convention on `CoreToTray::Error` rather than a schema change. Sound as a stopgap; consider a proper schema addition once you're iterating for real.
- Check `tasks.md`'s `## Verifier Fix Round N` section(s) for anything still open after the last fix→re-verify cycle — the loop is bounded to 3 iterations before escalating, so there may be a documented remaining gap rather than a clean PASS.
- `Windows/core/src/display.rs`'s `DEFAULT_REFRESH_HZ = 60` is a placeholder (neither `hello` nor `DisplaySpec` carries a real refresh rate) — fine for a first bring-up, revisit if a device's actual refresh rate matters.

## 7. Once gates are green for real

Re-run (or ask Claude Code to re-run, with the `tlc-spec-driven` skill active) the Verifier's discrimination sensor properly this time — inject a small behavior-level fault in a scratch worktree, confirm the relevant test actually fails, discard the scratch. That step was explicitly skipped on macOS (no compiler to run it with) and is the one piece of the verification that a working toolchain unlocks.
