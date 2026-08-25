// opendisplay-idd: device/adapter setup and the IddCx callbacks that stay
// constant regardless of whether a monitor currently exists. See Driver.h
// for the structural-reference and verification-status note.

#include "Driver.h"
#include "Device.h"

NTSTATUS
OpenDisplayIddDeviceCreate(
    _Inout_ PWDFDEVICE_INIT DeviceInit
)
{
    // Register this device as an IddCx client before WdfDeviceCreate, per
    // the documented IddCx driver initialization order.
    IDD_CX_CLIENT_CONFIG iddConfig;
    IDD_CX_CLIENT_CONFIG_INIT(&iddConfig);

    iddConfig.EvtIddCxAdapterInitFinished = OpenDisplayIddEvtAdapterInitFinished;
    iddConfig.EvtIddCxAdapterCommitModes = OpenDisplayIddEvtAdapterCommitModes;
    iddConfig.EvtIddCxParseMonitorDescription = OpenDisplayIddEvtParseMonitorDescription;
    iddConfig.EvtIddCxMonitorGetDefaultDescriptionModes = OpenDisplayIddEvtMonitorGetDefaultModes;
    iddConfig.EvtIddCxMonitorQueryTargetModes = OpenDisplayIddEvtMonitorQueryModes;
    iddConfig.EvtIddCxMonitorAssignSwapChain = OpenDisplayIddEvtMonitorAssignSwapChain;
    iddConfig.EvtIddCxMonitorUnassignSwapChain = OpenDisplayIddEvtMonitorUnassignSwapChain;

    NTSTATUS status = IddCxDeviceInitConfig(DeviceInit, &iddConfig);
    if (!NT_SUCCESS(status))
    {
        return status;
    }

    WDF_PNPPOWER_EVENT_CALLBACKS pnpCallbacks;
    WDF_PNPPOWER_EVENT_CALLBACKS_INIT(&pnpCallbacks);
    pnpCallbacks.EvtDeviceD0Entry = OpenDisplayIddEvtDeviceD0Entry;
    WdfDeviceInitSetPnpPowerEventCallbacks(DeviceInit, &pnpCallbacks);

    WDF_OBJECT_ATTRIBUTES deviceAttributes;
    WDF_OBJECT_ATTRIBUTES_INIT_CONTEXT_TYPE(&deviceAttributes, DEVICE_CONTEXT);

    WDFDEVICE device;
    status = WdfDeviceCreate(&DeviceInit, &deviceAttributes, &device);
    if (!NT_SUCCESS(status))
    {
        return status;
    }

    PDEVICE_CONTEXT deviceContext = DeviceGetContext(device);
    RtlZeroMemory(deviceContext, sizeof(DEVICE_CONTEXT));

    status = IddCxDeviceInitialize(device);
    if (!NT_SUCCESS(status))
    {
        return status;
    }

    // Private device interface windows-core opens (via
    // GUID_DEVINTERFACE_OPENDISPLAY_IDD) to issue the CREATE/RESIZE/DESTROY
    // IOCTLs. Distinct from whatever interface IddCx itself registers for
    // the indirect-display device stack.
    status = WdfDeviceCreateDeviceInterface(device, &GUID_DEVINTERFACE_OPENDISPLAY_IDD, NULL);
    if (!NT_SUCCESS(status))
    {
        return status;
    }

    // A single sequential default queue dispatches all three IOCTLs to
    // Display.cpp. Sequential dispatch means CREATE/RESIZE/DESTROY are
    // never processed concurrently against the same device context.
    WDF_IO_QUEUE_CONFIG queueConfig;
    WDF_IO_QUEUE_CONFIG_INIT_DEFAULT_QUEUE(&queueConfig, WdfIoQueueDispatchSequential);
    queueConfig.EvtIoDeviceControl = OpenDisplayIddEvtIoDeviceControl;

    WDFQUEUE queue;
    status = WdfIoQueueCreate(device, &queueConfig, WDF_NO_OBJECT_ATTRIBUTES, &queue);
    return status;
}

NTSTATUS
OpenDisplayIddEvtDeviceD0Entry(
    _In_ WDFDEVICE Device,
    _In_ WDF_POWER_DEVICE_STATE PreviousState
)
{
    UNREFERENCED_PARAMETER(PreviousState);

    PDEVICE_CONTEXT deviceContext = DeviceGetContext(Device);

    IDARG_IN_ADAPTER_INIT adapterInit = {};
    adapterInit.WdfDevice = Device;
    adapterInit.EndPointDiagnostics.Size = sizeof(adapterInit.EndPointDiagnostics);
    adapterInit.EndPointDiagnostics.GammaSupport = IDDCX_FEATURE_IMPLEMENTATION_NONE;
    adapterInit.EndPointDiagnostics.TransmissionType = IDDCX_TRANSMISSION_TYPE_WIRED_OTHER;
    adapterInit.EndPointDiagnostics.pEndPointFriendlyName = L"OpenDisplay Virtual Display";
    adapterInit.EndPointDiagnostics.pEndPointManufacturerName = L"OpenDisplay";
    adapterInit.EndPointDiagnostics.pEndPointModelName = L"opendisplay-idd";

    IDARG_OUT_ADAPTER_INIT adapterInitOut = {};
    NTSTATUS status = IddCxAdapterInitAsync(&adapterInit, &adapterInitOut);
    if (NT_SUCCESS(status))
    {
        deviceContext->AdapterObject = adapterInitOut.AdapterObject;
    }

    return status;
}

NTSTATUS
OpenDisplayIddEvtAdapterInitFinished(
    _In_ IDDCX_ADAPTER AdapterObject,
    _In_ const IDARG_IN_ADAPTER_INIT_FINISHED* pInArgs
)
{
    PDEVICE_CONTEXT deviceContext = DeviceGetContext(WdfObjectContextGetObject(AdapterObject));
    deviceContext->AdapterInitialized = NT_SUCCESS(pInArgs->AdapterInitStatus);
    return STATUS_SUCCESS;
}

NTSTATUS
OpenDisplayIddEvtAdapterCommitModes(
    _In_ IDDCX_ADAPTER AdapterObject,
    _In_ const IDARG_IN_COMMITMODES* pInArgs
)
{
    UNREFERENCED_PARAMETER(AdapterObject);
    UNREFERENCED_PARAMETER(pInArgs);
    // No hardware mode-set to perform: the "display" is fully virtual and
    // DXGI Desktop Duplication (windows-core T17) reads the composed
    // output, not a physical scanout this driver programs.
    return STATUS_SUCCESS;
}

NTSTATUS
OpenDisplayIddEvtParseMonitorDescription(
    _In_ const IDARG_IN_PARSEMONITORDESCRIPTION* pInArgs,
    _Out_ IDARG_OUT_PARSEMONITORDESCRIPTION* pOutArgs
)
{
    UNREFERENCED_PARAMETER(pInArgs);
    // No EDID/vendor descriptor to parse: modes come from the DisplaySpec
    // windows-core derives from the device's `hello` payload, not from a
    // monitor descriptor blob.
    pOutArgs->MonitorModeBufferOutputCount = 0;
    return STATUS_SUCCESS;
}

NTSTATUS
OpenDisplayIddEvtMonitorGetDefaultModes(
    _In_ IDDCX_MONITOR MonitorObject,
    _In_ const IDARG_IN_GETDEFAULTDESCRIPTIONMODES* pInArgs,
    _Out_ IDARG_OUT_GETDEFAULTDESCRIPTIONMODES* pOutArgs
)
{
    UNREFERENCED_PARAMETER(pInArgs);

    PDEVICE_CONTEXT deviceContext = DeviceGetContext(WdfObjectContextGetObject(MonitorObject));
    if (!deviceContext->MonitorCreated)
    {
        pOutArgs->DefaultMonitorModeBufferOutputCount = 0;
        return STATUS_SUCCESS;
    }

    // Exactly one mode: the resolution/refresh IOCTL_OPENDISPLAY_CREATE_
    // DISPLAY (Display.cpp, T5) was called with. There is never a second
    // mode to offer because Windows never gets to choose a resolution for
    // a device it did not attach.
    pOutArgs->DefaultMonitorModeBufferOutputCount = 1;
    return STATUS_SUCCESS;
}

NTSTATUS
OpenDisplayIddEvtMonitorQueryModes(
    _In_ IDDCX_MONITOR MonitorObject,
    _In_ const IDARG_IN_QUERYTARGETMODES* pInArgs,
    _Out_ IDARG_OUT_QUERYTARGETMODES* pOutArgs
)
{
    UNREFERENCED_PARAMETER(pInArgs);

    PDEVICE_CONTEXT deviceContext = DeviceGetContext(WdfObjectContextGetObject(MonitorObject));
    pOutArgs->TargetModeBufferOutputCount = deviceContext->MonitorCreated ? 1 : 0;
    return STATUS_SUCCESS;
}

NTSTATUS
OpenDisplayIddEvtMonitorAssignSwapChain(
    _In_ IDDCX_MONITOR MonitorObject,
    _In_ const IDARG_IN_SETSWAPCHAIN* pInArgs
)
{
    UNREFERENCED_PARAMETER(MonitorObject);
    UNREFERENCED_PARAMETER(pInArgs);
    // Frame servicing for the assigned swap chain is intentionally out of
    // scope for T4-T8 (the driver-skeleton phase): no task in this phase
    // covers it. windows-core's capture stage (T17, DXGI Desktop
    // Duplication) reads the desktop composited onto this monitor's
    // output directly and does not depend on this driver consuming the
    // swap chain's individual frames.
    return STATUS_SUCCESS;
}

NTSTATUS
OpenDisplayIddEvtMonitorUnassignSwapChain(
    _In_ IDDCX_MONITOR MonitorObject
)
{
    UNREFERENCED_PARAMETER(MonitorObject);
    return STATUS_SUCCESS;
}
