//! Recorded property commits through the real binary and one Application session.

mod support;

use lab_runtime::wire::encode_frame;
use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::TcpStream,
    path::Path,
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

const TIMEOUT: Duration = Duration::from_secs(5);

struct OwnedProcess(Child);
impl Drop for OwnedProcess {
    fn drop(&mut self) {
        // Only this test's process may be terminated, including assertion failures.
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Client {
    socket: BufReader<TcpStream>,
    scope: String,
    call: u64,
}

impl Client {
    fn receive(&mut self) -> Value {
        let mut frame = Vec::new();
        let count = self
            .socket
            .by_ref()
            .take(16_385)
            .read_until(b'\n', &mut frame)
            .unwrap();
        assert!(count > 0 && count <= 16_384 && frame.last() == Some(&b'\n'));
        serde_json::from_slice(&frame).unwrap()
    }

    fn request(&mut self, op: &str, args: Value, sequence: Option<u64>) -> Vec<Value> {
        self.call += 1;
        let id = self.call.to_string();
        let mut request = json!({"v":1,"msg_id":id,"op":op,"args":args});
        if let Some(sequence) = sequence {
            request["request_id"] = json!({"scope":self.scope,"seq":sequence.to_string()});
        }
        self.socket
            .get_mut()
            .write_all(&encode_frame(&request).unwrap())
            .unwrap();
        let deadline = Instant::now() + TIMEOUT;
        let mut replies = Vec::new();
        for _ in 0..16 {
            assert!(Instant::now() < deadline, "{op} terminal reply deadline");
            self.socket
                .get_mut()
                .set_read_timeout(Some(deadline.saturating_duration_since(Instant::now())))
                .unwrap();
            let reply = self.receive();
            assert_eq!(reply["msg_id"], id);
            if let Some(sequence) = sequence {
                assert_eq!(reply["request_id"]["scope"], self.scope);
                assert_eq!(reply["request_id"]["seq"], sequence.to_string());
            }
            let terminal = reply["state"] != "accepted";
            replies.push(reply);
            if terminal {
                return replies;
            }
        }
        panic!("{op} reply bound");
    }

    fn query(&mut self, op: &str, args: Value) -> Value {
        let replies = self.request(op, args, None);
        assert_eq!(replies.len(), 1);
        assert_eq!(replies[0]["type"], "result", "{replies:?}");
        replies[0]["result"].clone()
    }

    fn mutate(&mut self, op: &str, args: Value, sequence: u64) -> Value {
        let replies = self.request(op, args, Some(sequence));
        assert_eq!(replies.len(), 2, "{replies:?}");
        assert_eq!(replies[0]["state"], "accepted");
        replies[1].clone()
    }
}

fn property_args(property: &Value, value: Value, revision: u64) -> Value {
    json!({"target":property["owner"],"property":property["property"],
        "value":value,"expected_revision":revision.to_string()})
}

fn archive(path: &Path) -> Connection {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    connection.busy_timeout(TIMEOUT).unwrap();
    connection
}

struct ArchivedLifecycle {
    sequence: Vec<u8>,
    run: Option<Vec<u8>>,
    interval: Option<Vec<u8>>,
    payload: Vec<u8>,
}

fn assert_durable_property(
    path: &Path,
    revision: u64,
    status: &Value,
    recording: bool,
    toml_hash: &str,
) {
    assert_eq!(status["provenance"]["generation"], revision.to_string());
    assert_eq!(
        status["state"],
        if recording { "recording" } else { "idle" }
    );
    let db = archive(path);
    let ArchivedLifecycle {
        sequence,
        run,
        interval,
        payload,
    } = db
        .query_row(
            "SELECT record_seq,run_no,interval_no,payload FROM records
             WHERE kind='configuration_lifecycle'
             AND json_extract(CAST(payload AS TEXT),'$.activation_generation')=?1",
            [revision.to_string()],
            |row| {
                Ok(ArchivedLifecycle {
                    sequence: row.get(0)?,
                    run: row.get(1)?,
                    interval: row.get(2)?,
                    payload: row.get(3)?,
                })
            },
        )
        .unwrap();
    let payload: Value = serde_json::from_slice(&payload).unwrap();
    assert_eq!(payload["operation_kind"], "property_configure");
    assert_eq!(payload["base_revision"], (revision - 1).to_string());
    assert_eq!(payload["committed_revision"], revision.to_string());
    assert_eq!(payload["activation_generation"], revision.to_string());
    assert_eq!(payload["activation_root"], status["provenance"]["identity"]);
    assert_eq!(payload["runtime_toml_sha256"], toml_hash);
    assert_eq!(run.is_some(), recording);
    assert_eq!(interval.is_some(), recording);
    let lifecycle_sequence = u64::from_be_bytes(sequence.clone().try_into().unwrap());
    let receipted_sequence = status["persisted_through_seq"]
        .as_str()
        .unwrap()
        .parse::<u64>()
        .unwrap();
    assert!(
        receipted_sequence >= lifecycle_sequence,
        "the live projection after Completed must already receipt this lifecycle"
    );
    let checkpoint: Vec<u8> = db
        .query_row(
            "SELECT persisted_through_seq FROM durable_checkpoints",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        checkpoint >= sequence,
        "Completed requires a committed lifecycle prefix"
    );
    let generation = revision.to_be_bytes();
    let root: Vec<u8> = db
        .query_row(
            "SELECT manifest_root_hash FROM configurations WHERE activation_no=?1",
            [generation.as_slice()],
            |r| r.get(0),
        )
        .unwrap();
    let root_hex: String = root.iter().map(|byte| format!("{byte:02x}")).collect();
    assert_eq!(root_hex, status["provenance"]["identity"].as_str().unwrap());
}

#[test]
fn real_process_properties_commit_durably_in_idle_and_recording_without_replay() {
    let directory = support::temporary_database("property-process").with_extension("");
    fs::create_dir(&directory).unwrap();
    let config = directory.join("runtime.virtual.toml");
    let database = directory.join("history.sqlite");
    // Keep the shipped virtual instruments and Required Recorder; isolate only the port.
    let source =
        include_str!("../../../examples/runtime.virtual.toml").replace("port = 7420", "port = 0");
    fs::write(&config, &source).unwrap();
    let toml_hash: String = Sha256::digest(source.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let mut process = OwnedProcess(
        Command::new(env!("CARGO_BIN_EXE_lab-runtime"))
            .args(["--serve", "--config"])
            .arg(&config)
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    let output = process.0.stdout.take().unwrap();
    let (tx, rx) = mpsc::sync_channel(1);
    let reader = thread::spawn(move || {
        let mut line = String::new();
        let read = BufReader::new(output).take(16_385).read_line(&mut line);
        let _ = tx.send((read, line));
    });
    let (read, line) = rx
        .recv_timeout(TIMEOUT)
        .expect("Runtime readiness deadline");
    assert!(read.unwrap() > 0);
    let ready: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(ready["state"], "ready");
    reader.join().unwrap();
    // One TCP connection and one scope for queries, all mutations and shutdown.
    let address =
        std::net::SocketAddr::from(([127, 0, 0, 1], ready["port"].as_u64().unwrap() as u16));
    let socket = TcpStream::connect_timeout(&address, TIMEOUT).unwrap();
    socket.set_read_timeout(Some(TIMEOUT)).unwrap();
    socket.set_write_timeout(Some(TIMEOUT)).unwrap();
    let mut client = Client {
        socket: BufReader::new(socket),
        scope: String::new(),
        call: 0,
    };
    let hello = client.query("hello", json!({"scope":null}));
    client.scope = hello["scope"].as_str().unwrap().into();
    assert!(client.scope.ends_with(":1"));
    let properties = client.query("configuration_properties", json!({}));
    assert_eq!(properties["complete"], true);
    let writable = |kind| {
        properties["records"]
            .as_array()
            .unwrap()
            .iter()
            .find(|record| record["access"] == "read_write" && record["value_type"] == kind)
            .unwrap()
            .clone()
    };
    let integer = writable("integer");
    let text = writable("text");
    assert_eq!(integer["property"], "poll_period_ms");
    assert_eq!(text["property"], "display_name");
    let initial_status = client.query("recording_status", json!({}));
    assert_eq!(initial_status["policy"], "required");
    assert_eq!(initial_status["state"], "idle");
    assert_eq!(initial_status["provenance"]["generation"], "1");
    let mut receipts = Vec::new();

    for (seq, property, value, base, recording) in [
        (1, &integer, json!(200), 1, false),
        (2, &text, json!("Idle virtual plant"), 2, false),
        (6, &integer, json!(250), 3, true),
        (7, &text, json!("Recording virtual plant"), 4, true),
    ] {
        if seq == 6 {
            let stale = client.mutate(
                "property_configure",
                property_args(&integer, json!(300), 1),
                3,
            );
            assert_eq!(stale["state"], "failed");
            assert_eq!(stale["code"], "revision_conflict");
            let mut invalid = property_args(&integer, json!(300), 3);
            invalid["property"] = json!("not_a_property");
            let invalid = client.mutate("property_configure", invalid, 4);
            assert_eq!(invalid["state"], "failed");
            assert_eq!(invalid["code"], "invalid_configuration");
            let replay = client.request(
                "property_configure",
                property_args(&integer, json!(200), 1),
                Some(1),
            );
            assert_eq!(replay.len(), 1);
            assert_eq!(replay[0]["state"], "completed");
            assert_eq!(replay[0]["result"]["revision"], "2");
            let current = client.query("configuration_status", json!({}));
            assert_eq!(current["revision"], "3");
            assert_eq!(current["runtime_overrides"], 2);
            let unchanged = client.query("configuration_properties", json!({}));
            assert_eq!(
                unchanged["records"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|record| record["id"] == integer["id"])
                    .unwrap()["current"],
                200
            );
            assert_eq!(
                client.query("recording_status", json!({}))["provenance"]["generation"],
                "3"
            );
            let start = client.mutate(
                "recording_start",
                json!({"label":"Recorded property lifecycle"}),
                5,
            );
            assert_eq!(start["state"], "completed", "{start:?}");
        }
        let completed = client.mutate(
            "property_configure",
            property_args(property, value.clone(), base),
            seq,
        );
        assert_eq!(completed["state"], "completed", "{completed:?}");
        assert_eq!(completed["result"]["revision"], (base + 1).to_string());
        assert_eq!(completed["result"]["persisted_to_deployment_source"], false);
        let status = client.query("recording_status", json!({}));
        assert_eq!(status["provenance"]["generation"], (base + 1).to_string());
        assert_eq!(
            status["accepting_facts"], recording,
            "Required admission follows the durable activation receipt"
        );
        assert_eq!(
            status["state"],
            if recording { "recording" } else { "idle" }
        );
        receipts.push((base + 1, status, recording));
        let current = client.query("configuration_properties", json!({}));
        let updated = current["records"]
            .as_array()
            .unwrap()
            .iter()
            .find(|record| record["id"] == property["id"])
            .unwrap();
        assert_eq!(updated["current"], value);
        assert_eq!(updated["revision"], (base + 1).to_string());
        assert_eq!(updated["unit"], property["unit"]);
    }
    let active = client.query("recording_status", json!({}));
    let stopped = client.mutate("recording_stop", json!({"run_id":active["active_run"]}), 8);
    assert_eq!(stopped["state"], "completed", "{stopped:?}");
    let terminal = client.query(
        "operation_status",
        json!({"request_id":{"scope":client.scope,"seq":"6"}}),
    );
    assert_eq!(terminal["state"], "completed");
    let shutdown = client.mutate("runtime_shutdown", json!({}), 9);
    assert_eq!(shutdown["state"], "completed", "{shutdown:?}");
    assert_eq!(shutdown["result"]["recorder_flushed"], true);
    let exit = support::wait_until(
        Instant::now() + TIMEOUT,
        "owned Runtime exit deadline",
        || process.0.try_wait().unwrap(),
    );
    assert!(exit.success());
    // SQL inspection follows the exclusive writer's graceful shutdown. The
    // live projections above already required its post-COMMIT generation receipts.
    for (revision, status, recording) in receipts {
        assert_durable_property(&database, revision, &status, recording, &toml_hash);
    }
    let db = archive(&database);
    let count: i64 = db
        .query_row(
            "SELECT count(*) FROM records WHERE kind='configuration_lifecycle'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        count, 4,
        "rejected and replayed requests must not apply again"
    );
    let activations: i64 = db
        .query_row("SELECT count(*) FROM configurations", [], |r| r.get(0))
        .unwrap();
    assert_eq!(activations, 5);
    let overlays: i64 = db
        .query_row(
            "SELECT count(*) FROM provenance_content WHERE kind='runtime_configuration_overlay'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(overlays, 4);
    let frozen: Vec<u8> = db
        .query_row(
            "SELECT content FROM provenance_content WHERE kind='runtime_toml'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(frozen, source.as_bytes());
    assert_eq!(
        fs::read(&config).unwrap(),
        source.as_bytes(),
        "overlays never persist to deployment source"
    );
    let integrity: String = db
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(integrity, "ok");
    assert!(
        !db.prepare("PRAGMA foreign_key_check")
            .unwrap()
            .exists([])
            .unwrap()
    );
    let sequences: Vec<u64> = db
        .prepare("SELECT record_seq FROM records ORDER BY record_seq")
        .unwrap()
        .query_map([], |r| r.get::<_, Vec<u8>>(0))
        .unwrap()
        .map(|value| u64::from_be_bytes(value.unwrap().try_into().unwrap()))
        .collect();
    assert!(sequences.windows(2).all(|pair| pair[1] == pair[0] + 1));
    // Preserve this test-only archive for inspection; never clean other workspaces.
    eprintln!(
        "Recorded property acceptance archive: {}",
        database.display()
    );
}
