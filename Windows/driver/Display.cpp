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
    UNREFERENCED_PARAMETER(Device);
    UNREFERENCED_PARAMETER(Params);
    // Implemented by T5.
    return STATUS_NOT_IMPLEMENTED;
}

NTSTATUS
OpenDisplayIddResizeDisplay(
    _In_ WDFDEVICE Device,
    _In_ POPENDISPLAY_DISPLAY_PARAMS Params
)
{
    UNREFERENCED_PARAMETER(Device);
    UNREFERENCED_PARAMETER(Params);
    // Implemented by T6.
    return STATUS_NOT_IMPLEMENTED;
}

NTSTATUS
OpenDisplayIddDestroyDisplay(
    _In_ WDFDEVICE Device
)
{
    UNREFERENCED_PARAMETER(Device);
    // Implemented by T7.
    return STATUS_NOT_IMPLEMENTED;
}
