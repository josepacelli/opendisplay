// opendisplay-idd: driver-object-level declarations.
//
// Structural reference only: Microsoft's IddSample
// (microsoft/Windows-driver-samples, video/IndirectDisplay), as pointed to
// by design.md's Tech Decisions and non-normatively by PROTOCOL.md
// Appendix B. This file is original source written for this project
// (GPL-3.0), not vendored from that sample.
//
// Written on a macOS host with no WDK installed (T4-T8's environment
// limitation — see tasks.md). API names and call shapes reflect the
// documented IddCx 1.4+ / KMDF surface; this has not been compiled or
// verified against the real WDK headers yet. That verification is exactly
// task T4's unchecked manual "Project builds under the WDK toolchain"
// Done-when item.
#pragma once

#include <windows.h>
#include <wdf.h>
#include <iddcx.h>

EXTERN_C_START

DRIVER_INITIALIZE DriverEntry;

EVT_WDF_DRIVER_DEVICE_ADD OpenDisplayIddEvtDeviceAdd;
EVT_WDF_OBJECT_CONTEXT_CLEANUP OpenDisplayIddEvtDriverContextCleanup;

EXTERN_C_END
