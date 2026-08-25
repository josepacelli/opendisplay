// opendisplay-idd: the IOCTL_OPENDISPLAY_{CREATE,RESIZE,DESTROY}_DISPLAY
// handlers (T5, T6, T7 each add one). See Driver.h for the
// structural-reference and verification-status note.
#pragma once

#include <windows.h>
#include <wdf.h>
#include <iddcx.h>
#include "Device.h"
#include "Ioctl.h"

// Creates the single virtual monitor via IddCxMonitorArrival. Implemented
// by T5.
NTSTATUS OpenDisplayIddCreateDisplay(
    _In_ WDFDEVICE Device,
    _In_ POPENDISPLAY_DISPLAY_PARAMS Params
);

// Rebuilds the existing monitor's mode at a new resolution without a full
// destroy/recreate visible to Windows. Implemented by T6.
NTSTATUS OpenDisplayIddResizeDisplay(
    _In_ WDFDEVICE Device,
    _In_ POPENDISPLAY_DISPLAY_PARAMS Params
);

// Removes the monitor via IddCxMonitorDeparture. Implemented by T7.
NTSTATUS OpenDisplayIddDestroyDisplay(
    _In_ WDFDEVICE Device
);
