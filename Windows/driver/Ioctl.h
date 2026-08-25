// opendisplay-idd: the device-interface IOCTL contract between windows-core
// (Windows/core, T16 "display::create/resize/destroy") and this driver.
//
// This header is the shared contract: windows-core packs an
// OPENDISPLAY_DISPLAY_PARAMS into the input buffer of the matching IOCTL
// and issues it via DeviceIoControl against the device interface exposed
// under GUID_DEVINTERFACE_OPENDISPLAY_IDD.
#pragma once

#include <windows.h>
#include <initguid.h>

// {6E1B6F9B-6B0B-4E2A-9C36-9B7E6E6E7A11} - opendisplay-idd's private device
// interface. windows-core resolves this via SetupDiGetClassDevs /
// CM_Get_Device_Interface_List and opens it with CreateFile to issue the
// IOCTLs below. Generated once for this project; not derived from any
// Microsoft sample.
DEFINE_GUID(GUID_DEVINTERFACE_OPENDISPLAY_IDD,
    0x6e1b6f9b, 0x6b0b, 0x4e2a, 0x9c, 0x36, 0x9b, 0x7e, 0x6e, 0x6e, 0x7a, 0x11);

// Parameters shared by CREATE and RESIZE: everything windows-core derives
// from a hello handshake's DisplaySpec (design.md Data Models).
typedef struct _OPENDISPLAY_DISPLAY_PARAMS
{
    UINT32 WidthPx;
    UINT32 HeightPx;
    UINT32 RefreshHz;
    FLOAT  ScaleFactor;
} OPENDISPLAY_DISPLAY_PARAMS, *POPENDISPLAY_DISPLAY_PARAMS;

// Creates the (single, per the P1 single-device constraint) virtual
// monitor at the given resolution/refresh/scale.
// Input: OPENDISPLAY_DISPLAY_PARAMS. Output: none.
#define IOCTL_OPENDISPLAY_CREATE_DISPLAY \
    CTL_CODE(FILE_DEVICE_UNKNOWN, 0x800, METHOD_BUFFERED, FILE_WRITE_DATA)

// Rebuilds the existing virtual monitor at a new resolution (e.g. on
// device rotation), without a full destroy/recreate visible to Windows.
// Input: OPENDISPLAY_DISPLAY_PARAMS. Output: none.
#define IOCTL_OPENDISPLAY_RESIZE_DISPLAY \
    CTL_CODE(FILE_DEVICE_UNKNOWN, 0x801, METHOD_BUFFERED, FILE_WRITE_DATA)

// Removes the virtual monitor. Input: none. Output: none.
#define IOCTL_OPENDISPLAY_DESTROY_DISPLAY \
    CTL_CODE(FILE_DEVICE_UNKNOWN, 0x802, METHOD_BUFFERED, FILE_WRITE_DATA)
