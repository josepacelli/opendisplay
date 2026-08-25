// opendisplay-idd: per-device state and the IddCx adapter/monitor
// callbacks. See Driver.h for the structural-reference and
// verification-status note.
#pragma once

#include <windows.h>
#include <wdf.h>
#include <iddcx.h>
#include "Ioctl.h"

// One opendisplay-idd device instance manages at most one virtual
// monitor, matching the P1 single-device constraint (spec.md Edge Cases:
// "P1 has no multi-device support").
typedef struct _DEVICE_CONTEXT
{
    IDDCX_ADAPTER AdapterObject;
    BOOLEAN AdapterInitialized;

    IDDCX_MONITOR MonitorObject;
    BOOLEAN MonitorCreated;
    OPENDISPLAY_DISPLAY_PARAMS CurrentParams;
} DEVICE_CONTEXT, *PDEVICE_CONTEXT;

WDF_DECLARE_CONTEXT_TYPE_WITH_NAME(DEVICE_CONTEXT, DeviceGetContext)

// Called from OpenDisplayIddEvtDeviceAdd (Driver.cpp) to build the WDF
// device, configure it as an IddCx client, register the private device
// interface windows-core opens for the CREATE/RESIZE/DESTROY IOCTLs, and
// set up the queue that dispatches them to Display.cpp.
NTSTATUS OpenDisplayIddDeviceCreate(_Inout_ PWDFDEVICE_INIT DeviceInit);

EVT_WDF_DEVICE_D0_ENTRY OpenDisplayIddEvtDeviceD0Entry;

// Mandatory IddCx adapter/monitor callbacks. T4 registers all of them so
// the driver is a structurally complete IddCx client and enumerates zero
// monitors until IOCTL_OPENDISPLAY_CREATE_DISPLAY (T5) calls
// IddCxMonitorArrival. Mode-description/query bodies return a single
// default mode derived from the device context's CurrentParams once a
// monitor exists; before that they report zero modes, matching "enumerates
// zero monitors" (T4's Done-when).
EVT_IDD_CX_ADAPTER_INIT_FINISHED OpenDisplayIddEvtAdapterInitFinished;
EVT_IDD_CX_ADAPTER_COMMIT_MODES OpenDisplayIddEvtAdapterCommitModes;
EVT_IDD_CX_PARSE_MONITOR_DESCRIPTION OpenDisplayIddEvtParseMonitorDescription;
EVT_IDD_CX_MONITOR_GET_DEFAULT_DESCRIPTION_MODES OpenDisplayIddEvtMonitorGetDefaultModes;
EVT_IDD_CX_MONITOR_QUERY_TARGET_MODES OpenDisplayIddEvtMonitorQueryModes;
EVT_IDD_CX_MONITOR_ASSIGN_SWAPCHAIN OpenDisplayIddEvtMonitorAssignSwapChain;
EVT_IDD_CX_MONITOR_UNASSIGN_SWAPCHAIN OpenDisplayIddEvtMonitorUnassignSwapChain;

// Implemented in Display.cpp (T5/T6/T7 add one case each): dispatches
// IOCTL_OPENDISPLAY_{CREATE,RESIZE,DESTROY}_DISPLAY.
EVT_WDF_IO_QUEUE_IO_DEVICE_CONTROL OpenDisplayIddEvtIoDeviceControl;
