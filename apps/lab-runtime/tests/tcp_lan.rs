//! Explicit trusted-LAN TCP startup uses the same Application protocol.

use lab_runtime::service::ServiceOptions;
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    net::{Ipv4Addr, SocketAddr, TcpStream, UdpSocket},
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

#[test]
fn tcp_bind_requires_explicit_unicast_opt_in_in_both_startup_modes() {
    assert_eq!(
        ServiceOptions::parse(&["--serve", "--config", "runtime.toml"])
            .unwrap()
            .bind_address(),
        Ipv4Addr::LOCALHOST
    );
    for prefix in [
        vec!["--serve", "--config", "runtime.toml"],
        vec!["--serve", "--profile", "virtual-demo"],
    ] {
        for (address, opt_in, valid) in [
            ("127.0.0.2", false, true),
            ("192.168.1.50", false, false),
            ("192.168.1.50", true, true),
            ("0.0.0.0", true, false),
            ("255.255.255.255", true, false),
            ("224.0.0.1", true, false),
            ("0.1.2.3", true, false),
            ("240.1.2.3", true, false),
            ("::1", true, false),
            ("localhost", true, false),
        ] {
            let mut args = prefix.clone();
            args.extend(["--bind", address]);
            if opt_in {
                args.push("--allow-remote-tcp");
            }
            if prefix[1] == "--profile" {
                args.extend(["--port", "0"]);
            }
            assert_eq!(ServiceOptions::parse(&args).is_ok(), valid, "{args:?}");
        }
    }
    for args in [
        vec!["--serve", "--config", "x", "--allow-remote-tcp"],
        vec!["--serve", "--config", "x", "--bind"],
        vec![
            "--serve",
            "--config",
            "x",
            "--bind",
            "192.168.1.2",
            "--allow-remote-tcp",
            "--allow-remote-tcp",
        ],
        vec![
            "--serve",
            "--config",
            "x",
            "--bind",
            "192.168.1.2",
            "--allow-remote-tcp",
            "--ws-port",
            "0",
        ],
    ] {
        assert!(ServiceOptions::parse(&args).is_err(), "{args:?}");
    }
}

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn exchange(reader: &mut BufReader<TcpStream>, request: Value) -> Value {
    writeln!(reader.get_mut(), "{request}").unwrap();
    let mut line = String::new();
    assert!(reader.read_line(&mut line).unwrap() > 0);
    serde_json::from_str(&line).unwrap()
}

#[test]
fn config_startup_admits_tcp_on_the_selected_interface_with_loopback_default() {
    // UDP connect selects a local route without transmitting a datagram. This
    // exercises the host's assigned interface, not a physical second PC.
    let route = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).unwrap();
    route.connect("192.0.2.1:9").unwrap();
    let local_ip = route.local_addr().unwrap().ip();
    assert!(!local_ip.is_loopback() && !local_ip.is_unspecified());
    let mut entropy = [0u8; 16];
    getrandom::fill(&mut entropy).unwrap();
    let suffix: String = entropy.iter().map(|b| format!("{b:02x}")).collect();
    let directory = std::env::temp_dir().join(format!("lab-tcp-lan-{suffix}"));
    std::fs::create_dir(&directory).unwrap();
    let config = directory.join("runtime.toml");
    let source = include_str!("../../../examples/runtime.virtual.toml")
        .replace("port = 7420", "port = 0")
        .replace("enabled = true", "enabled = false")
        .replace("policy = \"required\"", "policy = \"best_effort\"");
    std::fs::write(&config, source).unwrap();
    for selected in [None, Some(local_ip)] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_lab-runtime"));
        command
            .args(["--serve", "--config"])
            .arg(&config)
            .current_dir(&directory)
            .env("LAB_RUNTIME_LOG_DIRECTORY", directory.join("logs"))
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        if let Some(ip) = selected {
            command.args(["--bind", &ip.to_string(), "--allow-remote-tcp"]);
        }
        let mut child = Process(command.spawn().unwrap());
        let stdout = child.0.stdout.take().unwrap();
        let (tx, rx) = mpsc::sync_channel(1);
        let readiness = std::thread::spawn(move || {
            let mut line = String::new();
            BufReader::new(stdout).read_line(&mut line).unwrap();
            tx.send(line).unwrap();
        });
        let ready: Value =
            serde_json::from_str(&rx.recv_timeout(Duration::from_secs(5)).unwrap()).unwrap();
        readiness.join().unwrap();
        let port = ready["port"].as_u64().unwrap() as u16;
        let ip = selected.unwrap_or(Ipv4Addr::LOCALHOST.into());
        if selected.is_none() {
            assert!(
                TcpStream::connect_timeout(
                    &SocketAddr::new(local_ip, port),
                    Duration::from_millis(250)
                )
                .is_err()
            );
        }
        let stream =
            TcpStream::connect_timeout(&SocketAddr::new(ip, port), Duration::from_secs(2)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut reader = BufReader::new(stream);
        let hello = exchange(
            &mut reader,
            json!({"v":1,"msg_id":"hello","op":"hello","args":{"scope":null}}),
        );
        assert_eq!(hello["type"], "result");
        assert_eq!(hello["result"]["boot_id"], ready["boot_id"]);
        let latest = exchange(
            &mut reader,
            json!({"v":1,"msg_id":"latest","op":"latest","args":{"signal":{"instrument":"1","parameter":"1"}}}),
        );
        assert_eq!(latest["type"], "result");
        let accepted = exchange(
            &mut reader,
            json!({"v":1,"msg_id":"shutdown","op":"runtime_shutdown",
            "request_id":{"scope":hello["result"]["scope"],"seq":hello["result"]["next_seq"]},"args":{}}),
        );
        assert_eq!(accepted["state"], "accepted");
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let terminal: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(terminal["state"], "completed");
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(status) = child.0.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
    }
    // Never replace a failed selected-address bind with a wildcard/fallback.
    assert!(std::net::TcpListener::bind("192.0.2.1:0").is_err());
    std::fs::remove_dir_all(directory).unwrap();
}
