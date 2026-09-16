//! C12-C15 software acceptance for the bounded read-only COM adapter.

use lab_core::transport::{ByteTransport, RecoveryStatus, TransportIoError};
use lab_runtime::serial::{ComSettings, ComTransport, SerialDevice, SerialError, SerialParity};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Default)]
struct Script {
    writes: VecDeque<Result<usize, SerialError>>,
    reads: VecDeque<Result<Vec<u8>, SerialError>>,
    write_slices: Vec<Vec<u8>>,
}

struct ScriptedDevice(Arc<Mutex<Script>>);

impl SerialDevice for ScriptedDevice {
    fn write_once(&mut self, bytes: &[u8]) -> Result<usize, SerialError> {
        let mut script = self.0.lock().unwrap();
        script.write_slices.push(bytes.to_vec());
        script.writes.pop_front().unwrap_or(Ok(bytes.len()))
    }

    fn read_once(&mut self, maximum: usize) -> Result<Vec<u8>, SerialError> {
        let result = self
            .0
            .lock()
            .unwrap()
            .reads
            .pop_front()
            .unwrap_or_else(|| Ok(Vec::new()));
        result.map(|mut bytes| {
            bytes.truncate(maximum);
            bytes
        })
    }
}

fn settings() -> ComSettings {
    ComSettings::new_read_only(
        7,
        "COM3",
        9600,
        8,
        SerialParity::None,
        1,
        Duration::from_millis(50),
        Duration::from_millis(50),
        4,
    )
    .unwrap()
}

fn wait_for<T>(mut operation: impl FnMut() -> Option<T>) -> T {
    for _ in 0..100_000 {
        if let Some(value) = operation() {
            return value;
        }
        std::thread::yield_now();
    }
    panic!("bounded worker did not make progress")
}

#[test]
fn c12_settings_are_bounded_and_open_worker_never_blocks_owner_calls() {
    assert!(
        ComSettings::new_read_only(
            1,
            "COM0",
            9600,
            8,
            SerialParity::None,
            1,
            Duration::from_millis(50),
            Duration::from_millis(50),
            1,
        )
        .is_err()
    );
    let shared = Arc::new(Mutex::new(Script::default()));
    let mut transport = ComTransport::with_device(settings(), ScriptedDevice(shared)).unwrap();
    assert_eq!(
        wait_for(|| match transport.try_write(&[1, 2, 3]) {
            Ok(0) => None,
            result => Some(result),
        })
        .unwrap(),
        3
    );
    assert_eq!(transport.snapshot().resource_id, 7);
    assert_eq!(transport.snapshot().binding_generation, 4);
}

#[test]
fn c13_partial_io_continues_same_frame_and_timeout_is_not_retried() {
    let shared = Arc::new(Mutex::new(Script {
        writes: [Ok(2), Ok(3)].into(),
        reads: [Ok(vec![9, 8]), Ok(vec![7])].into(),
        ..Script::default()
    }));
    let mut transport =
        ComTransport::with_device(settings(), ScriptedDevice(shared.clone())).unwrap();
    assert_eq!(
        wait_for(|| match transport.try_write(&[1, 2, 3, 4, 5]) {
            Ok(0) => None,
            result => Some(result),
        })
        .unwrap(),
        5
    );
    let mut response = [0u8; 3];
    let first = wait_for(|| match transport.try_read(&mut response) {
        Ok(0) => None,
        result => Some(result),
    })
    .unwrap();
    assert_eq!(first, 2);
    let second = wait_for(|| match transport.try_read(&mut response[first..]) {
        Ok(0) => None,
        result => Some(result),
    })
    .unwrap();
    assert_eq!(second, 1);
    assert_eq!(response, [9, 8, 7]);
    assert_eq!(
        shared.lock().unwrap().write_slices,
        [vec![1, 2, 3, 4, 5], vec![3, 4, 5]]
    );

    let timed = Arc::new(Mutex::new(Script {
        writes: [Err(SerialError::Timeout)].into(),
        ..Script::default()
    }));
    let mut transport =
        ComTransport::with_device(settings(), ScriptedDevice(timed.clone())).unwrap();
    assert_eq!(
        wait_for(|| match transport.try_write(&[4, 5]) {
            Ok(0) => None,
            result => Some(result),
        })
        .unwrap(),
        2
    );
    let error = wait_for(|| {
        let mut byte = [0u8; 1];
        transport.try_read(&mut byte).err()
    });
    assert_eq!(error, TransportIoError::Other);
    assert_eq!(timed.lock().unwrap().write_slices.len(), 1);
}

#[test]
fn c14_disconnect_requires_explicit_clean_boundary_before_recovery() {
    let shared = Arc::new(Mutex::new(Script {
        writes: [Err(SerialError::Disconnected)].into(),
        ..Script::default()
    }));
    let mut transport = ComTransport::with_device(settings(), ScriptedDevice(shared)).unwrap();
    assert_eq!(
        wait_for(|| match transport.try_write(&[1]) {
            Ok(0) => None,
            result => Some(result),
        })
        .unwrap(),
        1
    );
    let error = wait_for(|| {
        let mut byte = [0u8; 1];
        transport.try_read(&mut byte).err()
    });
    assert_eq!(error, TransportIoError::Disconnected);
    assert_eq!(transport.try_recover().unwrap(), RecoveryStatus::Pending);
    transport.confirm_operator_reset_boundary().unwrap();
    assert_eq!(transport.try_recover().unwrap(), RecoveryStatus::Complete);
}

#[test]
fn c15_oversize_and_stale_session_work_cannot_cross_rebind_boundary() {
    let shared = Arc::new(Mutex::new(Script::default()));
    let mut old = ComTransport::with_device(settings(), ScriptedDevice(shared)).unwrap();
    assert_eq!(old.try_write(&[0; 39]), Err(TransportIoError::Other));
    old.retire();
    assert_eq!(old.try_write(&[1]), Err(TransportIoError::Disconnected));
    assert_eq!(old.snapshot().binding_generation, 4);

    let replacement = ComSettings::new_read_only(
        7,
        "COM4",
        9600,
        8,
        SerialParity::None,
        1,
        Duration::from_millis(50),
        Duration::from_millis(50),
        5,
    )
    .unwrap();
    let new = ComTransport::with_device(
        replacement,
        ScriptedDevice(Arc::new(Mutex::new(Script::default()))),
    )
    .unwrap();
    assert_eq!(new.snapshot().resource_id, old.snapshot().resource_id);
    assert_eq!(new.snapshot().binding_generation, 5);
}
