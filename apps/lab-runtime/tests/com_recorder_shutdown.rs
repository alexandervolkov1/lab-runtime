//! C19 software acceptance for finite transport cleanup under the Host barrier.

use lab_core::transport::{
    ByteTransport, RecoveryStatus, ResourceId, TransportIoError, TransportShutdown,
};
use lab_runtime::{
    host::{Clock, HostCore},
    recorder::SqliteStore,
    service::{ServiceHost, ServiceOptions},
};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

struct ShutdownTransport(bool);
impl ByteTransport for ShutdownTransport {
    fn try_write(&mut self, _: &[u8]) -> Result<usize, TransportIoError> {
        Ok(0)
    }
    fn try_read(&mut self, _: &mut [u8]) -> Result<usize, TransportIoError> {
        Ok(0)
    }
    fn try_recover(&mut self) -> Result<RecoveryStatus, TransportIoError> {
        Ok(RecoveryStatus::Pending)
    }
    fn try_shutdown(&mut self) -> TransportShutdown {
        if self.0 {
            TransportShutdown::Complete
        } else {
            TransportShutdown::Pending
        }
    }
}

#[derive(Default)]
struct TestClock(Duration);
impl Clock for TestClock {
    fn now(&self) -> Duration {
        self.0
    }
}

#[test]
fn c19_clean_and_stalled_transport_shutdown_are_finite_and_distinct() {
    for (completes, expected_unfinished) in [(true, 0), (false, 1)] {
        let mut host = HostCore::virtual_demo().unwrap();
        host.register_transport(ResourceId::new(77), Box::new(ShutdownTransport(completes)))
            .unwrap();
        let mut clock = TestClock::default();
        host.begin_shutdown(&clock).unwrap();
        for milliseconds in [0, 10, 20] {
            clock.0 = Duration::from_millis(milliseconds);
            host.service(&clock).unwrap();
        }
        let status = host.shutdown_status();
        assert_eq!(status.unfinished_transports, expected_unfinished);
        assert_eq!(status.transports_closed, completes);
        assert_eq!(status.exit_success, completes);
    }
}

fn temporary_database() -> PathBuf {
    let mut entropy = [0u8; 12];
    getrandom::fill(&mut entropy).unwrap();
    let suffix: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
    std::env::temp_dir().join(format!("lab-runtime-m8-com-recorder-{suffix}.sqlite"))
}

#[test]
fn c19_com_and_recorder_close_independently_and_database_reopens() {
    let path = temporary_database();
    let text = path.to_string_lossy().into_owned();
    let options = ServiceOptions::parse(&[
        "--serve",
        "--profile",
        "virtual-demo",
        "--port",
        "0",
        "--record-db",
        &text,
        "--record-policy",
        "best-effort",
    ])
    .unwrap();
    let mut host = HostCore::virtual_demo().unwrap();
    host.register_transport(ResourceId::new(88), Box::new(ShutdownTransport(true)))
        .unwrap();
    let mut service = ServiceHost::startup_from_trusted_host(options, host).unwrap();
    service.request_shutdown().unwrap();
    let deadline = Instant::now() + Duration::from_secs(4);
    let terminal = loop {
        if let Some(status) = service.shutdown_step().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    };
    assert!(terminal.transports_closed);
    assert!(terminal.recorder_flushed);
    assert!(terminal.exit_success);
    drop(service);
    let store = SqliteStore::open(&path).unwrap();
    store.close().unwrap();
    std::fs::remove_file(path).unwrap();
}
