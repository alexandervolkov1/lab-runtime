//! Finite M1 demonstration by default; explicit serve mode runs the M6 host.
use lab_core::{Command, InstrumentId, Query, QueryResult, Runtime, VirtualInstrumentConfig};
use lab_runtime::{
    server,
    service::{ServiceHost, ServiceOptions},
};
use std::{
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !args.is_empty() {
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let options = ServiceOptions::parse(&refs)?;
        let host = ServiceHost::startup(options)?;
        println!("{}", host.ready_line());
        server::run(host, Arc::new(AtomicBool::new(false)))?;
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
