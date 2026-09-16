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
    sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
    thread::JoinHandle,
    time::Duration,
};

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

    fn spawn(
        settings: ComSettings,
        factory: impl FnOnce() -> Result<Box<dyn SerialDevice>, SerialError> + Send + 'static,
    ) -> Result<Self, SerialError> {
        let (request_tx, request_rx) = mpsc::sync_channel(1);
        let (completion_tx, completion_rx) = mpsc::sync_channel(1);
        let worker = std::thread::Builder::new()
            .name(format!("lab-com-{}", settings.resource_id))
            .spawn(move || worker_main(factory, request_rx, completion_tx))
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
        })
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
        if matches!(self.state, ComState::Closing | ComState::Closed) {
            return;
        }
        self.state = ComState::Closing;
        self.fault = Some(SerialError::Disconnected);
        let _ = self.requests.try_send(Request::Stop);
    }

    fn drain_completion(&mut self) {
        match self.completions.try_recv() {
            Ok(Completion::Opened) => self.state = ComState::Online,
            Ok(Completion::Written) => self.pending = false,
            Ok(Completion::Read(bytes)) => {
                self.pending = false;
                self.received
                    .extend(bytes.into_iter().take(MAX_FRAME_BYTES));
            }
            Ok(Completion::Failed(error)) => {
                self.pending = false;
                self.fault = Some(error);
                self.state = ComState::Offline;
            }
            Ok(Completion::Stopped) => {
                self.pending = false;
                self.state = ComState::Closed;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                if self.state != ComState::Closed {
                    self.pending = false;
                    self.fault.get_or_insert(SerialError::Disconnected);
                    self.state = ComState::Offline;
                }
            }
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
    factory: impl FnOnce() -> Result<Box<dyn SerialDevice>, SerialError>,
    requests: Receiver<Request>,
    completions: SyncSender<Completion>,
) {
    let mut device = match factory() {
        Ok(device) => {
            if completions.send(Completion::Opened).is_err() {
                return;
            }
            device
        }
        Err(error) => {
            let _ = completions.send(Completion::Failed(error));
            return;
        }
    };
    while let Ok(request) = requests.recv() {
        let completion = match request {
            Request::Write(bytes) => write_same_frame(device.as_mut(), &bytes),
            Request::Read(maximum) => device.read_once(maximum).map(Completion::Read),
            Request::Stop => {
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
