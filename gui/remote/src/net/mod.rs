//! The network side: datagram layouts, the session socket and its threads,
//! the readiness probe, and the TCP control channel.

pub mod control;
pub mod probe;
pub mod session;
pub mod wire;
