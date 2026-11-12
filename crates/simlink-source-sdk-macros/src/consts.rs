//! The constants the SDK connects with, and each event's rate limit.

use proc_macro2::TokenStream;
use quote::quote;

use crate::manifest::{Connection, Manifest};

pub fn generate(manifest: &Manifest) -> TokenStream {
    let Connection {
        protocol_version,
        default_port,
        port_probe_limit,
        max_payload_bytes,
        namespace,
        mdns_service_type,
        error_channel,
        socket_io_path,
    } = &manifest.connection;

    let events = manifest.events.iter().map(|event| {
        let name = &event.channel.name;
        let rate = match event.max_rate_hz {
            Some(rate) => quote!(Some(#rate)),
            None => quote!(None),
        };
        quote!(crate::EventInfo { name: #name, max_rate_hz: #rate })
    });

    quote! {
        /// Wire protocol major. Simlink serves `/v{PROTOCOL_VERSION}/source` and advertises it in mDNS TXT.
        pub const PROTOCOL_VERSION: u32 = #protocol_version;
        pub(crate) const NAMESPACE: &str = #namespace;
        pub(crate) const DEFAULT_PORT: u16 = #default_port;
        pub(crate) const PORT_PROBE_LIMIT: u16 = #port_probe_limit;
        pub(crate) const MDNS_SERVICE_TYPE: &str = #mdns_service_type;
        pub(crate) const ERROR_CHANNEL: &str = #error_channel;
        pub(crate) const SOCKET_IO_PATH: &str = #socket_io_path;
        /// Simlink drops the connection on a larger message.
        pub(crate) const MAX_PAYLOAD_BYTES: usize = #max_payload_bytes;
        pub(crate) const EVENTS: &[crate::EventInfo] = &[#(#events),*];
    }
}
