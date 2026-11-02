//! Authentication gateway primitives (pure; no I/O).
//!
//! The single perimeter is a signed session cookie checked in front of every route (HTTP and
//! WebSocket alike). This module holds the cookie's wire format and the signing/verification math;
//! the HTTP handlers and the key/credential wiring live in the always-on sync-service. See
//! deploy/docs/design-unified-auth-gateway.md.

pub mod access_jwt;
pub mod session;
