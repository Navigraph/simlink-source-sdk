//! Finds Simlink instances: loopback ports first, then mDNS for instances on other machines.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

use mdns_sd::{ServiceDaemon, ServiceEvent};

use crate::protocol::{DEFAULT_PORT, MDNS_SERVICE_TYPE, PORT_PROBE_LIMIT, PROTOCOL_VERSION};

const BROWSE_WINDOW: Duration = Duration::from_millis(1500);

/// The ports Simlink may bind on this machine, in the order it tries them.
pub fn loopback_candidates() -> Vec<SocketAddr> {
    (0..PORT_PROBE_LIMIT)
        .map(|offset| SocketAddr::new(Ipv4Addr::LOCALHOST.into(), DEFAULT_PORT + offset))
        .collect()
}

/// Simlink instances advertised over mDNS with a matching protocol major.
pub fn browse_mdns() -> Vec<SocketAddr> {
    let daemon = match ServiceDaemon::new() {
        Ok(daemon) => daemon,
        Err(error) => {
            log::debug!("mDNS unavailable: {error}");
            return Vec::new();
        }
    };

    let mut found = Vec::new();
    // mdns-sd expects the fully qualified name with its trailing dot.
    let service_type = format!("{MDNS_SERVICE_TYPE}.");

    if let Ok(events) = daemon.browse(&service_type) {
        let deadline = Instant::now() + BROWSE_WINDOW;

        while let Ok(event) = events.recv_deadline(deadline) {
            let ServiceEvent::ServiceResolved(service) = event else {
                continue;
            };
            let protocol = service.txt_properties.get_property_val_str("protocol");

            if !serves_protocol(protocol) {
                log::debug!("Skipping {}: protocol {protocol:?}", service.fullname);
                continue;
            }

            // Prefer loopback when Simlink is on this machine, then IPv4, which every host routes.
            let mut addresses: Vec<IpAddr> = service
                .addresses
                .iter()
                .map(|address| address.to_ip_addr())
                .collect();
            addresses.sort_by_key(|address| (!address.is_loopback(), address.is_ipv6()));

            found.extend(
                addresses
                    .into_iter()
                    .map(|address| SocketAddr::new(address, service.port)),
            );
        }
    }

    let _ = daemon.shutdown();

    found
}

/// Simlink lists every protocol major it serves, comma-separated, while it migrates between them.
fn serves_protocol(majors: Option<&str>) -> bool {
    majors.is_some_and(|majors| {
        majors
            .split(',')
            .any(|major| major.trim() == PROTOCOL_VERSION.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_this_protocol_among_the_advertised_majors() {
        let ours = PROTOCOL_VERSION.to_string();
        let other = (PROTOCOL_VERSION + 1).to_string();

        assert!(serves_protocol(Some(&ours)));
        assert!(serves_protocol(Some(&format!("{other}, {ours}"))));
        assert!(!serves_protocol(Some(&other)));
        assert!(!serves_protocol(None));
    }
}
