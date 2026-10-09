//! Validated connection configuration; credentials never enter Application messages.

use std::{io, net::SocketAddr, path::PathBuf};
use tungstenite::{
    client::IntoClientRequest,
    http::{HeaderValue, Request},
};
use url::Url;

pub(crate) const SUBPROTOCOL: &str = "lab-runtime.application.v1";

#[derive(Clone)]
pub(crate) enum RuntimeEndpoint {
    Tcp {
        address: SocketAddr,
        allow_remote: bool,
    },
    WebSocket(WebSocketEndpoint),
}

#[derive(Clone)]
pub(crate) struct WebSocketEndpoint {
    pub(crate) url: Url,
    origin: HeaderValue,
    token: Option<HeaderValue>,
    pub(crate) ca_file: Option<PathBuf>,
}

// Intentionally no Debug implementation: HTTP errors may include reflected credentials.
impl RuntimeEndpoint {
    pub(crate) fn tcp(address: SocketAddr, allow_remote: bool) -> Result<Self, &'static str> {
        Self::validate_tcp(address, allow_remote)?;
        Ok(Self::Tcp {
            address,
            allow_remote,
        })
    }

    pub(crate) fn validate_tcp(
        address: SocketAddr,
        allow_remote: bool,
    ) -> Result<(), &'static str> {
        if !address.ip().is_loopback() {
            if !allow_remote {
                return Err(
                    "remote TCP requires --allow-remote-tcp (trusted LAN only; no TLS/authentication)",
                );
            }
            let std::net::IpAddr::V4(ip) = address.ip() else {
                return Err("remote TCP requires a numeric unicast IPv4 endpoint");
            };
            if ip.is_unspecified()
                || ip.is_broadcast()
                || ip.is_multicast()
                || ip.octets()[0] == 0
                || ip.octets()[0] >= 240
                || address.port() == 0
            {
                return Err("remote TCP requires a numeric unicast IPv4 endpoint and nonzero port");
            }
        }
        Ok(())
    }

    pub(crate) fn parse(
        address: &str,
        origin: &str,
        allow_insecure: bool,
        token: Option<String>,
        ca_file: Option<PathBuf>,
    ) -> Result<Self, &'static str> {
        if !address.starts_with("ws://") && !address.starts_with("wss://") {
            if token.is_some() || ca_file.is_some() {
                return Err("WebSocket credentials and CA configuration require a WS/WSS endpoint");
            }
            let address: SocketAddr = address.parse().map_err(|_| "invalid Runtime endpoint")?;
            return Self::tcp(address, false);
        }
        if address.len() > 1024 {
            return Err("Runtime endpoint exceeds its bound");
        }
        let url = Url::parse(address).map_err(|_| "invalid WebSocket endpoint")?;
        if !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/application/v1"
            || url.host_str().is_none()
        {
            return Err(
                "WebSocket endpoint must have /application/v1 and no credentials, query or fragment",
            );
        }
        let loopback = url
            .host_str()
            .and_then(|host| {
                host.trim_matches(['[', ']'])
                    .parse::<std::net::IpAddr>()
                    .ok()
            })
            .is_some_and(|ip| ip.is_loopback());
        if url.scheme() == "ws" && !loopback && !allow_insecure {
            return Err("remote plaintext WS requires --allow-insecure-ws; prefer WSS");
        }
        if url.scheme() == "ws" && ca_file.is_some() {
            return Err("CA configuration requires WSS");
        }
        let parsed_origin = Url::parse(origin).map_err(|_| "invalid WebSocket Origin")?;
        if !matches!(parsed_origin.scheme(), "http" | "https")
            || parsed_origin.host_str().is_none()
            || parsed_origin.origin().ascii_serialization() != origin
        {
            return Err("WebSocket Origin must be an exact HTTP/HTTPS origin");
        }
        let origin = HeaderValue::from_str(origin).map_err(|_| "invalid WebSocket Origin")?;
        let token = token
            .map(|token| {
                if token.is_empty()
                    || token.len() > 512
                    || !token.is_ascii()
                    || token.contains(['\r', '\n'])
                {
                    return Err("tunnel token must be nonempty bounded ASCII without line breaks");
                }
                let mut value =
                    HeaderValue::from_str(&token).map_err(|_| "invalid tunnel token")?;
                value.set_sensitive(true);
                Ok(value)
            })
            .transpose()?;
        Ok(Self::WebSocket(WebSocketEndpoint {
            url,
            origin,
            token,
            ca_file,
        }))
    }

    pub(crate) fn label(&self) -> String {
        match self {
            Self::Tcp { address, .. } => address.to_string(),
            Self::WebSocket(endpoint) => endpoint.url.to_string(),
        }
    }

    pub(crate) fn insecure_remote(&self) -> bool {
        match self {
            Self::Tcp { address, .. } => !address.ip().is_loopback(),
            Self::WebSocket(endpoint) => {
                endpoint.url.scheme() == "ws"
                    && !endpoint
                        .url
                        .host_str()
                        .and_then(|host| {
                            host.trim_matches(['[', ']'])
                                .parse::<std::net::IpAddr>()
                                .ok()
                        })
                        .is_some_and(|ip| ip.is_loopback())
            }
        }
    }
}

impl WebSocketEndpoint {
    pub(crate) fn request(&self) -> io::Result<Request<()>> {
        let mut request = self
            .url
            .as_str()
            .into_client_request()
            .map_err(|_| io::Error::other("invalid WebSocket request"))?;
        request.headers_mut().insert("Origin", self.origin.clone());
        request.headers_mut().insert(
            "Sec-WebSocket-Protocol",
            HeaderValue::from_static(SUBPROTOCOL),
        );
        if let Some(token) = &self.token {
            request.headers_mut().insert("X-Token", token.clone());
        }
        Ok(request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(address: &str, allow: bool) -> Result<RuntimeEndpoint, &'static str> {
        RuntimeEndpoint::parse(address, "http://127.0.0.1:3000", allow, None, None)
    }

    #[test]
    fn remote_plaintext_is_explicit_and_credentials_cannot_enter_urls() {
        assert!(parse("127.0.0.1:7420", false).is_ok());
        assert!(parse("ws://127.0.0.1:8766/application/v1", false).is_ok());
        assert!(parse("ws://192.0.2.1/application/v1", false).is_err());
        assert!(parse("ws://192.0.2.1/application/v1", true).is_ok());
        assert!(parse("wss://example.com/application/v1", false).is_ok());
        for address in [
            "wss://user:secret@example.com/application/v1",
            "wss://example.com/application/v1?token=x",
            "wss://example.com/application/v1#x",
            "ws://example.com/",
        ] {
            assert!(parse(address, true).is_err());
        }
    }

    #[test]
    fn token_is_handshake_only_and_sensitive() {
        let endpoint = RuntimeEndpoint::parse(
            "wss://example.com/application/v1",
            "http://127.0.0.1:3000",
            false,
            Some("test-token".into()),
            None,
        )
        .unwrap();
        let RuntimeEndpoint::WebSocket(endpoint) = endpoint else {
            panic!("WS endpoint")
        };
        let request = endpoint.request().unwrap();
        assert!(request.headers()["X-Token"].is_sensitive());
        assert!(!request.uri().to_string().contains("test-token"));
        assert!(
            RuntimeEndpoint::parse(
                "wss://example.com/application/v1",
                "http://127.0.0.1:3000",
                false,
                Some("injected\r\nx: bad".into()),
                None
            )
            .is_err()
        );
    }

    #[test]
    fn remote_tcp_requires_explicit_opt_in_and_rejects_non_unicast_destinations() {
        let remote = "192.168.1.50:8765".parse().unwrap();
        assert!(RuntimeEndpoint::tcp(remote, false).is_err());
        let endpoint = RuntimeEndpoint::tcp(remote, true).unwrap();
        assert_eq!(endpoint.label(), "192.168.1.50:8765");
        assert!(endpoint.insecure_remote());
        assert!(RuntimeEndpoint::tcp("127.0.0.1:8765".parse().unwrap(), false).is_ok());
        for address in [
            "0.0.0.0:8765",
            "0.1.2.3:8765",
            "255.255.255.255:8765",
            "224.0.0.1:8765",
            "240.1.2.3:8765",
            "[2001:db8::1]:8765",
            "192.168.1.50:0",
        ] {
            assert!(
                RuntimeEndpoint::tcp(address.parse().unwrap(), true).is_err(),
                "{address}"
            );
        }
    }
}
