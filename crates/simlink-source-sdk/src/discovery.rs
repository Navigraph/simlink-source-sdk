//! Finds Simlink instances: loopback ports first, then mDNS for instances on other machines.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use mdns_sd::{ServiceDaemon, ServiceEvent};

use crate::protocol::{DEFAULT_PORT, MDNS_SERVICE_TYPE, PORT_PROBE_LIMIT, PROTOCOL_VERSION};

const BROWSE_WINDOW: Duration = Duration::from_millis(1500);
// The client tries addresses in turn, each with one timeout for its connect and handshake, so dead ones are probed out
// first. Windows takes about 2 s to refuse a closed loopback port but accepts an open one at once; an mDNS record can
// be stale or not route.
const LOOPBACK_PROBE_TIMEOUT: Duration = Duration::from_millis(250);
const REMOTE_PROBE_TIMEOUT: Duration = Duration::from_secs(1);

/// The ports Simlink may bind on this machine that accept a connection, in the order it tries them.
pub fn loopback_candidates() -> Vec<SocketAddr> {
    let ports = (0..PORT_PROBE_LIMIT)
        .map(|offset| SocketAddr::new(Ipv4Addr::LOCALHOST.into(), DEFAULT_PORT + offset))
        .collect();
    reachable(ports, LOOPBACK_PROBE_TIMEOUT)
}

/// The `addresses` that accept a connection within `timeout`, in order. Probed in parallel.
fn reachable(addresses: Vec<SocketAddr>, timeout: Duration) -> Vec<SocketAddr> {
    std::thread::scope(|scope| {
        let probes: Vec<_> = addresses
            .iter()
            .map(|address| {
                std::thread::Builder::new().spawn_scoped(scope, move || {
                    TcpStream::connect_timeout(address, timeout).is_ok()
                })
            })
            .collect();
        addresses
            .iter()
            .zip(probes)
            .filter_map(|(address, probe)| match probe {
                Ok(probe) => probe.join().unwrap_or(false).then_some(*address),
                // Left for the Socket.IO client to try.
                Err(_) => Some(*address),
            })
            .collect()
    })
}

/// Simlink instances advertised over mDNS with a matching protocol major, at addresses that accept a connection.
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

            // mdns-sd resolves a service again on each interface and update.
            for address in addresses {
                let address = SocketAddr::new(address, service.port);
                if !found.contains(&address) {
                    found.push(address);
                }
            }
        }
    }

    let _ = daemon.shutdown();

    reachable(found, REMOTE_PROBE_TIMEOUT)
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
