// opendisplay-idd: IOCTL dispatch plus the CREATE/RESIZE/DESTROY display
// handlers. This file is created empty-bodied by T4 (scaffolding) and
// filled in one IOCTL at a time by T5 (CREATE), T6 (RESIZE), T7 (DESTROY).
// See Driver.h for the structural-reference and verification-status note.

#include "Display.h"

VOID
OpenDisplayIddEvtIoDeviceControl(
    _In_ WDFQUEUE Queue,
    _In_ WDFREQUEST Request,
    _In_ size_t OutputBufferLength,
    _In_ size_t InputBufferLength,
    _In_ ULONG IoControlCode
)
{
    UNREFERENCED_PARAMETER(OutputBufferLength);

    WDFDEVICE device = WdfIoQueueGetDevice(Queue);
    NTSTATUS status;

    switch (IoControlCode)
    {
    case IOCTL_OPENDISPLAY_CREATE_DISPLAY:
    case IOCTL_OPENDISPLAY_RESIZE_DISPLAY:
    {
        if (InputBufferLength < sizeof(OPENDISPLAY_DISPLAY_PARAMS))
        {
            status = STATUS_BUFFER_TOO_SMALL;
            break;
        }

        POPENDISPLAY_DISPLAY_PARAMS params;
        status = WdfRequestRetrieveInputBuffer(
            Request,
            sizeof(OPENDISPLAY_DISPLAY_PARAMS),
            (PVOID*)&params,
            NULL);
        if (!NT_SUCCESS(status))
        {
            break;
        }

        status = (IoControlCode == IOCTL_OPENDISPLAY_CREATE_DISPLAY)
            ? OpenDisplayIddCreateDisplay(device, params)
            : OpenDisplayIddResizeDisplay(device, params);
        break;
    }

    case IOCTL_OPENDISPLAY_DESTROY_DISPLAY:
        status = OpenDisplayIddDestroyDisplay(device);
        break;

    default:
        status = STATUS_INVALID_DEVICE_REQUEST;
        break;
    }

    WdfRequestComplete(Request, status);
}

NTSTATUS
OpenDisplayIddCreateDisplay(
    _In_ WDFDEVICE Device,
    _In_ POPENDISPLAY_DISPLAY_PARAMS Params
)
{
    PDEVICE_CONTEXT deviceContext = DeviceGetContext(Device);

    if (!deviceContext->AdapterInitialized)
    {
        // The IddCx adapter hasn't finished EvtDeviceD0Entry /
        // IddCxAdapterInitAsync yet; windows-core must not race ahead of
        // driver load, per the design's driver-loads-first mitigation
        // (WSEND-14).
        return STATUS_DEVICE_NOT_READY;
    }

    if (deviceContext->MonitorCreated)
    {
        // P1's single-device constraint (spec.md Edge Cases) means there
        // is never a second monitor to create; a rotation/resolution
        // change goes through IOCTL_OPENDISPLAY_RESIZE_DISPLAY (T6)
        // instead of a second CREATE.
        return STATUS_DEVICE_ALREADY_ATTACHED;
    }

    IDDCX_MONITOR_INFO monitorInfo = {};
    monitorInfo.Size = sizeof(monitorInfo);
    monitorInfo.MonitorType = IDDCX_MONITOR_TYPE_ID_MONITOR_INTERFACE;
    monitorInfo.ConnectorIndex = 0;
    monitorInfo.MonitorContainerId = GUID_NULL;
    // No physical EDID: modes are supplied programmatically from Params
    // (via EvtIddCxMonitorGetDefaultDescriptionModes / QueryTargetModes
    // in Device.cpp) rather than parsed from a monitor descriptor blob.
    monitorInfo.MonitorDescription.Size = sizeof(IDDCX_MONITOR_DESCRIPTION);
    monitorInfo.MonitorDescription.Type = IDDCX_MONITOR_DESCRIPTION_TYPE_EDID;
    monitorInfo.MonitorDescription.DataSize = 0;
    monitorInfo.MonitorDescription.pData = NULL;

    IDARG_IN_MONITORCREATE monitorCreateIn = {};
    monitorCreateIn.ObjectAttributes = WDF_NO_OBJECT_ATTRIBUTES;
    monitorCreateIn.pMonitorInfo = &monitorInfo;

    IDARG_OUT_MONITORCREATE monitorCreateOut = {};
    NTSTATUS status = IddCxMonitorCreate(deviceContext->AdapterObject, &monitorCreateIn, &monitorCreateOut);
    if (!NT_SUCCESS(status))
    {
        return status;
    }

    // Record the requested resolution/refresh/scale before arrival: the
    // GetDefaultDescriptionModes / QueryTargetModes callbacks (Device.cpp)
    // read CurrentParams to advertise this exact mode, so it must be set
    // before Windows can ask for it.
    deviceContext->MonitorObject = monitorCreateOut.MonitorObject;
    deviceContext->CurrentParams = *Params;
    deviceContext->MonitorCreated = TRUE;

    IDARG_OUT_MONITORARRIVAL monitorArrivalOut = {};
    status = IddCxMonitorArrival(deviceContext->MonitorObject, &monitorArrivalOut);
    if (!NT_SUCCESS(status))
    {
        // Roll back so a retried CREATE isn't blocked by a monitor that
        // never actually became visible to Windows.
        deviceContext->MonitorCreated = FALSE;
    }

    return status;
}

NTSTATUS
OpenDisplayIddResizeDisplay(
    _In_ WDFDEVICE Device,
    _In_ POPENDISPLAY_DISPLAY_PARAMS Params
)
{
    PDEVICE_CONTEXT deviceContext = DeviceGetContext(Device);

    if (!deviceContext->MonitorCreated)
    {
        // Nothing to resize yet; the caller should CREATE first
        // (IOCTL_OPENDISPLAY_CREATE_DISPLAY, T5).
        return STATUS_DEVICE_NOT_READY;
    }

    // Update the mode windows-core wants before re-triggering mode
    // enumeration: EvtIddCxMonitorGetDefaultDescriptionModes and
    // EvtIddCxMonitorQueryTargetModes (Device.cpp) both read
    // CurrentParams, so this is what makes the *next* mode query report
    // the new resolution.
    deviceContext->CurrentParams = *Params;

    // Re-arrival on the same, still-live MonitorObject asks Windows to
    // re-query target modes for this monitor (WSEND-04: rebuild at the
    // new native resolution on rotation) without a
    // IddCxMonitorDeparture/IddCxMonitorCreate round trip, so the monitor
    // keeps its identity in the display arrangement instead of
    // disappearing and reappearing. This has not been exercised against
    // a real WDK/IddCx runtime (no toolchain on this host) — verify the
    // no-flicker behavior on real hardware per T6's manual verification.
    IDARG_OUT_MONITORARRIVAL monitorArrivalOut = {};
    return IddCxMonitorArrival(deviceContext->MonitorObject, &monitorArrivalOut);
}

NTSTATUS
OpenDisplayIddDestroyDisplay(
    _In_ WDFDEVICE Device
)
{
    PDEVICE_CONTEXT deviceContext = DeviceGetContext(Device);

    if (!deviceContext->MonitorCreated)
    {
        // Already torn down (or never created): destroy is idempotent so
        // a retried DESTROY from windows-core's teardown path never
        // errors on a monitor that is already gone.
        return STATUS_SUCCESS;
    }

    NTSTATUS status = IddCxMonitorDeparture(deviceContext->MonitorObject);
    if (!NT_SUCCESS(status))
    {
        return status;
    }

    // Leave Device Manager / Display Settings in their pre-create state
    // (WSEND-18 — driver-side half): clear the monitor handle and params
    // so a subsequent CREATE starts clean rather than seeing stale state.
    deviceContext->MonitorObject = NULL;
    deviceContext->MonitorCreated = FALSE;
    RtlZeroMemory(&deviceContext->CurrentParams, sizeof(deviceContext->CurrentParams));

    return STATUS_SUCCESS;
}
