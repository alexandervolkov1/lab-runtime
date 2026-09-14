use std::process::Command;

#[test]
fn demo_finishes_with_deterministic_generic_output_and_bounded_window() {
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_lab-runtime"))
            .output()
            .unwrap()
    };
    let first = run();
    let second = run();
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(second.status.success());
    assert_eq!(first.stdout, second.stdout);
    let output = String::from_utf8(first.stdout).unwrap();
    assert!(output.contains("Milestone 1: virtual-only"));
    assert!(output.contains("temperature: Float"));
    assert!(output.contains("heater_power: Float"));
    assert!(output.contains("Actuator / ReadWrite / OutputAffecting"));
    assert!(output.contains("latest: unknown"));
    assert!(output.contains("t=4s Good Some(Float(24.0)) °C"));
    assert!(output.contains("window: 3 samples, oldest=2s, latest=4s"));
}
