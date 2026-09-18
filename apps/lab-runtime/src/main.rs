//! Finite M1 demonstration by default; explicit serve mode runs the M6 host.
use lab_core::{Command, InstrumentId, Query, QueryResult, Runtime, VirtualInstrumentConfig};
use lab_runtime::{
    diagnostics::{self, Diagnostics},
    protocol, server,
    service::{ServiceHost, ServiceOptions},
};
use std::{
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let diagnostics = match Diagnostics::install() {
        Ok(diagnostics) => Some(diagnostics),
        Err(error) => {
            eprintln!("WARN diagnostic_subscriber_unavailable detail={error}");
            None
        }
    };
    let log_directory = diagnostics
        .as_ref()
        .map(|diagnostics| diagnostics.directory().to_path_buf())
        .unwrap_or_else(diagnostics::default_log_directory);
    let log_level = diagnostics
        .as_ref()
        .map(Diagnostics::level)
        .unwrap_or(tracing::Level::INFO);
    tracing::info!(
        event = "process_start",
        package = env!("CARGO_PKG_NAME"),
        version = env!("CARGO_PKG_VERSION"),
        protocol = protocol::PROTOCOL_ID,
        protocol_version = protocol::PROTOCOL_VERSION,
        application_api_version = protocol::APPLICATION_API_VERSION,
        log_directory = %log_directory.display(),
        log_level = %log_level,
        max_file_bytes = diagnostics::MAX_FILE_BYTES,
        retained_files = diagnostics::RETAINED_FILE_COUNT,
        queue_records = diagnostics::QUEUE_RECORDS,
        "lab-runtime process starting"
    );
    let result = run();
    match &result {
        Ok(()) => tracing::info!(
            event = "process_exit",
            "lab-runtime process stopped cleanly"
        ),
        Err(error) => tracing::error!(
            event = "process_exit_failed",
            detail = %error,
            "lab-runtime process failed"
        ),
    }
    if let Some(diagnostics) = diagnostics {
        let _ = diagnostics.finish(Duration::from_millis(250));
    }
    result
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !args.is_empty() {
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let options = ServiceOptions::parse(&refs)?;
        tracing::info!(
            event = "service_configuration_selected",
            deployment = options.configuration_path().is_some(),
            "service configuration accepted"
        );
        let stop = Arc::new(AtomicBool::new(false));
        let callback_flag = stop.clone();
        // The OS callback only publishes intent; the Runtime owner performs
        // every safe shutdown step and retains evidence before closing peers.
        ctrlc::set_handler(move || {
            callback_flag.store(true, std::sync::atomic::Ordering::Release)
        })?;
        let host = ServiceHost::startup(options)?;
        tracing::info!(
            event = "service_ready",
            address = %host.bound_address(),
            boot_id = host.boot_id(),
            "Runtime service ready"
        );
        println!("{}", host.ready_line());
        server::run(host, stop)?;
        return Ok(());
    }
    finite_demo()
}

fn finite_demo() -> Result<(), Box<dyn std::error::Error>> {
    let mut runtime = Runtime::new();
    runtime.command(Command::RegisterVirtual(VirtualInstrumentConfig {
        id: InstrumentId::new(1),
        name: "Virtual temperature".into(),
        history_capacity: 3,
        base_temperature: 20.0,
        measurement_enabled: true,
    }))?;

    println!("Milestone 1: virtual-only; no output execution");
    let QueryResult::Instruments(instruments) = runtime.query(Query::Discover)? else {
        return Err("unexpected discovery result".into());
    };
    // Presentation and measurement selection use descriptors, not concrete model names.
    for instrument in instruments {
        println!("Instrument {}: {}", instrument.id.get(), instrument.name);
        for parameter in &instrument.parameters {
            println!(
                "  {}: {:?} {} / {:?} / {:?} / {:?}",
                parameter.name,
                parameter.value_spec.value_type(),
                parameter.unit.symbol(),
                parameter.role,
                parameter.access,
                parameter.write_effect
            );
            let Some(signal) = parameter.signal else {
                continue;
            };
            if let QueryResult::Latest(None) = runtime.query(Query::GetLatestSignal(signal))? {
                println!("    latest: unknown");
            }
            for seconds in 0..5 {
                runtime.command(Command::RefreshMeasurement {
                    instrument: instrument.id,
                    parameter: parameter.id,
                    at: Duration::from_secs(seconds),
                })?;
                if let QueryResult::Latest(Some(sample)) =
                    runtime.query(Query::GetLatestSignal(signal))?
                {
                    println!(
                        "    t={:?} {:?} {:?} {}",
                        sample.at(),
                        sample.quality(),
                        sample.value(),
                        sample.unit().symbol()
                    );
                }
            }
            if let QueryResult::Window(samples) = runtime.query(Query::GetSignalWindow(signal))?
                && let (Some(first), Some(last)) = (samples.first(), samples.last())
            {
                println!(
                    "    window: {} samples, oldest={:?}, latest={:?}",
                    samples.len(),
                    first.at(),
                    last.at()
                );
            }
        }
    }
    Ok(())
}
