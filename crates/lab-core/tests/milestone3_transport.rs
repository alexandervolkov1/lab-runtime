//! M3 acceptance for bounded single-owner byte transport execution.

use lab_core::transport::{
    ByteTransport, ExecutorState, RecoveryStatus, ResourceExecutor, ResourceId, TransportError,
    TransportIoError, TransactionOutcome, MAX_QUEUED_TRANSACTIONS,
};
use std::{
    cell::RefCell,
    collections::VecDeque,
    rc::Rc,
    time::Duration,
};

#[derive(Default)]
struct Script {
    accepted: Vec<u8>,
    max_write: usize,
    request_lengths: VecDeque<usize>,
    responses: VecDeque<Vec<u8>>,
    readable: VecDeque<u8>,
    recoveries: VecDeque<Result<RecoveryStatus, TransportIoError>>,
    write_error_after: Option<usize>,
}

struct FakeTransport(Rc<RefCell<Script>>);

impl ByteTransport for FakeTransport {
    fn try_write(&mut self, bytes: &[u8]) -> Result<usize, TransportIoError> {
        let mut script = self.0.borrow_mut();
        if script.write_error_after == Some(script.accepted.len()) {
            script.write_error_after = None;
            return Err(TransportIoError::Disconnected);
        }
        let accepted = bytes.len().min(script.max_write);
        script.accepted.extend_from_slice(&bytes[..accepted]);
        if script.request_lengths.front().copied() == Some(script.accepted.len()) {
            script.request_lengths.pop_front();
            if let Some(response) = script.responses.pop_front() {
                script.readable.extend(response);
            }
        }
        Ok(accepted)
    }

    fn try_read(&mut self, bytes: &mut [u8]) -> Result<usize, TransportIoError> {
        let mut script = self.0.borrow_mut();
        let count = bytes.len().min(script.readable.len()).min(2);
        for slot in &mut bytes[..count] {
            *slot = script.readable.pop_front().unwrap();
        }
        Ok(count)
    }

    fn try_recover(&mut self) -> Result<RecoveryStatus, TransportIoError> {
        let mut script = self.0.borrow_mut();
        script.readable.clear();
        script
            .recoveries
            .pop_front()
            .unwrap_or(Ok(RecoveryStatus::Complete))
    }
}

fn executor(script: Rc<RefCell<Script>>) -> ResourceExecutor {
    ResourceExecutor::new(ResourceId::new(1), Box::new(FakeTransport(script)))
}

fn queue_read(executor: &mut ResourceExecutor, request: &[u8], now: u64) {
    executor
        .enqueue_read(
            request,
            3,
            Duration::from_millis(now),
            Duration::from_millis(now + 100),
            Duration::from_millis(20),
            false,
            1,
            1,
        )
        .unwrap();
}

#[test]
fn partial_io_serializes_two_instruments_without_byte_interleaving() {
    let script = Rc::new(RefCell::new(Script {
        max_write: 2,
        request_lengths: VecDeque::from([4, 8]),
        responses: VecDeque::from([vec![1, 2, 3], vec![4, 5, 6]]),
        ..Script::default()
    }));
    let mut executor = executor(script.clone());
    queue_read(&mut executor, &[10, 11, 12, 13], 0);
    queue_read(&mut executor, &[20, 21, 22, 23], 0);

    for millisecond in 0..12 {
        executor.poll(Duration::from_millis(millisecond)).unwrap();
    }

    assert_eq!(script.borrow().accepted, [10, 11, 12, 13, 20, 21, 22, 23]);
    assert_eq!(executor.snapshot().queue_len, 0);
    assert_eq!(executor.snapshot().state, ExecutorState::Idle);
    assert_eq!(
        executor.snapshot().latest.unwrap().outcome,
        TransactionOutcome::Completed
    );
}

#[test]
fn separate_resource_owners_progress_independently() {
    let stalled = Rc::new(RefCell::new(Script {
        max_write: 0,
        ..Script::default()
    }));
    let flowing = Rc::new(RefCell::new(Script {
        max_write: 8,
        request_lengths: VecDeque::from([2]),
        responses: VecDeque::from([vec![7, 8, 9]]),
        ..Script::default()
    }));
    let mut first = executor(stalled);
    let mut second = ResourceExecutor::new(
        ResourceId::new(2),
        Box::new(FakeTransport(flowing.clone())),
    );
    queue_read(&mut first, &[1, 1], 0);
    queue_read(&mut second, &[2, 2], 0);
    for millisecond in 0..3 {
        first.poll(Duration::from_millis(millisecond)).unwrap();
        second.poll(Duration::from_millis(millisecond)).unwrap();
    }
    assert!(flowing.borrow().accepted.len() == 2);
    assert_eq!(second.snapshot().state, ExecutorState::Idle);
    assert_eq!(first.snapshot().state, ExecutorState::InFlight);
}

#[test]
fn queue_capacity_and_exact_deadline_are_explicit() {
    let script = Rc::new(RefCell::new(Script {
        max_write: 0,
        ..Script::default()
    }));
    let mut executor = executor(script.clone());
    for index in 0..MAX_QUEUED_TRANSACTIONS {
        executor
            .enqueue_read(
                &[index as u8],
                1,
                Duration::ZERO,
                Duration::from_secs(1),
                Duration::from_secs(1),
                false,
                1,
                1,
            )
            .unwrap();
    }
    assert_eq!(
        executor.enqueue_read(
            &[99],
            1,
            Duration::ZERO,
            Duration::from_secs(1),
            Duration::from_secs(1),
            false,
            1,
            1,
        ),
        Err(TransportError::QueueFull)
    );

    let mut exact = executor(script.clone());
    exact
        .enqueue_read(
            &[42],
            1,
            Duration::ZERO,
            Duration::from_millis(10),
            Duration::from_secs(1),
            false,
            1,
            1,
        )
        .unwrap();
    exact.poll(Duration::from_millis(10)).unwrap();
    assert!(script.borrow().accepted.is_empty());
    assert_eq!(
        exact.snapshot().latest.unwrap().outcome,
        TransactionOutcome::QueueExpired
    );
}

#[test]
fn timeout_after_a_prefix_requires_recovery_before_next_work() {
    let script = Rc::new(RefCell::new(Script {
        max_write: 1,
        recoveries: VecDeque::from([Ok(RecoveryStatus::Pending), Ok(RecoveryStatus::Complete)]),
        ..Script::default()
    }));
    let mut executor = executor(script.clone());
    executor
        .enqueue_read(
            &[1, 2, 3],
            3,
            Duration::ZERO,
            Duration::from_secs(1),
            Duration::from_millis(2),
            false,
            1,
            1,
        )
        .unwrap();
    queue_read(&mut executor, &[9, 9], 0);

    executor.poll(Duration::ZERO).unwrap();
    executor.poll(Duration::from_millis(2)).unwrap();
    assert_eq!(executor.snapshot().state, ExecutorState::Recovering);
    let prefix = script.borrow().accepted.clone();
    executor.poll(Duration::from_millis(3)).unwrap();
    assert_eq!(script.borrow().accepted, prefix);
    executor.poll(Duration::from_millis(4)).unwrap();
    assert_eq!(executor.snapshot().state, ExecutorState::Idle);
    assert_eq!(executor.snapshot().generation, 2);
    executor.poll(Duration::from_millis(5)).unwrap();
    assert!(script.borrow().accepted.len() > prefix.len());
}

#[test]
fn retryable_read_gets_at_most_one_retry_after_recovery() {
    let script = Rc::new(RefCell::new(Script {
        max_write: 4,
        write_error_after: Some(0),
        request_lengths: VecDeque::from([2]),
        responses: VecDeque::from([vec![1, 2, 3]]),
        ..Script::default()
    }));
    let mut executor = executor(script.clone());
    executor
        .enqueue_read(
            &[7, 7],
            3,
            Duration::ZERO,
            Duration::from_secs(1),
            Duration::from_millis(20),
            true,
            1,
            1,
        )
        .unwrap();
    for millisecond in 0..5 {
        executor.poll(Duration::from_millis(millisecond)).unwrap();
    }
    assert_eq!(script.borrow().accepted, [7, 7]);
    assert_eq!(
        executor.snapshot().latest.unwrap().outcome,
        TransactionOutcome::Completed
    );
}
