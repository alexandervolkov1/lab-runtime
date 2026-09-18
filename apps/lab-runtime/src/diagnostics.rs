//! Bounded best-effort troubleshooting diagnostics.
//!
//! Diagnostic records are observational process information, never authoritative
//! experiment state. [`crate::recorder`] remains the durable scientific/audit
//! history. Producers use a fixed lossy queue, so a slow or failed log destination
//! cannot block acquisition, control, output safety, or Recorder admission.

use std::{
    fmt,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tracing::Level;
use tracing_appender::non_blocking::{ErrorCounter, NonBlocking, WorkerGuard};
use tracing_subscriber::fmt::{MakeWriter, format::Writer, time::FormatTime};

/// Maximum bytes in each diagnostic file.
pub const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;
/// Active file plus retained rotations.
pub const RETAINED_FILE_COUNT: usize = 4;
/// Maximum number of formatted records waiting for the diagnostic worker.
pub const QUEUE_RECORDS: usize = 1024;
/// Maximum bytes admitted for one formatted diagnostic record.
pub const MAX_RECORD_BYTES: usize = 8 * 1024;

const ACTIVE_FILE: &str = "lab-runtime.log";
const TRUNCATED: &[u8] = b"...[truncated]\n";
const DIRECTORY_ENV: &str = "LAB_RUNTIME_LOG_DIRECTORY";
const LEVEL_ENV: &str = "LAB_RUNTIME_LOG_LEVEL";

/// Production diagnostic configuration.
#[derive(Clone, Debug)]
struct DiagnosticConfig {
    /// Directory containing the active and rotated diagnostic files.
    directory: PathBuf,
    /// Most verbose enabled level.
    level: Level,
    /// Strict maximum bytes per file.
    max_file_bytes: u64,
    /// Active file plus retained rotations.
    retained_file_count: usize,
    /// Fixed producer queue capacity in complete records.
    queue_records: usize,
    /// Maximum bytes admitted per formatted record.
    max_record_bytes: usize,
    /// Mirror accepted records to stderr from the background writer.
    mirror_stderr: bool,
}

impl DiagnosticConfig {
    /// Safe fixed defaults for a developer-preview process.
    fn production() -> Self {
        let directory = std::env::var_os(DIRECTORY_ENV)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(default_log_directory);
        let level = match std::env::var(LEVEL_ENV) {
            Ok(value) => match value.to_ascii_uppercase().as_str() {
                "ERROR" => Level::ERROR,
                "WARN" => Level::WARN,
                "INFO" => Level::INFO,
                "DEBUG" => Level::DEBUG,
                "TRACE" => Level::TRACE,
                _ => {
                    eprintln!("WARN diagnostic_level_invalid fallback=INFO");
                    Level::INFO
                }
            },
            Err(_) => Level::INFO,
        };
        Self {
            directory,
            level,
            max_file_bytes: MAX_FILE_BYTES,
            retained_file_count: RETAINED_FILE_COUNT,
            queue_records: QUEUE_RECORDS,
            max_record_bytes: MAX_RECORD_BYTES,
            mirror_stderr: true,
        }
    }

    fn validate(&self) -> io::Result<()> {
        if self.max_file_bytes == 0
            || self.retained_file_count == 0
            || self.queue_records == 0
            || self.max_record_bytes <= TRUNCATED.len()
            || self.max_record_bytes as u64 > self.max_file_bytes
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid bounded diagnostic configuration",
            ));
        }
        Ok(())
    }
}

/// Process-wide diagnostic handle and bounded shutdown observation.
pub struct Diagnostics {
    directory: PathBuf,
    level: Level,
    accepting: Arc<AtomicBool>,
    attempted: Arc<AtomicU64>,
    written: Arc<AtomicU64>,
    dropped: ErrorCounter,
    guard: Option<WorkerGuard>,
}

impl Diagnostics {
    /// Install the global bounded subscriber using production defaults.
    pub fn install() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let (subscriber, diagnostics) = build(DiagnosticConfig::production())?;
        tracing::subscriber::set_global_default(subscriber)?;
        install_panic_hook();
        Ok(diagnostics)
    }

    /// Effective file directory selected before process readiness.
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// Effective maximum diagnostic verbosity.
    pub const fn level(&self) -> Level {
        self.level
    }

    /// Stop accepting diagnostics and wait at most `timeout` for admitted records.
    ///
    /// This is best effort. On timeout the library worker guard is deliberately
    /// detached rather than joined, so diagnostics cannot extend authoritative
    /// Runtime shutdown indefinitely. A drained worker then uses the logging
    /// library's own bounded shutdown handshake (currently at most 1.1 seconds).
    pub fn finish(mut self, timeout: Duration) -> bool {
        self.accepting.store(false, Ordering::Release);
        let target = self
            .attempted
            .load(Ordering::Acquire)
            .saturating_sub(self.dropped.dropped_lines() as u64);
        let deadline = Instant::now() + timeout;
        while self.written.load(Ordering::Acquire) < target && Instant::now() < deadline {
            std::thread::yield_now();
        }
        let drained = self.written.load(Ordering::Acquire) >= target;
        let dropped = self.dropped.dropped_lines();
        if dropped != 0 {
            eprintln!("WARN diagnostic_queue_overflow total={dropped}");
        }
        if let Some(guard) = self.guard.take() {
            if drained {
                drop(guard);
            } else {
                std::mem::forget(guard);
                eprintln!("WARN diagnostic_shutdown_flush_timeout");
            }
        }
        drained
    }
}

impl Drop for Diagnostics {
    fn drop(&mut self) {
        self.accepting.store(false, Ordering::Release);
        if let Some(guard) = self.guard.take() {
            // An implicit drop must never join a blocked diagnostic destination.
            std::mem::forget(guard);
        }
    }
}

/// Windows uses `%LOCALAPPDATA%\lab-runtime\logs`; other platforms use the
/// equivalent local data directory when present and a temporary fallback otherwise.
pub fn default_log_directory() -> PathBuf {
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(local).join("lab-runtime").join("logs");
    }
    if let Some(state) = std::env::var_os("XDG_STATE_HOME") {
        return PathBuf::from(state).join("lab-runtime").join("logs");
    }
    std::env::temp_dir().join("lab-runtime").join("logs")
}

fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let detail = bounded_text(&info.to_string(), 1024);
        tracing::error!(event = "process_panic", detail, "process panic");
        previous(info);
    }));
}

fn bounded_text(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

#[derive(Clone)]
struct DiagnosticTimer {
    origin: Instant,
}

impl FormatTime for DiagnosticTimer {
    fn format_time(&self, writer: &mut Writer<'_>) -> fmt::Result {
        tracing_subscriber::fmt::time::UtcTime::rfc_3339().format_time(writer)?;
        write!(
            writer,
            " monotonic_ms={}",
            self.origin.elapsed().as_millis()
        )
    }
}

type DiagnosticSubscriber = tracing_subscriber::FmtSubscriber<
    tracing_subscriber::fmt::format::DefaultFields,
    tracing_subscriber::fmt::format::Format<tracing_subscriber::fmt::format::Full, DiagnosticTimer>,
    LevelFilter,
    CappedMakeWriter,
>;

use tracing_subscriber::filter::LevelFilter;

fn build(
    config: DiagnosticConfig,
) -> Result<(DiagnosticSubscriber, Diagnostics), Box<dyn std::error::Error + Send + Sync>> {
    config.validate()?;
    let directory = config.directory.clone();
    let level = config.level;
    let accepting = Arc::new(AtomicBool::new(true));
    let attempted = Arc::new(AtomicU64::new(0));
    let written = Arc::new(AtomicU64::new(0));
    let drop_counter = Arc::new(OnceLock::new());
    let sink = RotatingSink::new(
        config.directory,
        config.max_file_bytes,
        config.retained_file_count,
        config.mirror_stderr,
        written.clone(),
        drop_counter.clone(),
    );
    let (writer, guard) = tracing_appender::non_blocking::NonBlockingBuilder::default()
        .buffered_lines_limit(config.queue_records)
        .lossy(true)
        .thread_name("lab-diagnostics")
        .finish(sink);
    let dropped = writer.error_counter();
    let _ = drop_counter.set(dropped.clone());
    let make_writer = CappedMakeWriter {
        writer,
        accepting: accepting.clone(),
        attempted: attempted.clone(),
        max_record_bytes: config.max_record_bytes,
    };
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_target(true)
        .with_thread_names(true)
        .with_timer(DiagnosticTimer {
            origin: Instant::now(),
        })
        .with_max_level(config.level)
        .with_writer(make_writer)
        .finish();
    Ok((
        subscriber,
        Diagnostics {
            directory,
            level,
            accepting,
            attempted,
            written,
            dropped,
            guard: Some(guard),
        },
    ))
}

#[derive(Clone)]
struct CappedMakeWriter {
    writer: NonBlocking,
    accepting: Arc<AtomicBool>,
    attempted: Arc<AtomicU64>,
    max_record_bytes: usize,
}

impl<'a> MakeWriter<'a> for CappedMakeWriter {
    type Writer = CappedRecord;

    fn make_writer(&'a self) -> Self::Writer {
        CappedRecord {
            writer: self.writer.clone(),
            accepting: self.accepting.clone(),
            attempted: self.attempted.clone(),
            bytes: Vec::with_capacity(self.max_record_bytes.min(1024)),
            max_record_bytes: self.max_record_bytes,
            truncated: false,
        }
    }
}

struct CappedRecord {
    writer: NonBlocking,
    accepting: Arc<AtomicBool>,
    attempted: Arc<AtomicU64>,
    bytes: Vec<u8>,
    max_record_bytes: usize,
    truncated: bool,
}

impl Write for CappedRecord {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let payload_limit = self.max_record_bytes - TRUNCATED.len();
        let available = payload_limit.saturating_sub(self.bytes.len());
        let admitted = available.min(bytes.len());
        self.bytes.extend_from_slice(&bytes[..admitted]);
        self.truncated |= admitted < bytes.len();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for CappedRecord {
    fn drop(&mut self) {
        if !self.accepting.load(Ordering::Acquire) || self.bytes.is_empty() {
            return;
        }
        if self.truncated {
            while self.bytes.last() == Some(&b'\n') {
                self.bytes.pop();
            }
            self.bytes.extend_from_slice(TRUNCATED);
        } else if self.bytes.last() != Some(&b'\n') {
            self.bytes.push(b'\n');
        }
        self.attempted.fetch_add(1, Ordering::AcqRel);
        let _ = self.writer.write_all(&self.bytes);
    }
}

struct RotatingSink {
    directory: PathBuf,
    active: PathBuf,
    file: Option<File>,
    file_len: u64,
    max_file_bytes: u64,
    retained_file_count: usize,
    mirror_stderr: bool,
    file_failed: bool,
    failure_reported: bool,
    written: Arc<AtomicU64>,
    drop_counter: Arc<OnceLock<ErrorCounter>>,
    reported_drops: usize,
}

impl RotatingSink {
    fn new(
        directory: PathBuf,
        max_file_bytes: u64,
        retained_file_count: usize,
        mirror_stderr: bool,
        written: Arc<AtomicU64>,
        drop_counter: Arc<OnceLock<ErrorCounter>>,
    ) -> Self {
        let active = directory.join(ACTIVE_FILE);
        let mut sink = Self {
            directory,
            active,
            file: None,
            file_len: 0,
            max_file_bytes,
            retained_file_count,
            mirror_stderr,
            file_failed: false,
            failure_reported: false,
            written,
            drop_counter,
            reported_drops: 0,
        };
        if let Err(error) = sink.open() {
            sink.disable_file(&error);
        }
        sink
    }

    fn open(&mut self) -> io::Result<()> {
        fs::create_dir_all(&self.directory)?;
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.active)?;
        self.file_len = file.metadata()?.len();
        self.file = Some(file);
        Ok(())
    }

    fn rotated(&self, index: usize) -> PathBuf {
        self.directory.join(format!("{ACTIVE_FILE}.{index}"))
    }

    fn rotate(&mut self) -> io::Result<()> {
        self.file.take();
        let last = self.retained_file_count - 1;
        if last > 0 {
            let oldest = self.rotated(last);
            if oldest.exists() {
                fs::remove_file(&oldest)?;
            }
            for index in (1..last).rev() {
                let source = self.rotated(index);
                if source.exists() {
                    fs::rename(source, self.rotated(index + 1))?;
                }
            }
            if self.active.exists() {
                fs::rename(&self.active, self.rotated(1))?;
            }
        } else if self.active.exists() {
            fs::remove_file(&self.active)?;
        }
        self.file_len = 0;
        self.file = Some(
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.active)?,
        );
        Ok(())
    }

    fn disable_file(&mut self, error: &io::Error) {
        self.file = None;
        self.file_failed = true;
        if !self.failure_reported {
            self.failure_reported = true;
            eprintln!(
                "WARN diagnostic_file_unavailable detail={}",
                bounded_text(&error.to_string(), 512)
            );
        }
    }

    fn write_record(&mut self, bytes: &[u8]) {
        if !self.file_failed {
            let result = (|| -> io::Result<()> {
                if self.file_len > 0
                    && self.file_len.saturating_add(bytes.len() as u64) > self.max_file_bytes
                {
                    self.rotate()?;
                }
                if let Some(file) = self.file.as_mut() {
                    file.write_all(bytes)?;
                    self.file_len = self.file_len.saturating_add(bytes.len() as u64);
                }
                Ok(())
            })();
            if let Err(error) = result {
                self.disable_file(&error);
            }
        }
        if self.mirror_stderr {
            let _ = io::stderr().lock().write_all(bytes);
        }
    }

    fn report_overflow(&mut self) {
        let Some(counter) = self.drop_counter.get() else {
            return;
        };
        let dropped = counter.dropped_lines();
        if dropped > self.reported_drops {
            let delta = dropped - self.reported_drops;
            self.reported_drops = dropped;
            self.write_record(
                format!("WARN diagnostic_queue_overflow dropped={delta} total={dropped}\n")
                    .as_bytes(),
            );
        }
    }
}

impl Write for RotatingSink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.report_overflow();
        self.write_record(bytes);
        self.written.fetch_add(1, Ordering::Release);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if let Some(file) = self.file.as_mut() {
            let _ = file.flush();
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use lab_core::{Command, InstrumentId, Query, QueryResult, Runtime, VirtualInstrumentConfig};
    use std::sync::{Condvar, Mutex, atomic::AtomicUsize};
    static NEXT: AtomicUsize = AtomicUsize::new(0);

    fn temporary_directory(label: &str) -> PathBuf {
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "lab-runtime-diagnostics-{}-{}-{id}",
            std::process::id(),
            label
        ))
    }

    fn test_config(directory: PathBuf) -> DiagnosticConfig {
        DiagnosticConfig {
            directory,
            level: Level::TRACE,
            max_file_bytes: 512,
            retained_file_count: 3,
            queue_records: 32,
            max_record_bytes: 256,
            mirror_stderr: false,
        }
    }

    fn creates_filters_caps_and_rotates_bounded_files() {
        let directory = temporary_directory("rotation");
        let (subscriber, diagnostics) = build(test_config(directory.clone())).unwrap();
        tracing::subscriber::with_default(subscriber, || {
            tracing::error!(event = "error_visible", "error entry");
            tracing::warn!(event = "warn_visible", "warn entry");
            tracing::info!(event = "info_visible", "info entry");
            tracing::debug!(event = "debug_visible", "debug entry");
            tracing::trace!(event = "trace_visible", payload = %"x".repeat(2000), "trace entry");
            for index in 0..40 {
                tracing::info!(event = "rotation", index, payload = %"y".repeat(80));
            }
        });
        assert!(diagnostics.finish(Duration::from_secs(2)));
        let files: Vec<_> = fs::read_dir(&directory).unwrap().collect();
        assert!(files.len() <= 3);
        let total: u64 = files
            .iter()
            .map(|entry| entry.as_ref().unwrap().metadata().unwrap().len())
            .sum();
        assert!(total <= 3 * 512);
        assert!(
            files
                .iter()
                .all(|entry| entry.as_ref().unwrap().metadata().unwrap().len() <= 512)
        );
        let text = files
            .iter()
            .map(|entry| fs::read_to_string(entry.as_ref().unwrap().path()).unwrap())
            .collect::<String>();
        assert!(text.contains("[truncated]") || text.contains("rotation"));
        fs::remove_dir_all(directory).unwrap();
    }

    fn unavailable_file_falls_back_without_failing_the_subscriber() {
        let root = temporary_directory("unavailable");
        fs::create_dir_all(&root).unwrap();
        let file = root.join("not-a-directory");
        File::create(&file).unwrap();
        let mut config = test_config(file);
        config.mirror_stderr = false;
        let (subscriber, diagnostics) = build(config).unwrap();
        tracing::subscriber::with_default(subscriber, || {
            tracing::warn!(event = "file_unavailable", "still observable");
        });
        assert!(diagnostics.finish(Duration::from_secs(2)));
        fs::remove_dir_all(root).unwrap();
    }

    fn level_filter_excludes_debug_and_trace_at_info() {
        let directory = temporary_directory("filter");
        let mut config = test_config(directory.clone());
        config.level = Level::INFO;
        let (subscriber, diagnostics) = build(config).unwrap();
        tracing::subscriber::with_default(subscriber, || {
            tracing::error!(event = "error_visible");
            tracing::warn!(event = "warn_visible");
            tracing::info!(event = "info_visible");
            tracing::debug!(event = "debug_hidden");
            tracing::trace!(event = "trace_hidden");
        });
        assert!(diagnostics.finish(Duration::from_secs(2)));
        let text = fs::read_dir(&directory)
            .unwrap()
            .map(|entry| fs::read_to_string(entry.unwrap().path()).unwrap())
            .collect::<String>();
        assert!(text.contains("error_visible"));
        assert!(text.contains("warn_visible"));
        assert!(text.contains("info_visible"));
        assert!(!text.contains("debug_hidden"));
        assert!(!text.contains("trace_hidden"));
        fs::remove_dir_all(directory).unwrap();
    }

    struct HeldWriter {
        state: Arc<(Mutex<(bool, bool)>, Condvar)>,
    }

    impl Write for HeldWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let (lock, changed) = &*self.state;
            let mut state = lock.lock().unwrap();
            state.1 = true;
            changed.notify_all();
            while state.0 {
                state = changed.wait(state).unwrap();
            }
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn full_diagnostic_queue_drops_records_without_blocking_runtime_progress() {
        let state = Arc::new((Mutex::new((true, false)), Condvar::new()));
        let (writer, guard) = tracing_appender::non_blocking::NonBlockingBuilder::default()
            .buffered_lines_limit(1)
            .lossy(true)
            .thread_name("held-test-diagnostics")
            .finish(HeldWriter {
                state: state.clone(),
            });
        let dropped = writer.error_counter();
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .with_writer(CappedMakeWriter {
                writer,
                accepting: Arc::new(AtomicBool::new(true)),
                attempted: Arc::new(AtomicU64::new(0)),
                max_record_bytes: 256,
            })
            .finish();

        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(event = "occupy_worker");
            let (lock, changed) = &*state;
            let mut held = lock.lock().unwrap();
            while !held.1 {
                held = changed.wait(held).unwrap();
            }
            drop(held);

            let mut runtime = Runtime::new();
            runtime
                .command(Command::RegisterVirtual(VirtualInstrumentConfig {
                    id: InstrumentId::new(900),
                    name: "diagnostic pressure oracle".into(),
                    history_capacity: 8,
                    base_temperature: 20.0,
                    measurement_enabled: true,
                }))
                .unwrap();
            for tick in 0..128 {
                tracing::info!(event = "pressure", tick);
                runtime
                    .command(Command::RefreshMeasurement {
                        instrument: InstrumentId::new(900),
                        parameter: lab_core::TEMPERATURE,
                        at: Duration::from_millis(tick),
                    })
                    .unwrap();
            }
            assert!(matches!(
                runtime
                    .query(Query::GetLatestSignal(lab_core::SignalId::new(
                        InstrumentId::new(900),
                        lab_core::TEMPERATURE,
                    )))
                    .unwrap(),
                QueryResult::Latest(Some(_))
            ));
        });
        assert!(dropped.dropped_lines() > 0);
        let (lock, changed) = &*state;
        lock.lock().unwrap().0 = false;
        changed.notify_all();
        drop(guard);
    }

    fn diagnostic_file_failure_does_not_change_recorder_lifecycle() {
        let root = temporary_directory("recorder-independence");
        fs::create_dir_all(&root).unwrap();
        let invalid_log_directory = root.join("ordinary-file");
        File::create(&invalid_log_directory).unwrap();
        let mut config = test_config(invalid_log_directory);
        config.mirror_stderr = false;
        let (subscriber, diagnostics) = build(config).unwrap();
        let database = root.join("experiment.sqlite");
        tracing::subscriber::with_default(subscriber, || {
            tracing::error!(event = "diagnostic_destination_failed");
            let mut recorder = crate::recorder::RecorderWorker::open(
                &database,
                crate::recorder::RecorderLimits::default(),
            )
            .unwrap();
            assert_eq!(recorder.poll().state, crate::recorder::RecordingState::Idle);
            recorder.request_finish().unwrap();
            let deadline = Instant::now() + Duration::from_secs(2);
            while recorder.poll().state != crate::recorder::RecordingState::Closed
                && Instant::now() < deadline
            {
                std::thread::yield_now();
            }
            assert_eq!(
                recorder.poll().state,
                crate::recorder::RecordingState::Closed
            );
        });
        assert!(diagnostics.finish(Duration::from_secs(2)));
        fs::remove_dir_all(root).unwrap();
    }

    fn safety_wording_never_equates_ack_or_readback_with_physical_effect() {
        let directory = temporary_directory("safety-wording");
        let mut config = test_config(directory.clone());
        config.max_file_bytes = 2048;
        config.max_record_bytes = 1024;
        let (subscriber, diagnostics) = build(config).unwrap();
        tracing::subscriber::with_default(subscriber, || {
            tracing::warn!(
                event = "physical_output_unconfirmed",
                outcome = "ambiguous",
                "physical output is unconfirmed; requested, sent, ACK, readback, and physical effect remain distinct"
            );
        });
        assert!(diagnostics.finish(Duration::from_secs(2)));
        let text = fs::read_to_string(directory.join(ACTIVE_FILE)).unwrap();
        assert!(text.contains("physical_output_unconfirmed"));
        assert!(text.contains("remain distinct"));
        assert!(!text.contains("physical success"));
        fs::remove_dir_all(directory).unwrap();
    }

    pub(crate) fn run_bounded_sink_contract() {
        creates_filters_caps_and_rotates_bounded_files();
        unavailable_file_falls_back_without_failing_the_subscriber();
        level_filter_excludes_debug_and_trace_at_info();
        full_diagnostic_queue_drops_records_without_blocking_runtime_progress();
        diagnostic_file_failure_does_not_change_recorder_lifecycle();
        safety_wording_never_equates_ack_or_readback_with_physical_effect();
    }
}
