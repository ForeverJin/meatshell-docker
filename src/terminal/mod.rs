//! Transport helpers shared by the SSH automation paths.
//!
//! This tree is the headless build: there is no terminal renderer, so only the
//! byte-encoding adapter and the ZMODEM transfer implementation live here.

#[path = "impls/encoding.rs"]
mod encoding;
#[path = "impls/zmodem.rs"]
pub(crate) mod zmodem;

pub(crate) use encoding::TerminalEncoding;
