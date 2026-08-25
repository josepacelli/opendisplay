//! WiFi/LAN device discovery via Bonjour (`_opensidecar._tcp`), per
//! `PROTOCOL.md` §2.1.
//!
//! The actual mDNS/DNS-SD browse (`mdns-sd`) is an OS/network-facing call
//! that can't run meaningfully off a live network, so it is isolated behind
//! the [`WifiBrowseSource`] trait. Everything below that seam — TXT-key
//! parsing and `pv` defaulting — is plain, unit-testable Rust.

use std::net::SocketAddr;

/// One resolved service instance as reported by the browse source, before
/// TXT-key parsing/defaulting is applied. Mirrors the shape `mdns-sd`'s
/// `ServiceInfo` exposes (instance name, resolved address, raw TXT
/// key/value pairs) without depending on that crate's types directly, so
/// tests can construct one without a live browse.
#[derive(Debug, Clone, PartialEq)]
pub struct ServiceRecord {
    /// The Bonjour instance name — display-only, per `PROTOCOL.md` §2.1
    /// ("MUST NOT be used as a device identity").
    pub name: String,
    pub address: SocketAddr,
    /// Raw TXT record key/value pairs, exactly as advertised. Absent TXT
    /// keys (or an absent TXT record entirely, pre-`pv` 2) are represented
    /// by this being empty or missing the key — never assumed present.
    pub txt: Vec<(String, String)>,
}

/// Abstraction over the mDNS/DNS-SD browse for `_opensidecar._tcp` itself,
/// so [`discover`]'s parsing/defaulting logic is testable without a live
/// network. The real implementation wraps `mdns-sd`.
pub trait WifiBrowseSource {
    fn discovered(&mut self) -> Vec<ServiceRecord>;
}

/// A WiFi-reachable device, as surfaced to `transport::dial` (T12) and
/// (eventually) the tray's device picker.
#[derive(Debug, Clone, PartialEq)]
pub struct DiscoveredDevice {
    /// The Bonjour TXT `id` key — stable per-install identity. `None` when
    /// the TXT record or key is absent (pre-`pv` 2 receivers advertise
    /// neither), per `PROTOCOL.md` §2.1: "Senders MUST tolerate an absent
    /// TXT record and absent keys."
    pub id: Option<String>,
    pub name: String,
    pub address: SocketAddr,
    /// The receiver's protocol version. Defaults to
    /// `protocol::ASSUMED_WHEN_ABSENT` when the `pv` TXT key is absent or
    /// fails to parse as a decimal integer, per `COMPATIBILITY.md` §2
    /// ("Absent `pv` = protocol `1`").
    pub pv: u32,
}

fn txt_value<'a>(txt: &'a [(String, String)], key: &str) -> Option<&'a str> {
    txt.iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

/// Parses one resolved service record into a [`DiscoveredDevice`], applying
/// the `pv`-absent/malformed default per `COMPATIBILITY.md` §2.
fn parse_service_record(record: &ServiceRecord) -> DiscoveredDevice {
    let id = txt_value(&record.txt, "id").map(str::to_owned);
    let pv = txt_value(&record.txt, "pv")
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(protocol::ASSUMED_WHEN_ABSENT);

    DiscoveredDevice {
        id,
        name: record.name.clone(),
        address: record.address,
        pv,
    }
}

/// Browses `source` for `_opensidecar._tcp` instances and parses each into
/// a [`DiscoveredDevice`], per `PROTOCOL.md` §2.1 / `COMPATIBILITY.md` §2.
pub fn discover(source: &mut dyn WifiBrowseSource) -> Vec<DiscoveredDevice> {
    source
        .discovered()
        .iter()
        .map(parse_service_record)
        .collect()
}

/// The real `WifiBrowseSource`, backed by `mdns-sd`. Not exercised by any
/// automated gate on this host (no Rust toolchain, no live network) — see
/// the Test Coverage Matrix's manual-verification note for OS/network-bound
/// code. Every non-trivial branch this file needs (TXT parsing, `pv`
/// defaulting) lives above this line, behind [`WifiBrowseSource`], and is
/// unit-tested without it.
pub struct MdnsSdBrowseSource {
    _daemon: mdns_sd::ServiceDaemon,
    receiver: mdns_sd::Receiver<mdns_sd::ServiceEvent>,
}

impl MdnsSdBrowseSource {
    /// Starts browsing for `_opensidecar._tcp`, per `PROTOCOL.md` §2.1.
    pub fn new() -> Result<Self, mdns_sd::Error> {
        let daemon = mdns_sd::ServiceDaemon::new()?;
        let receiver = daemon.browse("_opensidecar._tcp.local.")?;
        Ok(Self {
            _daemon: daemon,
            receiver,
        })
    }
}

impl WifiBrowseSource for MdnsSdBrowseSource {
    fn discovered(&mut self) -> Vec<ServiceRecord> {
        let mut records = Vec::new();
        while let Ok(event) = self.receiver.try_recv() {
            if let mdns_sd::ServiceEvent::ServiceResolved(info) = event {
                let txt: Vec<(String, String)> = info
                    .get_properties()
                    .iter()
                    .map(|p| (p.key().to_string(), p.val_str().to_string()))
                    .collect();
                if let Some(address) = info.get_addresses().iter().next() {
                    records.push(ServiceRecord {
                        name: info.get_fullname().to_string(),
                        address: SocketAddr::new(*address, info.get_port()),
                        txt,
                    });
                }
            }
        }
        records
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr() -> SocketAddr {
        "192.168.1.42:9000".parse().unwrap()
    }

    struct FakeBrowseSource(Vec<ServiceRecord>);

    impl WifiBrowseSource for FakeBrowseSource {
        fn discovered(&mut self) -> Vec<ServiceRecord> {
            self.0.clone()
        }
    }

    #[test]
    fn txt_present_surfaces_id_name_address_and_parsed_pv() {
        let mut source = FakeBrowseSource(vec![ServiceRecord {
            name: "Jose's iPad".into(),
            address: addr(),
            txt: vec![
                ("id".into(), "00008030-ABC".into()),
                ("pv".into(), "3".into()),
            ],
        }]);

        let devices = discover(&mut source);
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].id.as_deref(), Some("00008030-ABC"));
        assert_eq!(devices[0].name, "Jose's iPad");
        assert_eq!(devices[0].address, addr());
        assert_eq!(devices[0].pv, 3);
    }

    #[test]
    fn txt_absent_defaults_pv_to_assumed_when_absent_and_id_to_none() {
        let mut source = FakeBrowseSource(vec![ServiceRecord {
            name: "Old iPhone".into(),
            address: addr(),
            txt: vec![], // pre-pv-2 receiver: no TXT record at all
        }]);

        let devices = discover(&mut source);
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].id, None);
        assert_eq!(devices[0].pv, protocol::ASSUMED_WHEN_ABSENT);
    }

    #[test]
    fn malformed_pv_value_defaults_to_assumed_when_absent() {
        let mut source = FakeBrowseSource(vec![ServiceRecord {
            name: "Weird Device".into(),
            address: addr(),
            txt: vec![("pv".into(), "not-a-number".into())],
        }]);

        let devices = discover(&mut source);
        assert_eq!(devices[0].pv, protocol::ASSUMED_WHEN_ABSENT);
    }

    #[test]
    fn pv_key_present_but_id_key_absent_yields_none_id() {
        let mut source = FakeBrowseSource(vec![ServiceRecord {
            name: "Partial TXT".into(),
            address: addr(),
            txt: vec![("pv".into(), "2".into())],
        }]);

        let devices = discover(&mut source);
        assert_eq!(devices[0].id, None);
        assert_eq!(devices[0].pv, 2);
    }
}
