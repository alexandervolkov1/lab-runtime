//! Native Workbench client with one bounded Application worker and client-owned GUI.

mod client;
mod gui;
mod model;
mod ownership;
mod presentation;
mod recovery;
mod storage;

use presentation::default_presentation_path;
use std::{net::SocketAddr, path::PathBuf, process::ExitCode};

fn usage() {
    eprintln!("usage: lab-workbench --connect 127.0.0.1:PORT [--scope SCOPE] [--workspace PATH]");
}

struct Arguments {
    address: SocketAddr,
    scope: Option<String>,
    workspace: Option<PathBuf>,
}

fn arguments() -> Result<Arguments, &'static str> {
    let mut args = std::env::args().skip(1);
    let mut address = None;
    let mut scope = None;
    let mut workspace = None;
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--connect" if address.is_none() => {
                address = Some(
                    args.next()
                        .ok_or("--connect needs an address")?
                        .parse()
                        .map_err(|_| "--connect needs a numeric socket address")?,
                );
            }
            "--scope" if scope.is_none() => {
                scope = Some(args.next().ok_or("--scope needs a value")?);
            }
            "--workspace" if workspace.is_none() => {
                workspace = Some(PathBuf::from(
                    args.next().ok_or("--workspace needs a path")?,
                ));
            }
            _ => return Err("unknown or duplicate argument"),
        }
    }
    Ok(Arguments {
        address: address.ok_or("--connect is required")?,
        scope,
        workspace,
    })
}

fn run() -> Result<(), String> {
    let arguments = arguments().map_err(str::to_owned)?;
    let workspace = match arguments.workspace {
        Some(path) => path,
        None => default_presentation_path()
            .map_err(|error| error.to_string())?
            .parent()
            .expect("default presentation has a workspace parent")
            .to_owned(),
    };
    gui::run(gui::GuiLaunch {
        address: arguments.address,
        scope: arguments.scope,
        workspace,
    })
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            usage();
            eprintln!("lab-workbench: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod operator_boundary_tests {
    #[test]
    fn ordinary_gui_source_has_no_raw_or_prohibited_application_mutations() {
        let source = include_str!("gui/app.rs");
        assert!(!source.contains(".mutation("));
        assert!(!source.contains(".retry_mutation("));
        for prohibited in [
            "runtime_shutdown",
            "emulator_publish",
            "virtual_models_restart",
            "stage_configuration",
            "apply_configuration",
            "reload_configuration",
            "history_read",
        ] {
            assert!(
                !source.contains(prohibited),
                "ordinary GUI contains prohibited operation {prohibited}"
            );
        }
    }
}

#[cfg(test)]
mod runtime_acceptance {
    use super::{
        client::{
            ClientHandle, ClientUpdate,
            types::{ConnectionState, EventCursor, MutationIdentity, ReplyKind},
        },
        gui::rebuild::RebuildCoordinator,
        model::{
            ExactRetryWorkflow, Freshness, OperatorIntent, OperatorWorkflow, OperatorWorkflowState,
            RecoveryStatusTracker, WorkbenchModel,
        },
        presentation::{PresentationDocument, RuntimeRef},
        recovery::load_journal,
    };
    use serde_json::{Value, json};
    use std::{
        env, fs,
        io::{BufRead, BufReader, Read, Write},
        net::{SocketAddr, TcpListener},
        path::PathBuf,
        process::{Child, Command, ExitStatus, Stdio},
        sync::mpsc,
        thread,
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

    const ACCEPTANCE_TIMEOUT: Duration = Duration::from_secs(5);

    fn wait_for_exit(child: &mut Child, label: &str) -> ExitStatus {
        let deadline = Instant::now() + ACCEPTANCE_TIMEOUT;
        loop {
            if let Some(status) = child.try_wait().expect("query child status") {
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "{label} did not exit before deadline"
            );
            thread::yield_now();
        }
    }

    struct RuntimeChild(Child, Option<PathBuf>);

    impl Drop for RuntimeChild {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let deadline = Instant::now() + ACCEPTANCE_TIMEOUT;
            while Instant::now() < deadline {
                if self.0.try_wait().ok().flatten().is_some() {
                    break;
                }
                thread::yield_now();
            }
            if let Some(path) = self.1.as_ref() {
                let _ = std::fs::remove_file(path);
                let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
                let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
            }
        }
    }

    impl RuntimeChild {
        fn terminate(&mut self) -> ExitStatus {
            self.0.kill().expect("terminate Runtime process");
            wait_for_exit(&mut self.0, "Runtime")
        }
    }

    struct AcceptanceChild(Child);

    impl AcceptanceChild {
        fn terminate(&mut self) -> ExitStatus {
            self.0.kill().expect("terminate Workbench test helper");
            wait_for_exit(&mut self.0, "Workbench test helper")
        }
    }

    impl Drop for AcceptanceChild {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let deadline = Instant::now() + ACCEPTANCE_TIMEOUT;
            while Instant::now() < deadline {
                if self.0.try_wait().ok().flatten().is_some() {
                    break;
                }
                thread::yield_now();
            }
        }
    }

    fn runtime_binary() -> PathBuf {
        if let Some(path) = std::env::var_os("LAB_RUNTIME_BIN") {
            return path.into();
        }
        let executable = std::env::current_exe().expect("test executable path");
        let profile = executable
            .parent()
            .and_then(std::path::Path::parent)
            .expect("target profile directory");
        profile.join(if cfg!(windows) {
            "lab-runtime.exe"
        } else {
            "lab-runtime"
        })
    }

    fn start_runtime() -> (RuntimeChild, std::net::SocketAddr) {
        start_runtime_with_args(
            &["--serve", "--profile", "virtual-demo", "--port", "0"],
            None,
        )
    }

    fn start_runtime_on(port: u16) -> (RuntimeChild, SocketAddr) {
        let port = port.to_string();
        start_runtime_with_args(
            &["--serve", "--profile", "virtual-demo", "--port", &port],
            None,
        )
    }

    fn unused_loopback_address() -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").expect("reserve loopback port");
        let address = listener.local_addr().expect("reserved loopback address");
        drop(listener);
        address
    }

    fn start_runtime_with_recorder() -> (RuntimeChild, std::net::SocketAddr) {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let database = std::env::temp_dir().join(format!(
            "lab-workbench-m14-6b2b1-{}-{suffix}.sqlite",
            std::process::id()
        ));
        let database_text = database
            .to_str()
            .expect("temporary database path")
            .to_owned();
        start_runtime_with_args(
            &[
                "--serve",
                "--profile",
                "virtual-demo",
                "--port",
                "0",
                "--record-db",
                database_text.as_str(),
                "--record-policy",
                "best-effort",
            ],
            Some(database),
        )
    }

    fn start_runtime_with_args(
        arguments: &[&str],
        recorder_path: Option<PathBuf>,
    ) -> (RuntimeChild, std::net::SocketAddr) {
        let binary = runtime_binary();
        assert!(
            binary.is_file(),
            "build lab-runtime before the process acceptance: {}",
            binary.display()
        );
        let mut child = Command::new(binary)
            .args(arguments)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("start lab-runtime");
        let stdout = child.stdout.take().expect("runtime stdout");
        let (sender, receiver) = mpsc::sync_channel(1);
        thread::spawn(move || {
            let mut line = String::new();
            let result = BufReader::new(stdout).read_line(&mut line);
            let _ = sender.send((result, line));
        });
        let (read, line) = receiver
            .recv_timeout(Duration::from_secs(3))
            .expect("Runtime readiness deadline");
        assert_ne!(read.expect("Runtime readiness read"), 0);
        let ready: Value = serde_json::from_str(&line).expect("Runtime readiness JSON");
        let port = ready["port"].as_u64().expect("Runtime readiness TCP port") as u16;
        (
            RuntimeChild(child, recorder_path),
            std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        )
    }

    fn wait_for(client: &ClientHandle, predicate: impl Fn(&ClientUpdate) -> bool) -> ClientUpdate {
        let deadline = Instant::now() + ACCEPTANCE_TIMEOUT;
        loop {
            let update = client
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("Workbench client update before acceptance deadline");
            if predicate(&update) {
                return update;
            }
        }
    }

    fn reply_result(update: ClientUpdate) -> Value {
        let ClientUpdate::Reply { envelope, .. } = update else {
            panic!("expected Application reply")
        };
        envelope["result"].clone()
    }

    fn read_request(reader: &mut impl BufRead) -> Value {
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .expect("read Application request");
        assert!(!line.is_empty(), "Application peer closed before request");
        serde_json::from_str(&line).expect("Application request JSON")
    }

    fn write_value(writer: &mut impl Write, value: &Value) {
        serde_json::to_writer(&mut *writer, value).expect("encode Application reply");
        writer.write_all(b"\n").expect("write Application reply");
        writer.flush().expect("flush Application reply");
    }

    fn apply_until(
        client: &ClientHandle,
        model: &mut WorkbenchModel,
        rebuild: &mut RebuildCoordinator,
        mut predicate: impl FnMut(&ClientUpdate, &WorkbenchModel) -> bool,
    ) -> ClientUpdate {
        let deadline = Instant::now() + ACCEPTANCE_TIMEOUT;
        loop {
            let update = client
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("Workbench update before acceptance deadline");
            model.apply_client_update(update.clone());
            rebuild.after_update(&update, model, client);
            if predicate(&update, model) {
                return update;
            }
        }
    }

    #[test]
    #[ignore = "M14.5 operator process acceptance; run after cargo build -p lab-runtime -p lab-workbench"]
    fn real_runtime_operator_layer_confirms_completes_refreshes_and_never_retries_conflict() {
        let (mut runtime, address) = start_runtime();
        let client = ClientHandle::spawn(address).unwrap();
        let mut model = WorkbenchModel::new(PresentationDocument::empty("m14-5-acceptance"));
        let mut rebuild = RebuildCoordinator::default();
        client.connect(None).unwrap();
        apply_until(&client, &mut model, &mut rebuild, |_, model| {
            model.observations.freshness == Freshness::Fresh
        });

        let target = RuntimeRef::Reference {
            reference: "1".into(),
        };
        let initial = model.observations.entities[&target].value.clone();
        let original_target = initial["target"].as_f64().unwrap();
        let mut workflow = OperatorWorkflow::default();
        workflow
            .begin(
                &model,
                OperatorIntent::RetuneReference {
                    reference: "1".into(),
                    target: original_target + 0.5,
                    rate: 2.0,
                },
            )
            .unwrap();
        assert!(matches!(
            workflow.state,
            OperatorWorkflowState::AwaitingConfirmation(_)
        ));
        let command_id = workflow.confirm(&model, &client).unwrap();
        model
            .track_operator_intent(command_id)
            .expect("single operator workflow retains bounded action state");
        let deadline = Instant::now() + ACCEPTANCE_TIMEOUT;
        while Instant::now() < deadline {
            let update = client
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap();
            model.apply_client_update(update.clone());
            workflow.after_update(&update);
            rebuild.after_update(&update, &mut model, &client);
            let authoritative = model
                .observations
                .entities
                .get(&target)
                .is_some_and(|item| {
                    item.freshness == Freshness::Fresh
                        && item.value["revision"] != initial["revision"]
                        && item.value["target"] == json!(original_target + 0.5)
                });
            if matches!(workflow.state, OperatorWorkflowState::Completed { .. }) && authoritative {
                break;
            }
        }
        assert!(workflow.accepted_observed);
        assert!(matches!(
            workflow.state,
            OperatorWorkflowState::Completed { .. }
        ));
        assert_eq!(
            model.observations.entities[&target].value["target"],
            json!(original_target + 0.5)
        );

        // Freeze an old draft, commit a second exact intent without applying its
        // Reference event locally, then submit the old draft. Runtime, not the GUI,
        // returns the optimistic-concurrency conflict.
        let mut stale_draft = OperatorWorkflow::default();
        stale_draft
            .begin(
                &model,
                OperatorIntent::RetuneReference {
                    reference: "1".into(),
                    target: original_target + 0.75,
                    rate: 2.0,
                },
            )
            .unwrap();
        let mut intervening = OperatorWorkflow::default();
        intervening
            .begin(
                &model,
                OperatorIntent::RetuneReference {
                    reference: "1".into(),
                    target: original_target + 1.0,
                    rate: 2.0,
                },
            )
            .unwrap();
        intervening.confirm(&model, &client).unwrap();
        loop {
            let update = client.recv_timeout(ACCEPTANCE_TIMEOUT).unwrap();
            intervening.after_update(&update);
            if !matches!(update, ClientUpdate::Event { .. }) {
                model.apply_client_update(update.clone());
            }
            if matches!(intervening.state, OperatorWorkflowState::Completed { .. }) {
                break;
            }
        }

        let observation_before_conflict = model.observations.entities[&target].value.clone();
        stale_draft.confirm(&model, &client).unwrap();
        loop {
            let update = client.recv_timeout(ACCEPTANCE_TIMEOUT).unwrap();
            stale_draft.after_update(&update);
            model.apply_client_update(update);
            if matches!(
                stale_draft.state,
                OperatorWorkflowState::Failed { conflict: true, .. }
            ) {
                break;
            }
        }
        assert!(matches!(
            stale_draft.state,
            OperatorWorkflowState::Failed { conflict: true, .. }
        ));
        assert_eq!(
            model.observations.entities[&target].value,
            observation_before_conflict
        );

        client.disconnect().unwrap();
        wait_for(&client, |update| {
            matches!(
                update,
                ClientUpdate::State(super::client::types::ConnectionState::Disconnected)
            )
        });
        assert!(runtime.0.try_wait().unwrap().is_none());
        client.shutdown().unwrap();
    }

    #[test]
    #[ignore = "process acceptance; run after cargo build -p lab-runtime -p lab-workbench"]
    fn real_runtime_reference_reconnect_reconcile_and_replay() {
        let (mut runtime, address) = start_runtime();
        let presentation = PresentationDocument::empty("process-acceptance");
        let mut model = WorkbenchModel::new(presentation.clone());
        let client = ClientHandle::spawn(address).unwrap();
        client.connect(None).unwrap();
        let hello = wait_for(&client, |update| matches!(update, ClientUpdate::Hello(_)));
        let ClientUpdate::Hello(hello) = hello else {
            unreachable!()
        };
        model.apply_client_update(ClientUpdate::Hello(hello.clone()));
        assert!(hello.operations.iter().any(|op| op == "reference_retune"));
        assert!(hello.operations.iter().any(|op| op == "operation_status"));
        assert_eq!(hello.limits["client_pending_requests"], 8);
        let original_cursor = hello.event_latest.clone();

        client.query("reference", json!({"reference":"1"})).unwrap();
        let reference_update = wait_for(
            &client,
            |update| matches!(update, ClientUpdate::Reply { op, kind: ReplyKind::Result, .. } if op == "reference"),
        );
        model.apply_client_update(reference_update.clone());
        let reference = reply_result(reference_update);
        let revision = reference["revision"].as_str().unwrap().to_owned();
        let original_target = reference["target"].as_f64().unwrap();

        client
            .subscribe(original_cursor.clone(), json!({"kinds":[],"targets":[]}))
            .unwrap();
        wait_for(
            &client,
            |update| matches!(update, ClientUpdate::Reply { op, kind: ReplyKind::Result, .. } if op == "subscribe"),
        );

        let args = json!({"reference":"1","expected_revision":revision,
            "target":original_target + 1.0,"rate":2.0});
        client.mutation("reference_retune", args).unwrap();
        let accepted = wait_for(&client, |update| {
            matches!(
                update,
                ClientUpdate::Reply {
                    kind: ReplyKind::MutationAccepted,
                    ..
                }
            )
        });
        let ClientUpdate::Reply {
            recovery: Some(record),
            ..
        } = accepted
        else {
            panic!("accepted mutation needs a recovery record")
        };
        assert_eq!(record.identity.seq, 1);

        client.disconnect().unwrap();
        let disconnected = wait_for(&client, |update| {
            matches!(
                update,
                ClientUpdate::State(super::client::types::ConnectionState::Disconnected)
            )
        });
        model.apply_client_update(disconnected);
        assert_eq!(model.observations.freshness, Freshness::Stale);
        assert_eq!(model.presentation, presentation);
        assert!(runtime.0.try_wait().unwrap().is_none());

        client.connect(Some(hello.scope.clone())).unwrap();
        let reattached = wait_for(&client, |update| matches!(update, ClientUpdate::Hello(_)));
        let ClientUpdate::Hello(reattached) = reattached else {
            unreachable!()
        };
        model.apply_client_update(ClientUpdate::Hello(reattached.clone()));
        assert_eq!(reattached.scope, hello.scope);
        assert_eq!(reattached.next_seq, 2);

        let identity = MutationIdentity {
            scope: hello.scope.clone(),
            seq: 1,
        };
        client.operation_status(identity).unwrap();
        let status = reply_result(wait_for(
            &client,
            |update| matches!(update, ClientUpdate::Reply { op, kind: ReplyKind::Result, .. } if op == "operation_status"),
        ));
        assert_eq!(status["state"], "completed");

        client.query("reference", json!({"reference":"1"})).unwrap();
        let after_retry_update = wait_for(
            &client,
            |update| matches!(update, ClientUpdate::Reply { op, kind: ReplyKind::Result, .. } if op == "reference"),
        );
        model.apply_client_update(after_retry_update.clone());
        let after_retry = reply_result(after_retry_update);
        let reference_identity = RuntimeRef::Reference {
            reference: "1".into(),
        };
        assert_eq!(
            model.observations.entities[&reference_identity].freshness,
            Freshness::Fresh
        );
        assert_eq!(model.presentation, presentation);
        assert_eq!(after_retry["revision"], status["result"]["revision"]);
        client
            .mutation(
                "reference_retune",
                json!({"reference":"1","expected_revision":after_retry["revision"],
                    "target":original_target,"rate":2.0}),
            )
            .unwrap();
        let second_accepted = wait_for(&client, |update| {
            matches!(
                update,
                ClientUpdate::Reply {
                    kind: ReplyKind::MutationAccepted,
                    ..
                }
            )
        });
        let ClientUpdate::Reply {
            recovery: Some(second_record),
            ..
        } = second_accepted
        else {
            panic!("second accepted mutation needs a recovery record")
        };
        assert_eq!(second_record.identity.seq, 2);
        wait_for(&client, |update| {
            matches!(
                update,
                ClientUpdate::Reply {
                    kind: ReplyKind::MutationCompleted,
                    ..
                }
            )
        });

        client
            .subscribe(
                EventCursor {
                    boot_id: original_cursor.boot_id,
                    seq: original_cursor.seq,
                },
                json!({"kinds":["reference"],"targets":[]}),
            )
            .unwrap();
        wait_for(
            &client,
            |update| matches!(update, ClientUpdate::Reply { op, kind: ReplyKind::Result, .. } if op == "subscribe"),
        );
        let replay = wait_for(&client, |update| {
            matches!(update, ClientUpdate::Event { envelope, .. }
                if envelope.get("kind").and_then(Value::as_str) == Some("reference"))
        });
        assert!(matches!(replay, ClientUpdate::Event { .. }));
        client.unsubscribe().unwrap();
        wait_for(
            &client,
            |update| matches!(update, ClientUpdate::Reply { op, kind: ReplyKind::Result, .. } if op == "unsubscribe"),
        );

        client.shutdown().unwrap();
        assert!(runtime.0.try_wait().unwrap().is_none());
    }

    #[test]
    #[ignore = "M14.6B2B1 real Runtime retained-outcome Exact Retry acceptance"]
    fn real_runtime_exact_retry_retained_outcome_never_reexecutes() {
        let (mut runtime, address) = start_runtime_with_recorder();
        let client = ClientHandle::spawn(address).unwrap();
        client.connect(None).unwrap();
        let ClientUpdate::Hello(hello) =
            wait_for(&client, |update| matches!(update, ClientUpdate::Hello(_)))
        else {
            unreachable!()
        };

        client
            .mutation(
                "recording_start",
                json!({"label":"M14.6B2B1 retained Exact Retry"}),
            )
            .unwrap();
        let accepted = wait_for(&client, |update| {
            matches!(
                update,
                ClientUpdate::Reply {
                    kind: ReplyKind::MutationAccepted,
                    ..
                }
            )
        });
        let ClientUpdate::Reply {
            recovery: Some(original),
            ..
        } = accepted
        else {
            panic!("accepted recording start omitted recovery evidence")
        };
        client.disconnect().unwrap();
        wait_for(&client, |update| {
            matches!(
                update,
                ClientUpdate::State(super::client::types::ConnectionState::Disconnected)
            )
        });

        client.connect(Some(hello.scope.clone())).unwrap();
        let ClientUpdate::Hello(reattached) =
            wait_for(&client, |update| matches!(update, ClientUpdate::Hello(_)))
        else {
            unreachable!()
        };
        assert_eq!(reattached.next_seq, 2);
        client.retry_mutation(original.identity.clone()).unwrap();
        let retained = wait_for(&client, |update| {
            matches!(
                update,
                ClientUpdate::Reply {
                    kind: ReplyKind::MutationCompleted,
                    ..
                }
            )
        });
        let ClientUpdate::Reply { envelope, .. } = retained else {
            unreachable!()
        };
        let run_id = envelope["result"]["run_id"].clone();

        client.query("recording_status", json!({})).unwrap();
        let status = reply_result(wait_for(&client, |update| {
            matches!(update, ClientUpdate::Reply { op, kind: ReplyKind::Result, .. }
                if op == "recording_status")
        }));
        assert_eq!(status["state"], "recording", "{status:?}");
        assert_eq!(status["run_id"], run_id);

        client
            .mutation("recording_stop", json!({"run_id":run_id}))
            .unwrap();
        wait_for(&client, |update| {
            matches!(
                update,
                ClientUpdate::Reply {
                    kind: ReplyKind::MutationAccepted,
                    ..
                }
            )
        });
        wait_for(&client, |update| {
            matches!(
                update,
                ClientUpdate::Reply {
                    kind: ReplyKind::MutationCompleted,
                    ..
                }
            )
        });
        assert!(runtime.0.try_wait().unwrap().is_none());
        client.shutdown().unwrap();
    }

    #[test]
    #[ignore = "M14.6B2B1 real Runtime evicted-outcome Exact Retry acceptance"]
    fn real_runtime_exact_retry_outcome_unknown_never_reexecutes() {
        let (mut runtime, address) = start_runtime_with_recorder();
        let client = ClientHandle::spawn(address).unwrap();
        client.connect(None).unwrap();
        let ClientUpdate::Hello(hello) =
            wait_for(&client, |update| matches!(update, ClientUpdate::Hello(_)))
        else {
            unreachable!()
        };

        client
            .mutation(
                "recording_start",
                json!({"label":"M14.6B2B1 exact retry evidence"}),
            )
            .unwrap();
        let accepted = wait_for(&client, |update| {
            matches!(
                update,
                ClientUpdate::Reply {
                    kind: ReplyKind::MutationAccepted,
                    ..
                }
            )
        });
        let ClientUpdate::Reply {
            recovery: Some(original),
            ..
        } = accepted
        else {
            panic!("accepted mutation omitted recovery evidence")
        };
        assert_eq!(original.identity.seq, 1);

        client.disconnect().unwrap();
        wait_for(&client, |update| {
            matches!(
                update,
                ClientUpdate::State(super::client::types::ConnectionState::Disconnected)
            )
        });
        client.connect(Some(hello.scope.clone())).unwrap();
        let ClientUpdate::Hello(reattached) =
            wait_for(&client, |update| matches!(update, ClientUpdate::Hello(_)))
        else {
            unreachable!()
        };
        assert_eq!(reattached.next_seq, 2);

        client.query("recording_status", json!({})).unwrap();
        let recording_before = reply_result(wait_for(&client, |update| {
            matches!(update, ClientUpdate::Reply { op, kind: ReplyKind::Result, .. }
                if op == "recording_status")
        }));
        assert_eq!(
            recording_before["state"], "recording",
            "{recording_before:?}"
        );
        let run_id = recording_before["run_id"].clone();

        client
            .mutation("recording_stop", json!({"run_id":run_id}))
            .unwrap();
        wait_for(&client, |update| {
            matches!(
                update,
                ClientUpdate::Reply {
                    kind: ReplyKind::MutationAccepted,
                    ..
                }
            )
        });
        wait_for(&client, |update| {
            matches!(
                update,
                ClientUpdate::Reply {
                    kind: ReplyKind::MutationCompleted,
                    ..
                }
            )
        });

        for seq in 3..=35_u64 {
            client.query("reference", json!({"reference":"1"})).unwrap();
            let current = reply_result(wait_for(&client, |update| {
                matches!(update, ClientUpdate::Reply { op, kind: ReplyKind::Result, .. }
                    if op == "reference")
            }));
            let revision = current["revision"].as_str().unwrap();
            let target = 20.0 + (seq % 2) as f64;
            client
                .mutation(
                    "reference_retune",
                    json!({"reference":"1","expected_revision":revision,
                        "target":target,"rate":2.0}),
                )
                .unwrap();
            wait_for(&client, |update| {
                matches!(
                    update,
                    ClientUpdate::Reply {
                        kind: ReplyKind::MutationAccepted,
                        ..
                    }
                )
            });
            wait_for(&client, |update| {
                matches!(
                    update,
                    ClientUpdate::Reply {
                        kind: ReplyKind::MutationCompleted,
                        ..
                    }
                )
            });
        }

        client.retry_mutation(original.identity.clone()).unwrap();
        let unknown = wait_for(&client, |update| {
            matches!(
                update,
                ClientUpdate::Reply {
                    kind: ReplyKind::PublicError,
                    envelope,
                    ..
                } if envelope.get("code").and_then(Value::as_str) == Some("outcome_unknown")
            )
        });
        assert!(matches!(unknown, ClientUpdate::Reply { .. }));

        client.query("recording_status", json!({})).unwrap();
        let recording_after = reply_result(wait_for(&client, |update| {
            matches!(update, ClientUpdate::Reply { op, kind: ReplyKind::Result, .. }
                if op == "recording_status")
        }));
        assert_eq!(recording_after["state"], "idle", "{recording_after:?}");

        client.disconnect().unwrap();
        let reconciliation = wait_for(&client, |update| {
            matches!(update, ClientUpdate::ReconciliationRequired { records }
                if records.iter().any(|record| record.identity == original.identity))
        });
        let ClientUpdate::ReconciliationRequired { records } = reconciliation else {
            unreachable!()
        };
        let retained = records
            .iter()
            .find(|record| record.identity == original.identity)
            .unwrap();
        assert_eq!(retained.op, original.op);
        assert_eq!(retained.args, original.args);
        assert_eq!(
            retained.admission,
            super::client::types::KnownAdmission::Accepted
        );
        assert!(runtime.0.try_wait().unwrap().is_none());
        client.shutdown().unwrap();
    }

    #[test]
    #[ignore = "M14.6B4 scenario E Workbench process-crash recovery journal acceptance"]
    fn b4_scenario_e_process_crash_preserves_exact_journal_without_auto_send() {
        const CHILD_MODE: &str = "LAB_WORKBENCH_B4_CRASH_CHILD";
        const CHILD_ADDRESS: &str = "LAB_WORKBENCH_B4_CRASH_ADDRESS";
        const CHILD_JOURNAL: &str = "LAB_WORKBENCH_B4_CRASH_JOURNAL";
        const TEST_NAME: &str = "runtime_acceptance::b4_scenario_e_process_crash_preserves_exact_journal_without_auto_send";

        if env::var_os(CHILD_MODE).is_some() {
            let address = env::var(CHILD_ADDRESS).unwrap().parse().unwrap();
            let journal = PathBuf::from(env::var_os(CHILD_JOURNAL).unwrap());
            let client = ClientHandle::spawn_with_recovery_journal(address, Some(journal)).unwrap();
            client.connect(None).unwrap();
            wait_for(&client, |update| matches!(update, ClientUpdate::Hello(_)));
            client
                .mutation(
                    "reference_retune",
                    json!({"reference":"1","expected_revision":"1","target":7.5,"rate":1.25}),
                )
                .unwrap();
            loop {
                thread::park_timeout(Duration::from_secs(1));
            }
        }

        let directory = env::temp_dir().join(format!(
            "lab-workbench-b4-e-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&directory).unwrap();
        let journal_path = directory.join("recovery-v1.json");
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let child = Command::new(env::current_exe().unwrap())
            .args(["--ignored", "--exact", TEST_NAME, "--nocapture"])
            .env(CHILD_MODE, "1")
            .env(CHILD_ADDRESS, address.to_string())
            .env(CHILD_JOURNAL, &journal_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start Workbench crash helper");
        let mut child = AcceptanceChild(child);
        listener.set_nonblocking(true).unwrap();
        let accept_deadline = Instant::now() + ACCEPTANCE_TIMEOUT;
        let stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        && Instant::now() < accept_deadline =>
                {
                    thread::yield_now();
                }
                Err(error) => panic!("crash helper connection deadline: {error}"),
            }
        };
        stream.set_nonblocking(false).unwrap();
        stream.set_read_timeout(Some(ACCEPTANCE_TIMEOUT)).unwrap();
        stream.set_write_timeout(Some(ACCEPTANCE_TIMEOUT)).unwrap();
        let mut peer = BufReader::new(stream);
        let hello = read_request(&mut peer);
        write_value(
            peer.get_mut(),
            &json!({"v":1,"msg_id":hello["msg_id"],"type":"result",
            "result":{"boot_id":"boot-e","scope":"scope-e","next_seq":"1",
            "operations":["hello","reference_retune","operation_status"],
            "capabilities":[],"limits":{"client_pending_requests":8},
            "event_oldest":{"boot_id":"boot-e","seq":"0"},
            "event_latest":{"boot_id":"boot-e","seq":"0"}}}),
        );
        let mutation = read_request(&mut peer);
        assert_eq!(mutation["op"], "reference_retune");
        assert_eq!(mutation["request_id"], json!({"scope":"scope-e","seq":"1"}));
        assert_eq!(mutation["args"]["target"], 7.5);
        let _ = child.terminate();
        drop(peer);

        let journal = load_journal(&journal_path).expect("crash preserves exact journal");
        assert_eq!(journal.boot_id, "boot-e");
        assert_eq!(journal.scope, "scope-e");
        assert_eq!(journal.records.len(), 1);
        assert_eq!(journal.records[0].op, "reference_retune");
        assert_eq!(journal.records[0].args, mutation["args"]);

        let restart_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        restart_listener.set_nonblocking(true).unwrap();
        let restart_address = restart_listener.local_addr().unwrap();
        let restarted =
            ClientHandle::spawn_with_recovery_journal(restart_address, Some(journal_path.clone()))
                .unwrap();
        assert!(matches!(
            restart_listener.accept(),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock
        ));
        restart_listener.set_nonblocking(false).unwrap();
        let (quiet_tx, quiet_rx) = mpsc::channel();
        let restart_peer = thread::spawn(move || {
            let (stream, _) = restart_listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_millis(250)))
                .unwrap();
            let mut reader = BufReader::new(stream);
            let hello = read_request(&mut reader);
            assert_eq!(hello["args"]["scope"], "scope-e");
            write_value(
                reader.get_mut(),
                &json!({"v":1,"msg_id":hello["msg_id"],"type":"result",
                "result":{"boot_id":"boot-e","scope":"scope-e","next_seq":"1",
                "operations":["hello","reference_retune","operation_status"],
                "capabilities":[],"limits":{"client_pending_requests":8},
                "event_oldest":{"boot_id":"boot-e","seq":"0"},
                "event_latest":{"boot_id":"boot-e","seq":"0"}}}),
            );
            let mut byte = [0_u8; 1];
            quiet_tx.send(reader.read(&mut byte)).unwrap();
        });
        restarted.connect(Some("scope-e".into())).unwrap();
        wait_for(&restarted, |update| {
            matches!(update, ClientUpdate::Hello(_))
        });
        let reconciliation = wait_for(&restarted, |update| {
            matches!(update, ClientUpdate::ReconciliationRequired { .. })
        });
        let ClientUpdate::ReconciliationRequired { records } = reconciliation else {
            unreachable!()
        };
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].op, journal.records[0].op);
        assert_eq!(records[0].args, journal.records[0].args);
        assert!(quiet_rx.recv_timeout(ACCEPTANCE_TIMEOUT).unwrap().is_err());
        restarted.shutdown().unwrap();
        restart_peer.join().unwrap();
        assert_eq!(
            load_journal(&journal_path).unwrap().records,
            journal.records
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    #[ignore = "M14.6B4 scenario A real-process Runtime availability acceptance"]
    fn b4_scenario_a_runtime_absent_then_manual_connect_reaches_fresh() {
        let address = unused_loopback_address();
        let directory = env::temp_dir().join(format!(
            "lab-workbench-b4-a-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let journal_path = directory.join("recovery-v1.json");
        let client =
            ClientHandle::spawn_with_recovery_journal(address, Some(journal_path.clone())).unwrap();
        assert!(!journal_path.exists());
        client.connect(None).unwrap();
        wait_for(&client, |update| {
            matches!(update, ClientUpdate::TransportFailure { .. })
        });
        wait_for(&client, |update| {
            matches!(update, ClientUpdate::State(ConnectionState::Disconnected))
        });

        let (mut runtime, runtime_address) = start_runtime_on(address.port());
        assert_eq!(runtime_address, address);
        let mut model = WorkbenchModel::new(PresentationDocument::empty("b4-a"));
        let mut rebuild = RebuildCoordinator::default();
        client.connect(None).unwrap();
        apply_until(&client, &mut model, &mut rebuild, |_, model| {
            model.observations.freshness == Freshness::Fresh
        });
        assert_eq!(model.connection, ConnectionState::Ready);
        assert!(model.hello.is_some());
        assert!(model.recovery_problem.is_none());
        assert!(!journal_path.exists());
        let reference = model.observations.entities[&RuntimeRef::Reference {
            reference: "1".into(),
        }]
            .value
            .clone();
        client
            .mutation(
                "reference_retune",
                json!({"reference":"1","expected_revision":reference["revision"],
                    "target":reference["target"].as_f64().unwrap() + 0.125,"rate":2.0}),
            )
            .unwrap();
        wait_for(&client, |update| {
            matches!(
                update,
                ClientUpdate::Reply {
                    kind: ReplyKind::MutationAccepted,
                    ..
                }
            )
        });
        assert!(journal_path.is_file());
        assert!(runtime.0.try_wait().unwrap().is_none());
        client.shutdown().unwrap();
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    #[ignore = "M14.6B4 scenario B explicit Disconnect real-process acceptance"]
    fn b4_scenario_b_explicit_disconnect_requires_manual_fresh_reconnect() {
        let (mut runtime, address) = start_runtime();
        let client = ClientHandle::spawn(address).unwrap();
        let mut model = WorkbenchModel::new(PresentationDocument::empty("b4-b"));
        let mut rebuild = RebuildCoordinator::default();
        client.connect(None).unwrap();
        apply_until(&client, &mut model, &mut rebuild, |_, model| {
            model.observations.freshness == Freshness::Fresh
        });
        let scope = model.hello.as_ref().unwrap().scope.clone();

        client.disconnect().unwrap();
        apply_until(&client, &mut model, &mut rebuild, |update, _| {
            matches!(update, ClientUpdate::State(ConnectionState::Disconnected))
        });
        assert_eq!(model.observations.freshness, Freshness::Stale);
        let quiet_deadline = Instant::now() + Duration::from_millis(100);
        while Instant::now() < quiet_deadline {
            match client.recv_timeout(quiet_deadline.saturating_duration_since(Instant::now())) {
                Ok(update) => assert!(!matches!(
                    update,
                    ClientUpdate::Hello(_)
                        | ClientUpdate::State(ConnectionState::Connecting)
                        | ClientUpdate::State(ConnectionState::Reattaching)
                )),
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(error) => panic!("client stopped during explicit disconnect proof: {error}"),
            }
        }
        assert!(runtime.0.try_wait().unwrap().is_none());

        client.connect(Some(scope.clone())).unwrap();
        apply_until(&client, &mut model, &mut rebuild, |_, model| {
            model.observations.freshness == Freshness::Fresh
        });
        assert_eq!(model.hello.as_ref().unwrap().scope, scope);
        assert!(runtime.0.try_wait().unwrap().is_none());
        client.shutdown().unwrap();
    }

    #[test]
    #[ignore = "M14.6B4 scenario D Runtime restart and quarantine acceptance"]
    fn b4_scenario_d_runtime_restart_quarantines_old_boot_and_builds_new_epoch() {
        let (mut runtime_a, address) = start_runtime();
        let client = ClientHandle::spawn(address).unwrap();
        let mut model = WorkbenchModel::new(PresentationDocument::empty("b4-d"));
        let mut rebuild = RebuildCoordinator::default();
        client.connect(None).unwrap();
        apply_until(&client, &mut model, &mut rebuild, |_, model| {
            model.observations.freshness == Freshness::Fresh
        });
        let boot_a = model.hello.as_ref().unwrap().boot_id.clone();
        let target = RuntimeRef::Reference {
            reference: "1".into(),
        };
        let reference = model.observations.entities[&target].value.clone();
        let original_target = reference["target"].clone();
        let mut stale_draft = OperatorWorkflow::default();
        stale_draft
            .begin(
                &model,
                OperatorIntent::RetuneReference {
                    reference: "1".into(),
                    target: reference["target"].as_f64().unwrap() + 0.75,
                    rate: 2.0,
                },
            )
            .unwrap();
        client
            .mutation(
                "reference_retune",
                json!({"reference":"1","expected_revision":reference["revision"],
                    "target":reference["target"].as_f64().unwrap() + 0.5,"rate":2.0}),
            )
            .unwrap();
        let accepted = apply_until(&client, &mut model, &mut rebuild, |update, _| {
            matches!(
                update,
                ClientUpdate::Reply {
                    kind: ReplyKind::MutationAccepted,
                    ..
                }
            )
        });
        let ClientUpdate::Reply {
            recovery: Some(old_record),
            ..
        } = accepted
        else {
            panic!("accepted mutation omitted recovery evidence")
        };

        let _ = runtime_a.terminate();
        let (mut runtime_b, restarted_address) = start_runtime_on(address.port());
        assert_eq!(restarted_address, address);
        apply_until(&client, &mut model, &mut rebuild, |update, model| {
            matches!(update, ClientUpdate::State(ConnectionState::Disconnected))
                && model
                    .recovery
                    .quarantined
                    .iter()
                    .any(|item| item.record.identity == old_record.identity)
        });
        assert_eq!(model.observations.freshness, Freshness::Stale);
        assert!(model.recovery.mutations.is_empty());
        assert!(model.recovery.quarantined.iter().any(|item| {
            item.record.identity == old_record.identity
                && item.record.op == old_record.op
                && item.record.args == old_record.args
        }));
        let status = RecoveryStatusTracker::default();
        assert!(!ExactRetryWorkflow::default().can_begin(&model, &status, &old_record.identity));

        client.connect(None).unwrap();
        apply_until(&client, &mut model, &mut rebuild, |_, model| {
            model.observations.freshness == Freshness::Fresh
        });
        let boot_b = model.hello.as_ref().unwrap().boot_id.clone();
        assert_ne!(boot_b, boot_a);
        assert_eq!(
            model.observations.entities[&target].value["target"], original_target,
            "old-boot mutation was sent into Runtime B"
        );
        assert!(stale_draft.confirm(&model, &client).is_err());
        assert!(model.recovery.quarantined.iter().any(|item| {
            item.record.identity == old_record.identity
                && item.record.op == old_record.op
                && item.record.args == old_record.args
        }));
        assert!(runtime_b.0.try_wait().unwrap().is_none());
        client.shutdown().unwrap();
    }

    #[test]
    #[ignore = "M14.6B4 scenario J corrupt journal with real Runtime acceptance"]
    fn b4_scenario_j_corrupt_journal_allows_fresh_observation_but_blocks_mutation() {
        let directory = env::temp_dir().join(format!(
            "lab-workbench-b4-j-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("recovery-v1.json");
        let corrupt = b"{known-corrupt-journal\xff".to_vec();
        fs::write(&path, &corrupt).unwrap();
        let (mut runtime, address) = start_runtime();
        let client =
            ClientHandle::spawn_with_recovery_journal(address, Some(path.clone())).unwrap();
        let mut model = WorkbenchModel::new(PresentationDocument::empty("b4-j"));
        let mut rebuild = RebuildCoordinator::default();
        client.connect(None).unwrap();
        apply_until(&client, &mut model, &mut rebuild, |_, model| {
            model.observations.freshness == Freshness::Fresh
        });
        assert!(model.recovery_problem.is_some());
        assert_eq!(fs::read(&path).unwrap(), corrupt);

        let command_id = client
            .mutation(
                "reference_retune",
                json!({"reference":"1","expected_revision":"1","target":2.0,"rate":1.0}),
            )
            .unwrap();
        let rejected = wait_for(&client, |update| {
            matches!(update, ClientUpdate::LocalRejected { command_id: id, reason }
                if *id == command_id && reason == "recovery_journal_unavailable")
        });
        model.apply_client_update(rejected);
        assert!(model.recovery.mutations.is_empty());
        assert!(!ExactRetryWorkflow::default().can_begin(
            &model,
            &RecoveryStatusTracker::default(),
            &MutationIdentity {
                scope: model.hello.as_ref().unwrap().scope.clone(),
                seq: 1,
            },
        ));
        assert_eq!(fs::read(&path).unwrap(), corrupt);
        assert!(runtime.0.try_wait().unwrap().is_none());
        client.shutdown().unwrap();
        assert_eq!(fs::read(&path).unwrap(), corrupt);
        fs::remove_dir_all(directory).unwrap();
    }
}
