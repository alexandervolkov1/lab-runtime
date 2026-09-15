//! Indexed raw pages freeze a durable watermark while new rows append.

use lab_core::{
    Command, InstrumentId, ParameterId, Runtime, Sample, SignalId, Unit, Value,
    VirtualInstrumentConfig, recording::RecordingFact,
};
use lab_runtime::recorder::{
    HistoryBudget, HistoryFilter, RecorderLimits, RecorderWorker, SqliteStore,
};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

fn temporary_database() -> PathBuf {
    let mut entropy = [0u8; 16];
    getrandom::fill(&mut entropy).unwrap();
    let suffix: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
    std::env::temp_dir().join(format!("lab-runtime-m7-history-{suffix}.sqlite"))
}

#[test]
fn keyset_pages_at_frozen_checkpoint_exclude_rows_appended_between_pages() {
    let path = temporary_database();
    let boot = "33333333333333333333333333333333";
    let instrument = InstrumentId::new(91);
    let mut runtime = Runtime::new();
    runtime
        .command(Command::RegisterVirtual(VirtualInstrumentConfig {
            id: instrument,
            name: "history fixture".into(),
            history_capacity: 1,
            base_temperature: 20.0,
            measurement_enabled: true,
        }))
        .unwrap();
    runtime.enable_recording_facts();
    let mut store = SqliteStore::open_with_boot(&path, boot).unwrap();
    store.start_run("history fixture").unwrap();
    for second in 1..=3 {
        runtime
            .command(Command::RefreshMeasurement {
                instrument,
                parameter: lab_core::TEMPERATURE,
                at: Duration::from_secs(second),
            })
            .unwrap();
        store.append_facts(&runtime.take_recording_facts()).unwrap();
    }
    let filter = HistoryFilter {
        boot_id: boot.to_owned(),
        run_no: 1,
        instrument,
        parameter: lab_core::TEMPERATURE,
        from: Duration::from_secs(1),
        to: Duration::from_secs(10),
    };
    let first = store.read_history_measurements(&filter, None, 2).unwrap();
    assert_eq!(first.rows.len(), 2);
    assert_eq!(first.watermark, 4);
    assert_eq!(first.rows[0].published_at, Duration::from_secs(1));
    assert_eq!(first.rows[1].published_at, Duration::from_secs(2));
    runtime
        .command(Command::RefreshMeasurement {
            instrument,
            parameter: lab_core::TEMPERATURE,
            at: Duration::from_secs(4),
        })
        .unwrap();
    store.append_facts(&runtime.take_recording_facts()).unwrap();
    let second = store
        .read_history_measurements(&filter, first.next_cursor.as_ref(), 2)
        .unwrap();
    assert_eq!(second.watermark, 4);
    assert_eq!(second.rows.len(), 1);
    assert_eq!(second.rows[0].published_at, Duration::from_secs(3));
    assert!(second.next_cursor.is_none());
    let fresh = store.read_history_measurements(&filter, None, 8).unwrap();
    assert_eq!(fresh.watermark, 5);
    assert_eq!(fresh.rows.len(), 4);
    drop(store);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn sqlite_history_budget_interrupts_without_returning_a_partial_page_or_cursor() {
    let path = temporary_database();
    let boot = "99999999999999999999999999999999";
    let instrument = InstrumentId::new(93);
    let mut runtime = Runtime::new();
    runtime
        .command(Command::RegisterVirtual(VirtualInstrumentConfig {
            id: instrument,
            name: "budget fixture".into(),
            history_capacity: 1,
            base_temperature: 20.0,
            measurement_enabled: true,
        }))
        .unwrap();
    runtime.enable_recording_facts();
    let mut store = SqliteStore::open_with_boot(&path, boot).unwrap();
    store.start_run("budget fixture").unwrap();
    runtime
        .command(Command::RefreshMeasurement {
            instrument,
            parameter: lab_core::TEMPERATURE,
            at: Duration::from_secs(1),
        })
        .unwrap();
    store.append_facts(&runtime.take_recording_facts()).unwrap();
    let filter = HistoryFilter {
        boot_id: boot.into(),
        run_no: 1,
        instrument,
        parameter: lab_core::TEMPERATURE,
        from: Duration::ZERO,
        to: Duration::from_secs(2),
    };
    let error = store
        .read_history_measurements_with_budget(
            &filter,
            None,
            1,
            HistoryBudget::for_testing(1, Duration::from_secs(1)),
        )
        .unwrap_err();
    assert_eq!(error.to_string(), "history_budget_exceeded");
    let page = store.read_history_measurements(&filter, None, 1).unwrap();
    assert_eq!(
        page.rows.len(),
        1,
        "interrupted query cannot change durable history"
    );
    drop(store);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn archived_history_sql_runs_only_on_storage_worker_and_retains_one_bounded_result() {
    let path = temporary_database();
    let archive_boot = "44444444444444444444444444444444";
    let instrument = InstrumentId::new(92);
    let mut runtime = Runtime::new();
    runtime
        .command(Command::RegisterVirtual(VirtualInstrumentConfig {
            id: instrument,
            name: "worker archive".into(),
            history_capacity: 1,
            base_temperature: 20.0,
            measurement_enabled: true,
        }))
        .unwrap();
    runtime.enable_recording_facts();
    let mut store = SqliteStore::open_with_boot(&path, archive_boot).unwrap();
    store.start_run("archive").unwrap();
    runtime
        .command(Command::RefreshMeasurement {
            instrument,
            parameter: lab_core::TEMPERATURE,
            at: Duration::from_secs(1),
        })
        .unwrap();
    store.append_facts(&runtime.take_recording_facts()).unwrap();
    store.stop_run().unwrap();
    drop(store);
    let mut worker = RecorderWorker::open_with_boot(
        &path,
        RecorderLimits::default(),
        "55555555555555555555555555555555",
    )
    .unwrap();
    let filter = HistoryFilter {
        boot_id: archive_boot.into(),
        run_no: 1,
        instrument,
        parameter: lab_core::TEMPERATURE,
        from: Duration::ZERO,
        to: Duration::from_secs(2),
    };
    let job = worker.request_history(filter, None, 8).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut result = None;
    while result.is_none() && Instant::now() < deadline {
        result = worker.try_take_history(job);
        std::thread::yield_now();
    }
    let page = result.unwrap().unwrap();
    assert_eq!(
        page.watermark, 3,
        "archived checkpoint includes the terminal interval seal"
    );
    assert_eq!(page.rows.len(), 1);
    assert!(worker.try_take_history(job).is_none());
    worker.request_finish().unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while worker.poll().state != lab_runtime::recorder::RecordingState::Closed
        && Instant::now() < deadline
    {
        std::thread::yield_now();
    }
    assert_eq!(
        worker.poll().state,
        lab_runtime::recorder::RecordingState::Closed
    );
    std::fs::remove_file(path).unwrap();
}

#[test]
fn run_discovery_pages_old_boots_and_freezes_upper_key_across_new_runs() {
    let path = temporary_database();
    let mut first = SqliteStore::open_with_boot(&path, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap();
    first.start_run("A1").unwrap();
    first.stop_run().unwrap();
    first.start_run("A2").unwrap();
    first.stop_run().unwrap();
    drop(first);
    let mut second =
        SqliteStore::open_with_boot(&path, "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").unwrap();
    second.start_run("B1").unwrap();
    second.stop_run().unwrap();
    let first_page = second.read_history_runs(None, 2).unwrap();
    assert_eq!(first_page.runs.len(), 2);
    assert_eq!(first_page.runs[0].label, "A1");
    assert_eq!(first_page.runs[1].label, "A2");
    second.start_run("B2").unwrap();
    second.stop_run().unwrap();
    let second_page = second
        .read_history_runs(first_page.next_cursor.as_ref(), 2)
        .unwrap();
    assert_eq!(second_page.runs.len(), 1);
    assert_eq!(second_page.runs[0].label, "B1");
    assert!(second_page.next_cursor.is_none());
    let fresh = second.read_history_runs(None, 8).unwrap();
    assert_eq!(fresh.runs.len(), 4);
    drop(second);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn escaped_equal_time_rows_shorten_pages_without_shifting_frozen_identities() {
    let path = temporary_database();
    let boot = "66666666666666666666666666666666";
    let instrument = InstrumentId::new(96);
    let parameter = ParameterId::new(19);
    let signal = SignalId::new(instrument, parameter);
    let at = Duration::from_secs(1);
    let long_text = "\0".repeat(1000);
    let mut store = SqliteStore::open_with_boot(&path, boot).unwrap();
    store.start_run("escaped equal time").unwrap();
    let facts: Vec<_> = (1..=3u64)
        .map(|sequence| RecordingFact::Measurement {
            sequence,
            sample: Sample::validated_good(
                signal,
                Unit::CELSIUS,
                at,
                Value::Text(long_text.clone()),
            )
            .unwrap(),
            generation: 1,
            revision: 1,
        })
        .collect();
    store.append_facts(&facts).unwrap();
    let filter = HistoryFilter {
        boot_id: boot.into(),
        run_no: 1,
        instrument,
        parameter,
        from: Duration::ZERO,
        to: Duration::from_secs(2),
    };
    let first = store.read_history_measurements(&filter, None, 128).unwrap();
    assert_eq!(
        first.rows.len(),
        1,
        "escaped JSON text must shorten the 8-KiB page"
    );
    let frozen = first.watermark;
    let mut ids = first
        .rows
        .iter()
        .map(|row| row.record_sequence)
        .collect::<Vec<_>>();
    store
        .append_facts(&[RecordingFact::Measurement {
            sequence: 4,
            sample: Sample::validated_good(
                signal,
                Unit::CELSIUS,
                at,
                Value::Text("later one".into()),
            )
            .unwrap(),
            generation: 1,
            revision: 1,
        }])
        .unwrap();
    let second = store
        .read_history_measurements(&filter, first.next_cursor.as_ref(), 128)
        .unwrap();
    assert_eq!(second.watermark, frozen);
    assert_eq!(second.rows.len(), 1);
    ids.extend(second.rows.iter().map(|row| row.record_sequence));
    store
        .append_facts(&[RecordingFact::Measurement {
            sequence: 5,
            sample: Sample::validated_good(
                signal,
                Unit::CELSIUS,
                at,
                Value::Text("later two".into()),
            )
            .unwrap(),
            generation: 1,
            revision: 1,
        }])
        .unwrap();
    let final_page = store
        .read_history_measurements(&filter, second.next_cursor.as_ref(), 128)
        .unwrap();
    assert_eq!(final_page.watermark, frozen);
    assert_eq!(final_page.rows.len(), 1);
    assert!(final_page.next_cursor.is_none());
    ids.extend(final_page.rows.iter().map(|row| row.record_sequence));
    assert_eq!(ids.len(), 3);
    assert_eq!(
        ids.iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        3
    );
    let mut cursor = None;
    let mut fresh_ids = Vec::new();
    for _ in 0..6 {
        let page = store
            .read_history_measurements(&filter, cursor.as_ref(), 128)
            .unwrap();
        fresh_ids.extend(page.rows.iter().map(|row| row.record_sequence));
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(fresh_ids.len(), 5);
    assert_eq!(fresh_ids[..3], ids);
    drop(store);
    std::fs::remove_file(path).unwrap();
}
