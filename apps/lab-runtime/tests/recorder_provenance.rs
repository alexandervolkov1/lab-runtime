//! Exact loaded bytes and their SHA-256 survive source-file changes and reopen.

use lab_runtime::{
    host::Clock,
    recorder::{ProvenanceEntry, RecordingState, SqliteStore},
    service::{ServiceHost, ServiceOptions},
};
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

fn temporary_database() -> PathBuf {
    let mut entropy = [0u8; 16];
    getrandom::fill(&mut entropy).unwrap();
    let suffix: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
    std::env::temp_dir().join(format!("lab-runtime-m7-provenance-{suffix}.sqlite"))
}

#[test]
fn serving_activation_records_exact_managed_sources_and_links_the_new_run() {
    let path = temporary_database();
    let path_text = path.to_string_lossy();
    let options = ServiceOptions::parse(&[
        "--serve",
        "--profile",
        "virtual-demo",
        "--port",
        "0",
        "--record-db",
        path_text.as_ref(),
    ])
    .unwrap();
    let mut service = ServiceHost::startup(options).unwrap();
    let clock = service.clock_copy();
    service
        .owner_mut()
        .start_recording("activation oracle", clock.now())
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while service.owner().recording_status().unwrap().state != RecordingState::Recording
        && Instant::now() < deadline
    {
        let clock = service.clock_copy();
        service.owner_mut().service(&clock).unwrap();
        std::thread::yield_now();
    }
    assert_eq!(
        service.owner().recording_status().unwrap().state,
        RecordingState::Recording
    );
    service.request_shutdown().unwrap();
    let shutdown_deadline = Instant::now() + Duration::from_secs(4);
    loop {
        if let Some(status) = service.shutdown_step().unwrap() {
            assert!(status.recorder_flushed, "{status:?}");
            break;
        }
        assert!(Instant::now() < shutdown_deadline);
        std::thread::yield_now();
    }
    drop(service);
    let db = rusqlite::Connection::open(&path).unwrap();
    for source in [
        lab_lua::fixtures::VIRTUAL_MODEL_SOURCE,
        lab_lua::fixtures::MOVING_MEAN_SOURCE,
    ] {
        let hash = Sha256::digest(source.as_bytes());
        let exact: Vec<u8> = db
            .query_row(
                "SELECT content FROM provenance_content \
            WHERE kind='managed_lua_source' AND content_hash=?1",
                [hash.as_slice()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(exact, source.as_bytes());
    }
    let (root, activation): (Vec<u8>, Vec<u8>) = db
        .query_row(
            "SELECT manifest_root_hash,activation_no FROM configurations LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    let linked: Vec<u8> = db
        .query_row(
            "SELECT initial_activation_id FROM runs \
        WHERE label='activation oracle'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(linked, activation);
    let root_content: Vec<u8> = db
        .query_row(
            "SELECT content FROM provenance_content WHERE content_hash=?1",
            [root.as_slice()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(Sha256::digest(&root_content).to_vec(), root);
    let object_count: i64 = db
        .query_row(
            "SELECT count(*) FROM object_snapshots \
        WHERE boot_id=(SELECT boot_id FROM runs WHERE label='activation oracle')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        object_count >= 5,
        "committed activation must freeze actual object baseline"
    );
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn provenance_commits_loaded_script_bytes_not_later_path_contents_and_deduplicates() {
    let path = temporary_database();
    let source_path = path.with_extension("lua");
    let loaded = b"return 21.5\n".to_vec();
    std::fs::write(&source_path, &loaded).unwrap();
    let captured = std::fs::read(&source_path).unwrap();
    std::fs::write(&source_path, b"return 99.0\n").unwrap();
    let entry = ProvenanceEntry {
        kind: "managed_lua_source".into(),
        encoding: "utf8".into(),
        content: captured,
    };
    let expected = Sha256::digest(&loaded);
    let mut store = SqliteStore::open(&path).unwrap();
    let single_root = store
        .commit_provenance(std::slice::from_ref(&entry))
        .unwrap();
    let root = store.commit_provenance(&[entry.clone(), entry]).unwrap();
    assert_eq!(
        root, single_root,
        "duplicate loaded content cannot change the manifest root"
    );
    assert_ne!(root, [0u8; 32]);
    drop(store);
    let connection = rusqlite::Connection::open(&path).unwrap();
    let rows: i64 = connection
        .query_row("SELECT COUNT(*) FROM provenance_content", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(
        rows, 2,
        "activation root JSON is itself immutable stored content"
    );
    let (hash, content): (Vec<u8>, Vec<u8>) = connection
        .query_row(
            "SELECT content_hash,content FROM provenance_content WHERE kind='managed_lua_source'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(hash, expected.to_vec());
    assert_eq!(content, loaded);
    let manifest: Vec<u8> = connection
        .query_row(
            "SELECT content FROM provenance_content WHERE kind='activation_manifest'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let canonical: serde_json::Value = serde_json::from_slice(&manifest).unwrap();
    assert_eq!(canonical["encoding_version"], 1);
    assert_eq!(canonical["entries"].as_array().unwrap().len(), 1);
    drop(connection);
    std::fs::remove_file(path).unwrap();
    std::fs::remove_file(source_path).unwrap();
}
