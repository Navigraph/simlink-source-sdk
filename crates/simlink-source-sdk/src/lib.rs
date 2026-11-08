//! Connects a flight simulator data source to Navigraph Simlink.
//!
//! A [`Source`] finds Simlink, joins with its options and declared channels, and keeps
//! reconnecting until dropped. Every exported type and method is a UniFFI item, which is what
//! bindings for other languages are generated from.

uniffi::setup_scaffolding!();

mod discovery;
mod logging;
mod socket_io;
mod source;
mod worker;

/// Protocol types and typed channel methods, generated from Simlink's source manifest.
pub mod protocol {
    simlink_source_sdk_macros::protocol!("../../protocol/source-schema.json");
}

pub use logging::*;
pub use protocol::*;
pub use source::*;
pub(crate) use worker::EventInfo;
