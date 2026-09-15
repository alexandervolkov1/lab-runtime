//! Headless service startup prepares a safe owner before exposing the listener.
//!
//! Entropy failure, malformed CLI or failed safe profile prevents readiness.
//! The network reactor is a separate adapter added after this host foundation.

use crate::host::{HostCore, ShutdownStatus, SystemClock};
use lab_core::Error as DomainError;
use std::{
    error::Error,
    io,
    net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener},
};

/// Strict virtual-only service options; default binary execution remains finite.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ServiceOptions {
    port: u16,
}
impl ServiceOptions {
    /// Accept only the fixed M6 profile and loopback port argument.
    pub fn parse(args: &[&str]) -> Result<Self, String> {
        if args.len() != 5
            || args[0] != "--serve"
            || args[1] != "--profile"
            || args[2] != "virtual-demo"
            || args[3] != "--port"
        {
            return Err("expected --serve --profile virtual-demo --port <0..65535>".into());
        }
        let port = args[4]
            .parse::<u16>()
            .map_err(|_| "port must be an integer in 0..65535".to_string())?;
        Ok(Self { port })
    }

    /// Requested loopback TCP port; zero delegates selection to the OS.
    pub const fn port(self) -> u16 {
        self.port
    }
}

/// Safe owner and bound listener; neither socket nor serialization enters Core.
pub struct ServiceHost {
    host: HostCore,
    clock: SystemClock,
    listener: TcpListener,
    bound: SocketAddr,
    boot_id: String,
    stopping_since: Option<std::time::Instant>,
    safe_since: Option<std::time::Instant>,
    terminal: Option<ShutdownStatus>,
}
impl ServiceHost {
    /// Validate identity/profile, then bind only IPv4 loopback in that order.
    pub fn startup(options: ServiceOptions) -> Result<Self, Box<dyn Error>> {
        let mut bytes = [0u8; 16];
        getrandom::fill(&mut bytes)
            .map_err(|error| io::Error::other(format!("OS boot entropy unavailable: {error}")))?;
        let mut boot_id = String::with_capacity(32);
        for byte in bytes {
            use std::fmt::Write;
            write!(&mut boot_id, "{byte:02x}")?;
        }
        let mut host = HostCore::virtual_demo()?;
        host.set_boot_id(&boot_id);
        if !host.shutdown_status().safe_confirmed {
            return Err(io::Error::other("startup safe evidence unavailable").into());
        }
        let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, options.port()))?;
        listener.set_nonblocking(true)?;
        let bound = listener.local_addr()?;
        Ok(Self {
            host,
            clock: SystemClock::new(),
            listener,
            bound,
            boot_id,
            stopping_since: None,
            safe_since: None,
            terminal: None,
        })
    }

    /// Raise the producer stop barrier before subsequent network or worker work.
    pub fn request_shutdown(&mut self) -> Result<(), DomainError> {
        if self.stopping_since.is_some() {
            return Ok(());
        }
        self.stopping_since = Some(std::time::Instant::now());
        let clock = self.clock;
        self.host.begin_shutdown(&clock)
    }

    /// Progress trusted safe work once; never sleep or join on the owner lane.
    /// The caller keeps sweeping clients/requests between these bounded turns.
    pub fn shutdown_step(&mut self) -> Result<Option<ShutdownStatus>, DomainError> {
        if let Some(terminal) = self.terminal {
            return Ok(Some(terminal));
        }
        let Some(started) = self.stopping_since else {
            return Ok(None);
        };
        let clock = self.clock;
        self.host.service(&clock)?;
        let status = self.host.shutdown_status();
        let now = std::time::Instant::now();
        if status.safe_confirmed {
            self.safe_since.get_or_insert(now);
            if status.unfinished_workers == 0
                || self.safe_since.is_some_and(|safe_at| {
                    now.duration_since(safe_at) >= std::time::Duration::from_millis(200)
                })
            {
                self.terminal = Some(status);
            }
        } else if now.duration_since(started) >= std::time::Duration::from_secs(2) {
            self.terminal = Some(status);
        }
        Ok(self.terminal)
    }

    /// OS-selected loopback endpoint; no wildcard or external interface is bound.
    pub const fn bound_address(&self) -> SocketAddr {
        self.bound
    }
    /// Fresh 128-bit process identity; old scopes/cursors cannot attach after restart.
    pub fn boot_id(&self) -> &str {
        &self.boot_id
    }
    /// One bounded JSON readiness line for a process harness; caller prints it once.
    pub fn ready_line(&self) -> String {
        serde_json::json!({"boot_id":self.boot_id,"port":self.bound.port(),"state":"ready"})
            .to_string()
    }
    /// Borrow the committed owner only on the owning service thread.
    pub fn owner(&self) -> &HostCore {
        &self.host
    }
    /// Borrow the owner mutably only from the serialized service loop.
    pub fn owner_mut(&mut self) -> &mut HostCore {
        &mut self.host
    }
    /// Return the one monotonic process clock used by the owner.
    pub const fn clock(&self) -> &SystemClock {
        &self.clock
    }
    /// Copy the same Instant origin to drive an owner mutably in one thread.
    pub const fn clock_copy(&self) -> SystemClock {
        self.clock
    }
    /// Borrow the nonblocking listener only for the separate network reactor.
    pub const fn listener(&self) -> &TcpListener {
        &self.listener
    }
}
