// opendisplay-idd: DriverEntry and the WDF driver object.
//
// See Driver.h for the structural-reference and verification-status note.

#include "Driver.h"
#include "Device.h"

NTSTATUS
DriverEntry(
    _In_ PDRIVER_OBJECT DriverObject,
    _In_ PUNICODE_STRING RegistryPath
)
{
    WDF_DRIVER_CONFIG config;
    WDF_OBJECT_ATTRIBUTES attributes;

    WDF_OBJECT_ATTRIBUTES_INIT(&attributes);
    attributes.EvtCleanupCallback = OpenDisplayIddEvtDriverContextCleanup;

    WDF_DRIVER_CONFIG_INIT(&config, OpenDisplayIddEvtDeviceAdd);
    config.DriverPoolTag = 'diDO'; // "ODid" - OpenDisplay Indirect Display

    return WdfDriverCreate(
        DriverObject,
        RegistryPath,
        &attributes,
        &config,
        WDF_NO_HANDLE);
}

VOID
OpenDisplayIddEvtDriverContextCleanup(
    _In_ WDFOBJECT DriverObject
)
{
    UNREFERENCED_PARAMETER(DriverObject);
}

NTSTATUS
OpenDisplayIddEvtDeviceAdd(
    _In_ WDFDRIVER Driver,
    _Inout_ PWDFDEVICE_INIT DeviceInit
)
{
    UNREFERENCED_PARAMETER(Driver);
    return OpenDisplayIddDeviceCreate(DeviceInit);
}
