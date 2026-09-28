//! Headless M14.2 shell for the private Workbench Application client.

mod client;

use client::{ClientHandle, ClientUpdate};
use serde_json::json;
use std::{net::SocketAddr, process::ExitCode, time::Duration};

const STARTUP_WAIT: Duration = Duration::from_secs(5);

fn usage() {
    eprintln!("usage: lab-workbench --connect 127.0.0.1:PORT [--scope SCOPE]");
}

fn arguments() -> Result<(SocketAddr, Option<String>), &'static str> {
    let mut args = std::env::args().skip(1);
    let mut address = None;
    let mut scope = None;
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
            _ => return Err("unknown or duplicate argument"),
        }
    }
    Ok((address.ok_or("--connect is required")?, scope))
}

fn run() -> Result<(), String> {
    let (address, scope) = arguments().map_err(str::to_owned)?;
    let client = ClientHandle::spawn(address).map_err(|error| error.to_string())?;
    client.connect(scope).map_err(|error| error.to_string())?;
    let deadline = std::time::Instant::now() + STARTUP_WAIT;
    let result = loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            break Err("hello did not complete before the startup deadline".to_owned());
        }
        match client.recv_timeout(remaining) {
            Ok(ClientUpdate::Hello(hello)) => {
                println!(
                    "{}",
                    json!({"status":"connected","boot_id":hello.boot_id,
                        "scope":hello.scope,"next_seq":hello.next_seq,
                        "operations":hello.operations.len(),
                        "capabilities":hello.capabilities.as_array().map_or(0,Vec::len)})
                );
                break Ok(());
            }
            Ok(ClientUpdate::TransportFailure { reason }) => break Err(reason),
            Ok(ClientUpdate::Reply { envelope, .. })
                if envelope.get("type").and_then(serde_json::Value::as_str) == Some("error") =>
            {
                break Err(format!("hello rejected: {envelope}"));
            }
            Ok(_) => {}
            Err(error) => break Err(format!("client worker stopped during hello: {error}")),
        }
    };
    let _ = client.disconnect();
    client.shutdown().map_err(str::to_owned)?;
    result
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
mod runtime_acceptance {
    use super::client::{
        ClientHandle, ClientUpdate,
        types::{EventCursor, MutationIdentity, ReplyKind},
    };
    use serde_json::{Value, json};
    use std::{
        io::{BufRead, BufReader},
        path::PathBuf,
        process::{Child, Command, Stdio},
        sync::mpsc,
        thread,
        time::{Duration, Instant},
    };

    const ACCEPTANCE_TIMEOUT: Duration = Duration::from_secs(5);

    struct RuntimeChild(Child);

    impl Drop for RuntimeChild {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
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
        let binary = runtime_binary();
        assert!(
            binary.is_file(),
            "build lab-runtime before the process acceptance: {}",
            binary.display()
        );
        let mut child = Command::new(binary)
            .args(["--serve", "--profile", "virtual-demo", "--port", "0"])
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
            RuntimeChild(child),
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

    #[test]
    #[ignore = "process acceptance; run after cargo build -p lab-runtime -p lab-workbench"]
    fn real_runtime_reference_reconnect_reconcile_and_replay() {
        let (mut runtime, address) = start_runtime();
        let client = ClientHandle::spawn(address).unwrap();
        client.connect(None).unwrap();
        let hello = wait_for(&client, |update| matches!(update, ClientUpdate::Hello(_)));
        let ClientUpdate::Hello(hello) = hello else {
            unreachable!()
        };
        assert!(hello.operations.iter().any(|op| op == "reference_retune"));
        assert!(hello.operations.iter().any(|op| op == "operation_status"));
        assert_eq!(hello.limits["client_pending_requests"], 8);
        let original_cursor = hello.event_latest.clone();

        client.query("reference", json!({"reference":"1"})).unwrap();
        let reference = reply_result(wait_for(
            &client,
            |update| matches!(update, ClientUpdate::Reply { op, kind: ReplyKind::Result, .. } if op == "reference"),
        ));
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
        wait_for(&client, |update| {
            matches!(
                update,
                ClientUpdate::State(super::client::types::ConnectionState::Disconnected)
            )
        });
        assert!(runtime.0.try_wait().unwrap().is_none());

        client.connect(Some(hello.scope.clone())).unwrap();
        let reattached = wait_for(&client, |update| matches!(update, ClientUpdate::Hello(_)));
        let ClientUpdate::Hello(reattached) = reattached else {
            unreachable!()
        };
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

        client.retry_mutation(record.identity.clone()).unwrap();
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
        assert_eq!(envelope["result"], status["result"]);

        client.query("reference", json!({"reference":"1"})).unwrap();
        let after_retry = reply_result(wait_for(
            &client,
            |update| matches!(update, ClientUpdate::Reply { op, kind: ReplyKind::Result, .. } if op == "reference"),
        ));
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
}
