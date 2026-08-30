//! The versioned JSON Lines protocol spoken between the Rust host and plugin
//! workers.
//!
//! # Purpose
//!
//! This crate is deliberately isolated from the rest of the host because it is
//! the compatibility boundary with independently developed plugins (ARC-004,
//! NFR-032, NFR-033). Nothing here knows about processes, Tokio, or CSV: it
//! turns bytes into typed messages and back.
//!
//! # Framing
//!
//! One UTF-8 JSON object per line, terminated by `\n` (REQ-080, REQ-160,
//! REQ-161). A worker's `stdout` carries protocol messages only; diagnostics
//! belong on `stderr` (REQ-065, REQ-191).
//!
//! # Strictness
//!
//! Decoding is strict by design. A syntactically valid message that does not
//! match the contract is a protocol failure, not a best-effort interpretation
//! (REQ-087). Protocol failures are reported separately from schema failures:
//! the two live at different validation boundaries (SRS 8.6).

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod codec;
pub mod error;
pub mod request;
pub mod response;

pub use codec::{decode_response, encode_request, encode_response, record_from_pairs};
pub use error::ProtocolError;
pub use request::{Request, RequestOp};
pub use response::{PluginError, Response};

/// The protocol version this crate implements.
///
/// It is deliberately the same token as the manifest `api_version`: v1 has one
/// compatibility axis, and OQ-013 (an independent schema-language version) is
/// unresolved, so introducing a second version number now would invent a
/// contract the SRS does not require.
pub const PROTOCOL_VERSION: &str = crawl_domain::SUPPORTED_API_VERSION;
