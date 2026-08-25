# Manual test: touch input and cursor forwarding story (T36, WSEND-19..22)

Not performed — requires a live connected session between a real Windows
11 PC and a real iOS 16.4+ device; this repository's implementation work
happened on macOS with no Rust toolchain available (see
`.specs/features/windows-sender/tasks.md`'s environment-limitation note
on every T1-T32 task). Record results here the first time this is run on
real hardware, after `manual-test-wifi.md` or `manual-test-usb.md` has
confirmed a working session.

**Independent Test (spec.md, P1: Touch input and cursor forwarding):**
With a device connected as a display, tap and drag on the phone screen
and confirm the Windows pointer moves and clicks/drags at the same
normalized position; two-finger-scroll on the phone and confirm the
focused Windows window scrolls; observe the Windows cursor icon rendered
on the phone as it moves.

## Prerequisites

- A device connected and streaming as a display (either
  `manual-test-wifi.md` or `manual-test-usb.md` completed successfully
  first).

## Steps

1. Tap once on the phone's screen, at a known point on the virtual
   display (e.g. a specific icon or window title bar).
   - [ ] The Windows pointer moves to and clicks at the corresponding
     point on the virtual display (WSEND-19).
2. Press and drag on the phone screen across the virtual display.
   - [ ] The Windows pointer performs a matching drag (mouse-down, move,
     mouse-up) — e.g. a window can be dragged by its title bar this way.
3. Two-finger-scroll on the phone over a scrollable window.
   - [ ] The focused Windows window scrolls in the matching direction
     (WSEND-20).
4. Move the Windows mouse cursor around the virtual display (using a
   physical mouse on the Windows PC).
   - [ ] The current cursor position and icon are visible on the phone,
     updating live as the cursor moves (WSEND-21).
5. Send a `pencil`/`proximity` control message from a `pv >= 3` test
   peer (or use a real Apple Pencil against the device, if available).
   - [ ] `windows-core` accepts the message without erroring and simply
     ignores it — no crash, no unexpected input injected (WSEND-22;
     Apple Pencil support is out of scope for this spec).

## Result

Not run. Pending real Windows 11 hardware and a real iOS 16.4+ device.
