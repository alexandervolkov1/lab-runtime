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
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
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

struct OneTurnShutdownTransport(Arc<AtomicUsize>);

impl ByteTransport for OneTurnShutdownTransport {
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
        if self.0.fetch_add(1, Ordering::AcqRel) == 0 {
            TransportShutdown::Pending
        } else {
            TransportShutdown::Complete
        }
    }
}

#[test]
fn service_does_not_freeze_a_one_turn_transport_retirement_as_terminal_failure() {
    // Regression for the M9D shutdown correction: Pending is progress state, not a
    // terminal classification. The owner must grant a later bounded turn so a COM
    // worker that closes asynchronously can prove completion.
    let attempts = Arc::new(AtomicUsize::new(0));
    let mut host = HostCore::virtual_demo().unwrap();
    host.register_transport(
        ResourceId::new(89),
        Box::new(OneTurnShutdownTransport(attempts.clone())),
    )
    .unwrap();
    let options =
        ServiceOptions::parse(&["--serve", "--profile", "virtual-demo", "--port", "0"]).unwrap();
    let mut service = ServiceHost::startup_from_trusted_host(options, host).unwrap();
    service.request_shutdown().unwrap();

    assert!(service.shutdown_step().unwrap().is_none());
    assert_eq!(attempts.load(Ordering::Acquire), 1);
    assert_eq!(service.owner().shutdown_status().unfinished_transports, 1);

    let terminal = service.shutdown_step().unwrap().unwrap();
    assert_eq!(attempts.load(Ordering::Acquire), 2);
    assert_eq!(terminal.unfinished_transports, 0);
    assert!(terminal.transports_closed);
    assert!(terminal.exit_success);
}

#[test]
fn service_retains_a_finite_deadline_for_a_transport_that_never_retires() {
    let mut host = HostCore::virtual_demo().unwrap();
    host.register_transport(ResourceId::new(90), Box::new(ShutdownTransport(false)))
        .unwrap();
    let options =
        ServiceOptions::parse(&["--serve", "--profile", "virtual-demo", "--port", "0"]).unwrap();
    let mut service = ServiceHost::startup_from_trusted_host(options, host).unwrap();
    service.request_shutdown().unwrap();

    let started = Instant::now();
    let terminal = loop {
        if let Some(status) = service.shutdown_step().unwrap() {
            break status;
        }
        assert!(started.elapsed() < Duration::from_millis(2_500));
        std::thread::sleep(Duration::from_millis(5));
    };
    assert!(started.elapsed() >= Duration::from_secs(2));
    assert_eq!(terminal.unfinished_transports, 1);
    assert!(!terminal.transports_closed);
    assert!(!terminal.exit_success);
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
