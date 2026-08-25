//! USB device discovery via usbmuxd, per `PROTOCOL.md` §2.2.
//!
//! The actual usbmuxd client (the `idevice` crate) is an OS/hardware-facing
//! call that can't run meaningfully without a real cable and device, so it
//! is isolated behind the [`UsbAttachSource`] trait. Everything below that
//! seam — folding attach/detach events into a current device set — is
//! plain, unit-testable Rust.

use std::collections::BTreeSet;

/// One attach/detach event as reported by the usbmuxd client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UsbEvent {
    Attached { device_id: String },
    Detached { device_id: String },
}

/// Abstraction over the usbmuxd attach/detach event stream itself, so
/// [`discover`]'s fold logic is testable without a live cable/device. The
/// real implementation wraps the `idevice` crate's usbmuxd client.
pub trait UsbAttachSource {
    fn poll_events(&mut self) -> Vec<UsbEvent>;
}

/// A USB-attached device, identified by its stable usbmuxd device
/// identifier (spec USB AC 1: "detect the device... without requiring
/// Apple's own driver").
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct DiscoveredDevice {
    pub id: String,
}

/// Folds `source`'s attach/detach events into the set of currently
/// USB-attached devices.
///
/// A charge-only cable (no data-capable device visible to usbmuxd) never
/// produces an `Attached` event, so it yields the same empty result as "no
/// device attached" — spec USB AC 3 requires exactly this: the two cases
/// are indistinguishable by design, not a gap to close.
pub fn discover(source: &mut dyn UsbAttachSource) -> Vec<DiscoveredDevice> {
    let mut attached: BTreeSet<String> = BTreeSet::new();
    for event in source.poll_events() {
        match event {
            UsbEvent::Attached { device_id } => {
                attached.insert(device_id);
            }
            UsbEvent::Detached { device_id } => {
                attached.remove(&device_id);
            }
        }
    }
    attached.into_iter().map(|id| DiscoveredDevice { id }).collect()
}

/// The real `UsbAttachSource`, backed by the `idevice` crate's usbmuxd
/// client. Not exercised by any automated gate on this host (no Rust
/// toolchain, no live cable/device) — see the Test Coverage Matrix's
/// manual-verification note for OS/hardware-bound code. Every non-trivial
/// branch this file needs (folding attach/detach into a device set) lives
/// above this line, behind [`UsbAttachSource`], and is unit-tested without
/// it.
///
/// Polls `UsbmuxdConnection::get_devices()` (the documented, stable part of
/// `idevice`'s API) each call and diffs against the previously seen set to
/// synthesize attach/detach events, rather than depending on an
/// unconfirmed streaming-listener API shape.
pub struct IdeviceUsbAttachSource {
    runtime: tokio::runtime::Handle,
    connection: idevice::usbmuxd::UsbmuxdConnection,
    known_udids: BTreeSet<String>,
}

impl IdeviceUsbAttachSource {
    pub fn new(runtime: tokio::runtime::Handle) -> Result<Self, idevice::IdeviceError> {
        let connection = runtime.block_on(idevice::usbmuxd::UsbmuxdConnection::default())?;
        Ok(Self {
            runtime,
            connection,
            known_udids: BTreeSet::new(),
        })
    }
}

impl UsbAttachSource for IdeviceUsbAttachSource {
    fn poll_events(&mut self) -> Vec<UsbEvent> {
        let devices = match self.runtime.block_on(self.connection.get_devices()) {
            Ok(devices) => devices,
            Err(_) => return Vec::new(),
        };
        let current: BTreeSet<String> = devices.into_iter().map(|d| d.udid).collect();

        let mut events = Vec::new();
        for udid in current.difference(&self.known_udids) {
            events.push(UsbEvent::Attached { device_id: udid.clone() });
        }
        for udid in self.known_udids.difference(&current) {
            events.push(UsbEvent::Detached { device_id: udid.clone() });
        }
        self.known_udids = current;
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeAttachSource(Vec<UsbEvent>);

    impl UsbAttachSource for FakeAttachSource {
        fn poll_events(&mut self) -> Vec<UsbEvent> {
            self.0.clone()
        }
    }

    #[test]
    fn attach_event_surfaces_a_stable_device_identifier() {
        let mut source = FakeAttachSource(vec![UsbEvent::Attached {
            device_id: "00008030-ABC".into(),
        }]);

        let devices = discover(&mut source);
        assert_eq!(devices, vec![DiscoveredDevice { id: "00008030-ABC".into() }]);
    }

    #[test]
    fn detach_event_removes_the_device_from_the_discovered_set() {
        let mut source = FakeAttachSource(vec![
            UsbEvent::Attached { device_id: "00008030-ABC".into() },
            UsbEvent::Detached { device_id: "00008030-ABC".into() },
        ]);

        let devices = discover(&mut source);
        assert_eq!(devices, Vec::<DiscoveredDevice>::new());
    }

    #[test]
    fn charge_only_cable_is_indistinguishable_from_no_device_attached() {
        // A charge-only cable never fires an Attached event at all — the
        // usbmuxd client simply has nothing to report, per PROTOCOL.md
        // §2.2 / spec USB AC 3.
        let mut charge_only = FakeAttachSource(vec![]);
        let mut nothing_plugged = FakeAttachSource(vec![]);

        assert_eq!(discover(&mut charge_only), discover(&mut nothing_plugged));
        assert_eq!(discover(&mut charge_only), Vec::<DiscoveredDevice>::new());
    }
}
