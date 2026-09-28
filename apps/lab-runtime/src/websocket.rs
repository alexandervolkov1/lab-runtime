//! Bounded local WebSocket endpoint policy, separate from Application semantics.

use std::collections::BTreeSet;

/// Fixed WebSocket HTTP Upgrade path for Application protocol version one.
pub const APPLICATION_PATH: &str = "/application/v1";
/// Required WebSocket subprotocol for Application protocol version one.
pub const APPLICATION_SUBPROTOCOL: &str = "lab-runtime.application.v1";
/// Maximum complete HTTP Upgrade request bytes, including the final CRLF pair.
pub const HANDSHAKE_BYTES: usize = 8 * 1024;
/// Maximum HTTP Upgrade header fields after parsing.
pub const HANDSHAKE_HEADERS: usize = 32;
/// Eager Tungstenite read allocation per admitted WebSocket connection.
pub const READ_BUFFER_BYTES: usize = 4 * 1024;
/// Maximum Tungstenite-owned pending encoded output per connection.
pub const WRITE_BUFFER_BYTES: usize = 32 * 1024;
/// Maximum exact browser origins admitted in one endpoint allowlist.
pub const ALLOWED_ORIGINS: usize = 16;
/// Maximum bytes in one configured or received Origin value.
pub const ORIGIN_BYTES: usize = 256;

/// Validated optional loopback WebSocket endpoint configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebSocketOptions {
    port: u16,
    allowed_origins: Vec<String>,
}

impl WebSocketOptions {
    /// Validate a loopback listener port and exact browser Origin allowlist.
    pub fn new(port: u16, allowed_origins: Vec<String>) -> Result<Self, &'static str> {
        validate_origins(&allowed_origins)?;
        Ok(Self {
            port,
            allowed_origins,
        })
    }

    /// Requested IPv4-loopback port; zero delegates selection to the OS.
    pub const fn port(&self) -> u16 {
        self.port
    }

    /// Exact canonical scheme/host/port Origin values admitted before Upgrade.
    pub fn allowed_origins(&self) -> &[String] {
        &self.allowed_origins
    }
}

pub(crate) fn validate_origins(origins: &[String]) -> Result<(), &'static str> {
    if origins.is_empty() {
        return Err("enabled WebSocket endpoint requires allowed_origins");
    }
    if origins.len() > ALLOWED_ORIGINS {
        return Err("too many WebSocket allowed_origins");
    }
    let mut unique = BTreeSet::new();
    for origin in origins {
        validate_origin(origin)?;
        if !unique.insert(origin) {
            return Err("duplicate WebSocket allowed_origin");
        }
    }
    Ok(())
}

fn validate_origin(origin: &str) -> Result<(), &'static str> {
    if origin.is_empty() || origin.len() > ORIGIN_BYTES || !origin.is_ascii() {
        return Err("invalid WebSocket allowed_origin length");
    }
    if origin.contains('*') || origin.contains(['/', '?', '#', '@']) {
        let scheme_separator = origin.find("://");
        if origin.contains('*')
            || origin.contains(['?', '#', '@'])
            || origin[scheme_separator.map_or(0, |index| index + 3)..].contains('/')
        {
            return Err("invalid WebSocket allowed_origin syntax");
        }
    }
    let (scheme, authority) = origin
        .split_once("://")
        .ok_or("invalid WebSocket allowed_origin syntax")?;
    if !matches!(scheme, "http" | "https") {
        return Err("WebSocket allowed_origin scheme must be http or https");
    }
    let (host, port) = authority
        .rsplit_once(':')
        .ok_or("WebSocket allowed_origin requires an explicit port")?;
    if host.is_empty()
        || host.len() > 253
        || host.starts_with('.')
        || host.ends_with('.')
        || host.starts_with('-')
        || host.ends_with('-')
        || !host.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-')
        })
    {
        return Err("invalid WebSocket allowed_origin host");
    }
    let port = port
        .parse::<u16>()
        .map_err(|_| "invalid WebSocket allowed_origin port")?;
    if port == 0 {
        return Err("WebSocket allowed_origin port must be nonzero");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_origin_allowlist_rejects_ambiguous_or_wildcard_forms() {
        for accepted in ["http://127.0.0.1:3000", "https://localhost:443"] {
            assert!(WebSocketOptions::new(0, vec![accepted.to_owned()]).is_ok());
        }
        for rejected in [
            "*",
            "null",
            "http://localhost",
            "http://*.localhost:3000",
            "http://localhost:3000/path",
            "http://LOCALHOST:3000",
            "file://localhost:3000",
            "http://localhost:0",
        ] {
            assert!(WebSocketOptions::new(0, vec![rejected.to_owned()]).is_err());
        }
        assert!(
            WebSocketOptions::new(
                0,
                vec![
                    "http://localhost:3000".to_owned(),
                    "http://localhost:3000".to_owned()
                ]
            )
            .is_err()
        );
    }
}
