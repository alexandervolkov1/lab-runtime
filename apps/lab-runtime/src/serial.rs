//! Bounded read-only COM worker implementing the existing M3 byte boundary.
//!
//! The Runtime owner calls only nonblocking [`lab_core::transport::ByteTransport`]
//! attempts. One
//! worker owns the blocking serial handle and one request/completion slot. It
//! neither parses Metakon frames nor retries a logical frame. A positive local
//! admission is send-start evidence because bytes may reach the device after
//! that point; it is not evidence that the OS or device accepted every byte.

use lab_core::{
    metakon::MAX_FRAME_BYTES,
    transport::{ByteTransport, RecoveryStatus, TransportIoError, TransportShutdown},
};
use std::{
    collections::VecDeque,
    io::{Read, Write},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const RECONNECT_OPEN_MAX_ATTEMPTS: usize = 64;
const RECONNECT_OPEN_RETRY_INTERVAL: Duration = Duration::from_millis(100);
const RETRY_STOP_POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Serial parity accepted by schema-v1 deployment configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SerialParity {
    /// No parity bit.
    None,
    /// Odd parity.
    Odd,
    /// Even parity.
    Even,
}

/// Serial flow-control policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SerialFlowControl {
    /// No flow control.
    None,
    /// XON/XOFF software flow control.
    Software,
    /// RTS/CTS hardware flow control.
    Hardware,
}

/// Validated immutable settings for one read-only COM worker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComSettings {
    resource_id: u64,
    port: String,
    baud_rate: u32,
    data_bits: u8,
    parity: SerialParity,
    stop_bits: u8,
    flow_control: SerialFlowControl,
    read_timeout: Duration,
    write_timeout: Duration,
    binding_generation: u64,
}

impl ComSettings {
    /// Validate the common no-flow-control settings used by the first bench.
    #[allow(clippy::too_many_arguments)]
    pub fn new_read_only(
        resource_id: u64,
        port: &str,
        baud_rate: u32,
        data_bits: u8,
        parity: SerialParity,
        stop_bits: u8,
        read_timeout: Duration,
        write_timeout: Duration,
        binding_generation: u64,
    ) -> Result<Self, SerialError> {
        Self::new(
            resource_id,
            port,
            baud_rate,
            data_bits,
            parity,
            stop_bits,
            SerialFlowControl::None,
            read_timeout,
            write_timeout,
            binding_generation,
        )
    }

    /// Validate every schema-v1 serial setting without opening a device.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        resource_id: u64,
        port: &str,
        baud_rate: u32,
        data_bits: u8,
        parity: SerialParity,
        stop_bits: u8,
        flow_control: SerialFlowControl,
        read_timeout: Duration,
        write_timeout: Duration,
        binding_generation: u64,
    ) -> Result<Self, SerialError> {
        let normalized = normalize_port(port)?;
        if resource_id == 0
            || binding_generation == 0
            || !(1..=4_000_000).contains(&baud_rate)
            || !matches!(data_bits, 5..=8)
            || !matches!(stop_bits, 1 | 2)
            || !valid_call_timeout(read_timeout)
            || !valid_call_timeout(write_timeout)
        {
            return Err(SerialError::InvalidSettings);
        }
        Ok(Self {
            resource_id,
            port: normalized,
            baud_rate,
            data_bits,
            parity,
            stop_bits,
            flow_control,
            read_timeout,
            write_timeout,
            binding_generation,
        })
    }
}

/// Bounded device-call failure, distinct from protocol validity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SerialError {
    /// Settings are outside the schema-v1 hard bounds.
    InvalidSettings,
    /// The device or cable is no longer available.
    Disconnected,
    /// One finite OS call reached its configured timeout/no-progress boundary.
    Timeout,
    /// Another finite serial I/O error occurred.
    Other,
}

/// Narrow blocking device owned only by the COM worker.
pub trait SerialDevice: Send + 'static {
    /// Perform at most one finite write call and report its actual prefix length.
    fn write_once(&mut self, bytes: &[u8]) -> Result<usize, SerialError>;
    /// Perform at most one finite read call, returning no more than `maximum` bytes.
    fn read_once(&mut self, maximum: usize) -> Result<Vec<u8>, SerialError>;
}

/// Observable adapter lifecycle copied without touching the OS handle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComState {
    /// Worker exists but has not reported a successful open.
    Opening,
    /// Current-generation worker accepted bounded byte requests.
    Online,
    /// An I/O failure requires a separately justified recovery boundary.
    Offline,
    /// Owner retired this session; no new byte admission is possible.
    Closing,
    /// Worker confirmed it stopped and released its handle.
    Closed,
}

/// Nonblocking result of preparing a newly spawned COM candidate.
///
/// `Ready` means that the worker completed the actual OS open and accepted the
/// configured settings readback. Constructing [`ComTransport`] proves only that
/// the bounded worker thread was spawned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComOpenStatus {
    /// The worker exists, but the actual OS open/readback has not completed.
    Opening,
    /// The OS handle was opened and the configured settings readback matched.
    Ready,
    /// The asynchronous open or settings readback failed with this typed class.
    Failed(SerialError),
}

/// Fixed retry policy for one worker-owned asynchronous open sequence.
///
/// The policy carries one absolute deadline. Only a completed transient
/// `Disconnected` attempt may advance to another attempt, so a hung factory
/// call never creates concurrent open workers.
#[derive(Clone, Copy, Debug)]
pub(crate) struct OpenRetryPolicy {
    deadline: Instant,
    retry_interval: Duration,
    max_attempts: usize,
}

impl OpenRetryPolicy {
    fn single_attempt() -> Self {
        Self {
            deadline: Instant::now(),
            retry_interval: Duration::ZERO,
            max_attempts: 1,
        }
    }

    /// Apply the reviewed reconnect bounds to one existing candidate worker.
    pub(crate) fn transient_until(deadline: Instant) -> Self {
        Self {
            deadline,
            retry_interval: RECONNECT_OPEN_RETRY_INTERVAL,
            max_attempts: RECONNECT_OPEN_MAX_ATTEMPTS,
        }
    }

    #[cfg(test)]
    pub(crate) fn for_test(
        deadline: Instant,
        retry_interval: Duration,
        max_attempts: usize,
    ) -> Self {
        assert!(max_attempts > 0);
        Self {
            deadline,
            retry_interval,
            max_attempts,
        }
    }

    fn should_retry(self, error: SerialError, attempt: usize, now: Instant) -> bool {
        error == SerialError::Disconnected && attempt < self.max_attempts && now < self.deadline
    }

    #[cfg(test)]
    fn deadline(self) -> Instant {
        self.deadline
    }
}

/// Bounded resource diagnostics; no handle or authority is exposed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComSnapshot {
    /// Stable logical resource identity across rebind.
    pub resource_id: u64,
    /// Current binding generation, separate from M3 resource generation.
    pub binding_generation: u64,
    /// Normalized bound COM name.
    pub port: String,
    /// Current worker/session state.
    pub state: ComState,
    /// Latest typed error, replacing rather than accumulating history.
    pub last_error: Option<SerialError>,
    /// Whether one fixed request is currently owned by the worker.
    pub request_pending: bool,
    /// The bounded worker thread was successfully spawned.
    pub worker_spawned: bool,
    /// The worker thread has returned; only this permits a completion-loss close.
    pub worker_finished: bool,
    /// The worker confirmed the actual OS open and settings readback.
    pub os_port_open_confirmed: bool,
    /// Completed or active OS-open attempts in this one worker, capped by policy.
    pub open_attempts: usize,
    /// Last typed failed open attempt, replacing rather than accumulating history.
    pub last_open_error: Option<SerialError>,
    /// Persistent retirement intent visible independently of mailbox capacity.
    pub stop_requested: bool,
}

enum Request {
    Write(Vec<u8>),
    Read(usize),
    Stop,
}

enum Completion {
    Opened,
    Written,
    Read(Vec<u8>),
    Failed(SerialError),
    Stopped,
}

#[derive(Clone)]
struct StopIntent(Arc<AtomicBool>);

impl StopIntent {
    fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }

    fn request(&self) {
        self.0.store(true, Ordering::Release);
    }

    fn requested(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

/// Nonblocking M3 adapter backed by one bounded blocking worker.
pub struct ComTransport {
    settings: ComSettings,
    requests: SyncSender<Request>,
    completions: Receiver<Completion>,
    worker: Option<JoinHandle<()>>,
    pending: bool,
    received: VecDeque<u8>,
    state: ComState,
    fault: Option<SerialError>,
    clean_boundary: bool,
    stop: StopIntent,
    open_confirmed: bool,
    open_attempts: Arc<AtomicUsize>,
    last_open_error: Arc<AtomicU8>,
}

impl ComTransport {
    /// Spawn the concrete serialport-backed worker without opening on the owner lane.
    pub fn open_windows(settings: ComSettings) -> Result<Self, SerialError> {
        let worker_settings = settings.clone();
        Self::spawn(settings, move || {
            SerialPortDevice::open(&worker_settings)
                .map(|device| Box::new(device) as Box<dyn SerialDevice>)
        })
    }

    /// Spawn one candidate worker with bounded transient-absence retry.
    pub(crate) fn open_windows_with_transient_retry(
        settings: ComSettings,
        deadline: Instant,
    ) -> Result<Self, SerialError> {
        let worker_settings = settings.clone();
        Self::spawn_retrying(
            settings,
            OpenRetryPolicy::transient_until(deadline),
            move || {
                SerialPortDevice::open(&worker_settings)
                    .map(|device| Box::new(device) as Box<dyn SerialDevice>)
            },
        )
    }

    /// Install an already-created deterministic device for software acceptance.
    ///
    /// This constructor is a trusted Rust test/composition seam and is not
    /// exported through the wire protocol.
    pub fn with_device(
        settings: ComSettings,
        device: impl SerialDevice,
    ) -> Result<Self, SerialError> {
        Self::spawn(settings, move || Ok(Box::new(device)))
    }

    /// Spawn from a deterministic device factory for trusted in-process tests.
    ///
    /// Unlike [`Self::with_device`], the factory runs on the worker so tests can
    /// reproduce an asynchronous open failure without touching a real COM port.
    #[cfg(test)]
    pub(crate) fn with_device_factory(
        settings: ComSettings,
        factory: impl FnOnce() -> Result<Box<dyn SerialDevice>, SerialError> + Send + 'static,
    ) -> Result<Self, SerialError> {
        Self::spawn(settings, factory)
    }

    #[cfg(test)]
    pub(crate) fn with_retrying_device_factory(
        settings: ComSettings,
        policy: OpenRetryPolicy,
        factory: impl FnMut() -> Result<Box<dyn SerialDevice>, SerialError> + Send + 'static,
    ) -> Result<Self, SerialError> {
        Self::spawn_retrying(settings, policy, factory)
    }

    fn spawn(
        settings: ComSettings,
        factory: impl FnOnce() -> Result<Box<dyn SerialDevice>, SerialError> + Send + 'static,
    ) -> Result<Self, SerialError> {
        let mut factory = Some(factory);
        Self::spawn_retrying(settings, OpenRetryPolicy::single_attempt(), move || {
            factory
                .take()
                .expect("single-attempt serial factory called once")()
        })
    }

    fn spawn_retrying(
        settings: ComSettings,
        retry: OpenRetryPolicy,
        factory: impl FnMut() -> Result<Box<dyn SerialDevice>, SerialError> + Send + 'static,
    ) -> Result<Self, SerialError> {
        let (request_tx, request_rx) = mpsc::sync_channel(1);
        let (completion_tx, completion_rx) = mpsc::sync_channel(1);
        let stop = StopIntent::new();
        let worker_stop = stop.clone();
        let open_attempts = Arc::new(AtomicUsize::new(0));
        let worker_open_attempts = open_attempts.clone();
        let last_open_error = Arc::new(AtomicU8::new(0));
        let worker_last_open_error = last_open_error.clone();
        let worker = std::thread::Builder::new()
            .name(format!("lab-com-{}", settings.resource_id))
            .spawn(move || {
                worker_main(
                    factory,
                    request_rx,
                    completion_tx,
                    worker_stop,
                    retry,
                    worker_open_attempts,
                    worker_last_open_error,
                )
            })
            .map_err(|_| SerialError::Other)?;
        Ok(Self {
            settings,
            requests: request_tx,
            completions: completion_rx,
            worker: Some(worker),
            pending: false,
            received: VecDeque::with_capacity(MAX_FRAME_BYTES),
            state: ComState::Opening,
            fault: None,
            clean_boundary: false,
            stop,
            open_confirmed: false,
            open_attempts,
            last_open_error,
        })
    }

    /// Poll asynchronous OS-open preparation without blocking the Runtime owner.
    pub fn open_status(&mut self) -> ComOpenStatus {
        self.drain_completion();
        match self.state {
            ComState::Online if self.open_confirmed => ComOpenStatus::Ready,
            ComState::Offline | ComState::Closing | ComState::Closed => {
                ComOpenStatus::Failed(self.fault.unwrap_or(SerialError::Other))
            }
            ComState::Opening | ComState::Online => ComOpenStatus::Opening,
        }
    }

    /// Copy current bounded diagnostics after draining at most one completion.
    pub fn snapshot(&self) -> ComSnapshot {
        ComSnapshot {
            resource_id: self.settings.resource_id,
            binding_generation: self.settings.binding_generation,
            port: self.settings.port.clone(),
            state: self.state,
            last_error: self.fault,
            request_pending: self.pending,
            worker_spawned: true,
            worker_finished: self
                .worker
                .as_ref()
                .is_none_or(|worker| worker.is_finished()),
            os_port_open_confirmed: self.open_confirmed,
            open_attempts: self.open_attempts.load(Ordering::Acquire),
            last_open_error: decode_serial_error(self.last_open_error.load(Ordering::Acquire)),
            stop_requested: self.stop.requested(),
        }
    }

    /// Record an externally performed device/bus reset boundary.
    ///
    /// This trusted composition call is only admissible after the old request is
    /// terminal. It is explicitly not ACK, readback or physical-safe evidence.
    pub fn confirm_operator_reset_boundary(&mut self) -> Result<(), SerialError> {
        self.drain_completion();
        if self.pending || self.fault.is_none() || self.state == ComState::Closing {
            return Err(SerialError::Other);
        }
        self.clean_boundary = true;
        Ok(())
    }

    /// Fence this session and request handle retirement without joining the worker.
    pub fn retire(&mut self) {
        self.stop.request();
        if self.state == ComState::Closed {
            return;
        }
        self.state = ComState::Closing;
        self.fault = Some(SerialError::Disconnected);
        // This is only a wake-up hint. The persistent intent above is the
        // authority, so a full ordinary mailbox cannot lose retirement.
        let _ = self.requests.try_send(Request::Stop);
    }

    fn drain_completion(&mut self) {
        match self.completions.try_recv() {
            Ok(Completion::Opened) if self.state != ComState::Closing => {
                self.open_confirmed = true;
                self.state = ComState::Online;
            }
            Ok(Completion::Opened) => self.open_confirmed = true,
            Ok(Completion::Written) => self.pending = false,
            Ok(Completion::Read(bytes)) => {
                self.pending = false;
                if self.state != ComState::Closing {
                    self.received
                        .extend(bytes.into_iter().take(MAX_FRAME_BYTES));
                }
            }
            Ok(Completion::Failed(error)) => {
                self.pending = false;
                self.fault = Some(error);
                if self.state != ComState::Closing {
                    self.state = ComState::Offline;
                }
            }
            Ok(Completion::Stopped) => {
                self.pending = false;
                self.state = ComState::Closed;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                if !matches!(self.state, ComState::Closing | ComState::Closed) {
                    self.pending = false;
                    self.fault.get_or_insert(SerialError::Disconnected);
                    self.state = ComState::Offline;
                }
            }
        }
        if self.state == ComState::Closing
            && self
                .worker
                .as_ref()
                .is_some_and(|worker| worker.is_finished())
        {
            // A finished JoinHandle proves that the worker released its owned
            // device even if the bounded Stopped completion could not be sent.
            self.pending = false;
            self.state = ComState::Closed;
        }
    }

    fn transport_error(&self) -> TransportIoError {
        match self.fault {
            Some(SerialError::Disconnected) => TransportIoError::Disconnected,
            _ => TransportIoError::Other,
        }
    }
}

impl ByteTransport for ComTransport {
    fn try_write(&mut self, bytes: &[u8]) -> Result<usize, TransportIoError> {
        self.drain_completion();
        if matches!(self.state, ComState::Closing | ComState::Closed) {
            return Err(TransportIoError::Disconnected);
        }
        if self.fault.is_some() {
            return Err(self.transport_error());
        }
        if bytes.is_empty() || bytes.len() > MAX_FRAME_BYTES {
            return Err(TransportIoError::Other);
        }
        if self.state == ComState::Opening || self.pending {
            return Ok(0);
        }
        match self.requests.try_send(Request::Write(bytes.to_vec())) {
            Ok(()) => {
                self.pending = true;
                Ok(bytes.len())
            }
            Err(TrySendError::Full(_)) => Ok(0),
            Err(TrySendError::Disconnected(_)) => Err(TransportIoError::Disconnected),
        }
    }

    fn try_read(&mut self, bytes: &mut [u8]) -> Result<usize, TransportIoError> {
        self.drain_completion();
        if self.fault.is_some() {
            return Err(self.transport_error());
        }
        let copied = bytes.len().min(self.received.len());
        for target in &mut bytes[..copied] {
            *target = self.received.pop_front().expect("bounded length checked");
        }
        if copied != 0 || bytes.is_empty() {
            return Ok(copied);
        }
        if self.state != ComState::Online || self.pending {
            return Ok(0);
        }
        let maximum = bytes.len().min(MAX_FRAME_BYTES);
        match self.requests.try_send(Request::Read(maximum)) {
            Ok(()) => self.pending = true,
            Err(TrySendError::Full(_)) => {}
            Err(TrySendError::Disconnected(_)) => return Err(TransportIoError::Disconnected),
        }
        Ok(0)
    }

    fn try_recover(&mut self) -> Result<RecoveryStatus, TransportIoError> {
        self.drain_completion();
        if matches!(self.state, ComState::Closing | ComState::Closed) {
            return Err(TransportIoError::Disconnected);
        }
        if self.clean_boundary && !self.pending {
            self.clean_boundary = false;
            self.fault = None;
            self.state = ComState::Online;
            return Ok(RecoveryStatus::Complete);
        }
        Ok(RecoveryStatus::Pending)
    }

    fn try_shutdown(&mut self) -> TransportShutdown {
        self.retire();
        self.drain_completion();
        if self.state != ComState::Closed {
            return TransportShutdown::Pending;
        }
        if self
            .worker
            .as_ref()
            .is_some_and(|worker| worker.is_finished())
            && let Some(worker) = self.worker.take()
        {
            let _ = worker.join();
        }
        TransportShutdown::Complete
    }
}

impl Drop for ComTransport {
    fn drop(&mut self) {
        self.retire();
        self.drain_completion();
        if self
            .worker
            .as_ref()
            .is_some_and(|worker| worker.is_finished())
            && let Some(worker) = self.worker.take()
        {
            let _ = worker.join();
        }
        // Dropping an unfinished JoinHandle detaches one already-counted worker;
        // no replacement is created by this adapter. A host supervisor must keep
        // the resource quarantined until process exit if the driver never returns.
    }
}

fn worker_main(
    mut factory: impl FnMut() -> Result<Box<dyn SerialDevice>, SerialError>,
    requests: Receiver<Request>,
    completions: SyncSender<Completion>,
    stop: StopIntent,
    retry: OpenRetryPolicy,
    open_attempts: Arc<AtomicUsize>,
    last_open_error: Arc<AtomicU8>,
) {
    let device = loop {
        if stop.requested() {
            let _ = completions.send(Completion::Stopped);
            return;
        }

        let attempt = open_attempts.fetch_add(1, Ordering::AcqRel) + 1;
        match factory() {
            Ok(device) => break device,
            Err(error) if retry.should_retry(error, attempt, Instant::now()) => {
                last_open_error.store(encode_serial_error(error), Ordering::Release);
                match wait_for_open_retry(retry, &stop) {
                    RetryWait::Ready => {}
                    RetryWait::Deadline => {
                        let _ = completions.send(Completion::Failed(error));
                        return;
                    }
                    RetryWait::Stopped => {
                        let _ = completions.send(Completion::Stopped);
                        return;
                    }
                }
            }
            Err(error) => {
                last_open_error.store(encode_serial_error(error), Ordering::Release);
                let _ = completions.send(Completion::Failed(error));
                return;
            }
        }
    };
    if stop.requested() {
        let _ = completions.send(Completion::Stopped);
        return;
    }
    if completions.send(Completion::Opened).is_err() {
        return;
    }
    worker_request_loop(device, requests, completions, stop);
}

const fn encode_serial_error(error: SerialError) -> u8 {
    match error {
        SerialError::InvalidSettings => 1,
        SerialError::Disconnected => 2,
        SerialError::Timeout => 3,
        SerialError::Other => 4,
    }
}

const fn decode_serial_error(encoded: u8) -> Option<SerialError> {
    match encoded {
        1 => Some(SerialError::InvalidSettings),
        2 => Some(SerialError::Disconnected),
        3 => Some(SerialError::Timeout),
        4 => Some(SerialError::Other),
        _ => None,
    }
}

enum RetryWait {
    Ready,
    Deadline,
    Stopped,
}

fn wait_for_open_retry(retry: OpenRetryPolicy, stop: &StopIntent) -> RetryWait {
    let now = Instant::now();
    let retry_at = now
        .checked_add(retry.retry_interval)
        .unwrap_or(retry.deadline)
        .min(retry.deadline);
    loop {
        if stop.requested() {
            return RetryWait::Stopped;
        }
        let now = Instant::now();
        if now >= retry.deadline {
            return RetryWait::Deadline;
        }
        if now >= retry_at {
            return RetryWait::Ready;
        }
        thread::park_timeout((retry_at - now).min(RETRY_STOP_POLL_INTERVAL));
    }
}

fn worker_request_loop(
    mut device: Box<dyn SerialDevice>,
    requests: Receiver<Request>,
    completions: SyncSender<Completion>,
    stop: StopIntent,
) {
    // Stop is coalesced state, not an ordinary queued command. Checking before
    // and after every bounded OS call prevents queued data from crossing the
    // retirement fence even when the one-slot mailbox was full at retirement.
    if stop.requested() {
        let _ = completions.send(Completion::Stopped);
        return;
    }
    while let Ok(request) = requests.recv() {
        if stop.requested() {
            let _ = completions.send(Completion::Stopped);
            return;
        }
        let completion = match request {
            Request::Write(bytes) => write_same_frame(device.as_mut(), &bytes),
            Request::Read(maximum) => device.read_once(maximum).map(Completion::Read),
            Request::Stop => {
                stop.request();
                let _ = completions.send(Completion::Stopped);
                return;
            }
        };
        let completion = match completion {
            Ok(completion) => completion,
            Err(error) => Completion::Failed(error),
        };
        if completions.send(completion).is_err() {
            return;
        }
        if stop.requested() {
            let _ = completions.send(Completion::Stopped);
            return;
        }
    }
}

fn write_same_frame(
    device: &mut dyn SerialDevice,
    bytes: &[u8],
) -> Result<Completion, SerialError> {
    let mut offset = 0usize;
    while offset < bytes.len() {
        let written = device.write_once(&bytes[offset..])?;
        if written == 0 || written > bytes.len() - offset {
            return Err(SerialError::Timeout);
        }
        offset += written;
    }
    Ok(Completion::Written)
}

struct SerialPortDevice {
    port: Box<dyn serialport::SerialPort>,
    read_timeout: Duration,
    write_timeout: Duration,
}

impl SerialPortDevice {
    fn open(settings: &ComSettings) -> Result<Self, SerialError> {
        let builder = serialport::new(&settings.port, settings.baud_rate)
            .data_bits(match settings.data_bits {
                5 => serialport::DataBits::Five,
                6 => serialport::DataBits::Six,
                7 => serialport::DataBits::Seven,
                _ => serialport::DataBits::Eight,
            })
            .parity(match settings.parity {
                SerialParity::None => serialport::Parity::None,
                SerialParity::Odd => serialport::Parity::Odd,
                SerialParity::Even => serialport::Parity::Even,
            })
            .stop_bits(if settings.stop_bits == 2 {
                serialport::StopBits::Two
            } else {
                serialport::StopBits::One
            })
            .flow_control(match settings.flow_control {
                SerialFlowControl::None => serialport::FlowControl::None,
                SerialFlowControl::Software => serialport::FlowControl::Software,
                SerialFlowControl::Hardware => serialport::FlowControl::Hardware,
            })
            .timeout(settings.read_timeout);
        let port = builder.open().map_err(map_serialport_error)?;
        if port.baud_rate().map_err(map_serialport_error)? != settings.baud_rate
            || port.data_bits().map_err(map_serialport_error)?
                != match settings.data_bits {
                    5 => serialport::DataBits::Five,
                    6 => serialport::DataBits::Six,
                    7 => serialport::DataBits::Seven,
                    _ => serialport::DataBits::Eight,
                }
            || port.parity().map_err(map_serialport_error)?
                != match settings.parity {
                    SerialParity::None => serialport::Parity::None,
                    SerialParity::Odd => serialport::Parity::Odd,
                    SerialParity::Even => serialport::Parity::Even,
                }
            || port.stop_bits().map_err(map_serialport_error)?
                != if settings.stop_bits == 2 {
                    serialport::StopBits::Two
                } else {
                    serialport::StopBits::One
                }
            || port.flow_control().map_err(map_serialport_error)?
                != match settings.flow_control {
                    SerialFlowControl::None => serialport::FlowControl::None,
                    SerialFlowControl::Software => serialport::FlowControl::Software,
                    SerialFlowControl::Hardware => serialport::FlowControl::Hardware,
                }
        {
            return Err(SerialError::InvalidSettings);
        }
        Ok(Self {
            port,
            read_timeout: settings.read_timeout,
            write_timeout: settings.write_timeout,
        })
    }
}

impl SerialDevice for SerialPortDevice {
    fn write_once(&mut self, bytes: &[u8]) -> Result<usize, SerialError> {
        self.port
            .set_timeout(self.write_timeout)
            .map_err(map_serialport_error)?;
        self.port.write(bytes).map_err(map_io_error)
    }

    fn read_once(&mut self, maximum: usize) -> Result<Vec<u8>, SerialError> {
        self.port
            .set_timeout(self.read_timeout)
            .map_err(map_serialport_error)?;
        let mut bytes = vec![0; maximum.min(MAX_FRAME_BYTES)];
        let read = self.port.read(&mut bytes).map_err(map_io_error)?;
        bytes.truncate(read);
        Ok(bytes)
    }
}

fn map_serialport_error(error: serialport::Error) -> SerialError {
    match error.kind() {
        serialport::ErrorKind::NoDevice => SerialError::Disconnected,
        serialport::ErrorKind::InvalidInput => SerialError::InvalidSettings,
        _ => SerialError::Other,
    }
}

fn map_io_error(error: std::io::Error) -> SerialError {
    match error.kind() {
        std::io::ErrorKind::NotFound
        | std::io::ErrorKind::BrokenPipe
        | std::io::ErrorKind::ConnectionReset
        | std::io::ErrorKind::UnexpectedEof => SerialError::Disconnected,
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => SerialError::Timeout,
        _ => SerialError::Other,
    }
}

fn valid_call_timeout(timeout: Duration) -> bool {
    (Duration::from_millis(1)..=Duration::from_millis(250)).contains(&timeout)
}

fn normalize_port(port: &str) -> Result<String, SerialError> {
    let trimmed = port.trim();
    let digits = trimmed
        .strip_prefix("COM")
        .or_else(|| trimmed.strip_prefix("com"))
        .ok_or(SerialError::InvalidSettings)?;
    let number = digits
        .parse::<u16>()
        .map_err(|_| SerialError::InvalidSettings)?;
    if number == 0 {
        return Err(SerialError::InvalidSettings);
    }
    Ok(format!("COM{number}"))
}

#[cfg(test)]
mod retirement_tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };

    struct HeldDevice {
        entered: Arc<AtomicBool>,
        release: Arc<AtomicBool>,
        reads: Arc<AtomicUsize>,
    }

    impl SerialDevice for HeldDevice {
        fn write_once(&mut self, bytes: &[u8]) -> Result<usize, SerialError> {
            self.entered.store(true, Ordering::Release);
            while !self.release.load(Ordering::Acquire) {
                std::thread::yield_now();
            }
            Ok(bytes.len())
        }

        fn read_once(&mut self, _: usize) -> Result<Vec<u8>, SerialError> {
            self.reads.fetch_add(1, Ordering::AcqRel);
            Ok(Vec::new())
        }
    }

    fn wait_until(mut predicate: impl FnMut() -> bool) {
        for _ in 0..100_000 {
            if predicate() {
                return;
            }
            std::thread::yield_now();
        }
        panic!("bounded worker did not make progress");
    }

    #[test]
    fn occupied_mailbox_retirement_cannot_lose_stop_or_start_next_data_operation() {
        let entered = Arc::new(AtomicBool::new(false));
        let release = Arc::new(AtomicBool::new(false));
        let reads = Arc::new(AtomicUsize::new(0));
        let stop = StopIntent::new();
        let (request_tx, request_rx) = mpsc::sync_channel(1);
        let (completion_tx, completion_rx) = mpsc::sync_channel(2);
        let worker_stop = stop.clone();
        let worker = std::thread::spawn({
            let entered = entered.clone();
            let release = release.clone();
            let reads = reads.clone();
            move || {
                worker_request_loop(
                    Box::new(HeldDevice {
                        entered,
                        release,
                        reads,
                    }),
                    request_rx,
                    completion_tx,
                    worker_stop,
                )
            }
        });

        request_tx.send(Request::Write(vec![1])).unwrap();
        wait_until(|| entered.load(Ordering::Acquire));
        request_tx.send(Request::Read(1)).unwrap();
        stop.request();
        assert!(matches!(
            request_tx.try_send(Request::Stop),
            Err(TrySendError::Full(_))
        ));
        release.store(true, Ordering::Release);

        worker.join().unwrap();
        assert_eq!(reads.load(Ordering::Acquire), 0);
        assert!(matches!(completion_rx.recv().unwrap(), Completion::Written));
        assert!(matches!(completion_rx.recv().unwrap(), Completion::Stopped));
    }

    #[test]
    fn finished_worker_without_stopped_completion_is_honestly_closed() {
        let mut transport = ComTransport::spawn(test_settings(), || Err(SerialError::Other))
            .expect("worker thread should spawn");
        wait_until(|| {
            transport
                .worker
                .as_ref()
                .is_some_and(|worker| worker.is_finished())
        });

        assert_eq!(transport.try_shutdown(), TransportShutdown::Complete);
        assert_eq!(transport.snapshot().state, ComState::Closed);
    }

    #[test]
    fn finished_worker_with_closed_completion_channel_is_honestly_closed() {
        let mut transport = ComTransport::spawn(test_settings(), || {
            panic!("deterministic worker exit before completion")
        })
        .expect("worker thread should spawn");
        wait_until(|| {
            transport
                .worker
                .as_ref()
                .is_some_and(|worker| worker.is_finished())
        });

        assert_eq!(transport.try_shutdown(), TransportShutdown::Complete);
        assert_eq!(transport.snapshot().state, ComState::Closed);
    }

    #[test]
    fn asynchronous_open_failure_is_distinct_from_ready() {
        let release = Arc::new(AtomicBool::new(false));
        let worker_release = release.clone();
        let mut failed = ComTransport::spawn(test_settings(), move || {
            while !worker_release.load(Ordering::Acquire) {
                std::thread::yield_now();
            }
            Err(SerialError::Disconnected)
        })
        .expect("worker thread should spawn");
        assert_eq!(failed.open_status(), ComOpenStatus::Opening);
        release.store(true, Ordering::Release);
        wait_until(|| failed.open_status() != ComOpenStatus::Opening);
        assert_eq!(
            failed.open_status(),
            ComOpenStatus::Failed(SerialError::Disconnected)
        );

        let mut ready = ComTransport::with_device(test_settings(), ReadyDevice)
            .expect("worker thread should spawn");
        wait_until(|| ready.open_status() != ComOpenStatus::Opening);
        assert_eq!(ready.open_status(), ComOpenStatus::Ready);
    }

    #[test]
    fn transient_disconnected_open_retries_inside_one_worker_then_becomes_ready() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let factory_attempts = attempts.clone();
        let policy = OpenRetryPolicy::for_test(
            std::time::Instant::now() + Duration::from_secs(1),
            Duration::ZERO,
            4,
        );
        let mut transport =
            ComTransport::with_retrying_device_factory(test_settings(), policy, move || {
                if factory_attempts.fetch_add(1, Ordering::AcqRel) == 0 {
                    Err(SerialError::Disconnected)
                } else {
                    Ok(Box::new(ReadyDevice))
                }
            })
            .expect("one worker should spawn");

        wait_until(|| transport.open_status() != ComOpenStatus::Opening);
        assert_eq!(transport.open_status(), ComOpenStatus::Ready);
        assert_eq!(attempts.load(Ordering::Acquire), 2);
        assert_eq!(transport.snapshot().open_attempts, 2);
        assert!(transport.snapshot().os_port_open_confirmed);
    }

    #[test]
    fn several_transient_opens_are_bounded_and_eventually_ready() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let factory_attempts = attempts.clone();
        let policy = OpenRetryPolicy::for_test(
            std::time::Instant::now() + Duration::from_secs(1),
            Duration::ZERO,
            5,
        );
        let mut transport =
            ComTransport::with_retrying_device_factory(test_settings(), policy, move || {
                let attempt = factory_attempts.fetch_add(1, Ordering::AcqRel) + 1;
                if attempt < 4 {
                    Err(SerialError::Disconnected)
                } else {
                    Ok(Box::new(ReadyDevice))
                }
            })
            .expect("one worker should spawn");

        wait_until(|| transport.open_status() != ComOpenStatus::Opening);
        assert_eq!(transport.open_status(), ComOpenStatus::Ready);
        assert_eq!(attempts.load(Ordering::Acquire), 4);
        assert_eq!(transport.snapshot().open_attempts, 4);
    }

    #[test]
    fn transient_absence_stops_at_the_original_attempt_bound() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let factory_attempts = attempts.clone();
        let policy = OpenRetryPolicy::for_test(
            std::time::Instant::now() + Duration::from_secs(1),
            Duration::ZERO,
            3,
        );
        let mut transport =
            ComTransport::with_retrying_device_factory(test_settings(), policy, move || {
                factory_attempts.fetch_add(1, Ordering::AcqRel);
                Err(SerialError::Disconnected)
            })
            .expect("one worker should spawn");

        wait_until(|| transport.open_status() != ComOpenStatus::Opening);
        assert_eq!(
            transport.open_status(),
            ComOpenStatus::Failed(SerialError::Disconnected)
        );
        assert_eq!(attempts.load(Ordering::Acquire), 3);
        assert_eq!(transport.snapshot().open_attempts, 3);
        wait_until(|| transport.try_shutdown() == TransportShutdown::Complete);
    }

    #[test]
    fn invalid_settings_and_other_open_errors_are_not_retried() {
        for error in [
            SerialError::InvalidSettings,
            SerialError::Timeout,
            SerialError::Other,
        ] {
            let attempts = Arc::new(AtomicUsize::new(0));
            let factory_attempts = attempts.clone();
            let policy = OpenRetryPolicy::for_test(
                std::time::Instant::now() + Duration::from_secs(1),
                Duration::ZERO,
                4,
            );
            let mut transport =
                ComTransport::with_retrying_device_factory(test_settings(), policy, move || {
                    factory_attempts.fetch_add(1, Ordering::AcqRel);
                    Err(error)
                })
                .expect("one worker should spawn");

            wait_until(|| transport.open_status() != ComOpenStatus::Opening);
            assert_eq!(transport.open_status(), ComOpenStatus::Failed(error));
            assert_eq!(attempts.load(Ordering::Acquire), 1);
            assert_eq!(transport.snapshot().open_attempts, 1);
        }
    }

    #[test]
    fn hung_open_has_one_attempt_and_no_concurrent_replacement_worker() {
        let entered = Arc::new(AtomicBool::new(false));
        let release = Arc::new(AtomicBool::new(false));
        let active = Arc::new(AtomicUsize::new(0));
        let maximum_active = Arc::new(AtomicUsize::new(0));
        let worker_entered = entered.clone();
        let worker_release = release.clone();
        let worker_active = active.clone();
        let worker_maximum = maximum_active.clone();
        let policy = OpenRetryPolicy::for_test(
            std::time::Instant::now() + Duration::from_millis(10),
            Duration::ZERO,
            4,
        );
        let mut transport =
            ComTransport::with_retrying_device_factory(test_settings(), policy, move || {
                let now_active = worker_active.fetch_add(1, Ordering::AcqRel) + 1;
                worker_maximum.fetch_max(now_active, Ordering::AcqRel);
                worker_entered.store(true, Ordering::Release);
                while !worker_release.load(Ordering::Acquire) {
                    std::thread::yield_now();
                }
                worker_active.fetch_sub(1, Ordering::AcqRel);
                Ok(Box::new(ReadyDevice))
            })
            .expect("one worker should spawn");

        wait_until(|| entered.load(Ordering::Acquire));
        assert_eq!(transport.open_status(), ComOpenStatus::Opening);
        assert_eq!(transport.snapshot().open_attempts, 1);
        assert_eq!(maximum_active.load(Ordering::Acquire), 1);
        assert_eq!(transport.try_shutdown(), TransportShutdown::Pending);
        assert_eq!(transport.snapshot().open_attempts, 1);

        release.store(true, Ordering::Release);
        wait_until(|| transport.try_shutdown() == TransportShutdown::Complete);
        assert_eq!(maximum_active.load(Ordering::Acquire), 1);
    }

    #[test]
    fn shutdown_during_transient_retry_wait_is_finite() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let factory_attempts = attempts.clone();
        let (attempted_tx, attempted_rx) = mpsc::sync_channel(0);
        let policy = OpenRetryPolicy::for_test(
            std::time::Instant::now() + Duration::from_secs(10),
            Duration::from_secs(5),
            4,
        );
        let mut transport =
            ComTransport::with_retrying_device_factory(test_settings(), policy, move || {
                factory_attempts.fetch_add(1, Ordering::AcqRel);
                attempted_tx
                    .send(())
                    .expect("test must observe the first open attempt");
                Err(SerialError::Disconnected)
            })
            .expect("one worker should spawn");

        attempted_rx
            .recv()
            .expect("worker must publish the first open attempt");
        if transport.try_shutdown() == TransportShutdown::Pending {
            // StopIntent is the authority; unpark is only deterministic test
            // synchronization so the test does not exhaust a yield-count budget
            // while the production worker is correctly inside its 10 ms poll.
            transport
                .worker
                .as_ref()
                .expect("pending shutdown retains the one worker")
                .thread()
                .unpark();
            wait_until(|| transport.try_shutdown() == TransportShutdown::Complete);
        }
        assert_eq!(attempts.load(Ordering::Acquire), 1);
        assert_eq!(transport.snapshot().state, ComState::Closed);
    }

    #[test]
    fn retry_policy_uses_one_deadline_and_an_explicit_attempt_cap() {
        let start = std::time::Instant::now();
        let deadline = start + Duration::from_secs(1);
        let policy = OpenRetryPolicy::for_test(deadline, Duration::from_millis(10), 3);

        assert!(policy.should_retry(SerialError::Disconnected, 1, start));
        assert!(policy.should_retry(SerialError::Disconnected, 2, start));
        assert!(!policy.should_retry(SerialError::Disconnected, 3, start));
        assert!(!policy.should_retry(SerialError::Disconnected, 1, deadline));
        assert!(!policy.should_retry(SerialError::InvalidSettings, 1, start));
        assert!(!policy.should_retry(SerialError::Timeout, 1, start));
        assert!(!policy.should_retry(SerialError::Other, 1, start));
        assert_eq!(policy.deadline(), deadline);

        let production = OpenRetryPolicy::transient_until(deadline);
        assert_eq!(production.max_attempts, RECONNECT_OPEN_MAX_ATTEMPTS);
        assert_eq!(production.retry_interval, RECONNECT_OPEN_RETRY_INTERVAL);
        assert_eq!(production.deadline(), deadline);
    }

    struct ReadyDevice;

    impl SerialDevice for ReadyDevice {
        fn write_once(&mut self, bytes: &[u8]) -> Result<usize, SerialError> {
            Ok(bytes.len())
        }

        fn read_once(&mut self, _: usize) -> Result<Vec<u8>, SerialError> {
            Ok(Vec::new())
        }
    }

    fn test_settings() -> ComSettings {
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
}
