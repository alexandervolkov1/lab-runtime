//! Platform-specific serial names behind one validation boundary.
//!
//! Legacy schema-v1 resource kind names remain unchanged. Validation never opens
//! a device or resolves symlinks, and does not claim hardware availability.

/// Normalize a legacy COM name or, on Linux, validate an absolute `/dev/` path.
/// COM names remain accepted on Linux for portable configuration validation;
/// they do not imply that a corresponding device exists on that host.
pub(crate) fn normalize_serial_port(port: &str) -> Result<String, &'static str> {
    let port = port.trim();
    #[cfg(target_os = "linux")]
    if port.starts_with("/dev/") {
        return linux_serial_path(port);
    }
    let digits = port
        .strip_prefix("COM")
        .or_else(|| port.strip_prefix("com"))
        .ok_or("Windows port must be COM<number>")?;
    let number = digits.parse::<u16>().map_err(|_| "invalid COM port")?;
    if number == 0 {
        return Err("invalid COM port");
    }
    Ok(format!("COM{number}"))
}

#[cfg(target_os = "linux")]
fn linux_serial_path(port: &str) -> Result<String, &'static str> {
    // Keep spelling and case stable for device nodes and serial/by-id symlinks.
    // Reject traversal and ambiguous spelling without doing filesystem I/O.
    if port.len() > 4096
        || port.chars().any(char::is_control)
        || port[5..]
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
    {
        return Err("invalid Linux serial device path");
    }
    Ok(port.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_names_keep_the_existing_normalization_and_bounds() {
        assert_eq!(normalize_serial_port(" com003 "), Ok("COM3".into()));
        assert_eq!(normalize_serial_port("COM65535"), Ok("COM65535".into()));
        for port in ["COM0", "COM65536", "Com3", "ttyUSB0", "", "COM3/extra"] {
            assert!(normalize_serial_port(port).is_err(), "{port}");
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_device_paths_are_bounded_and_preserve_case_without_io() {
        for port in ["/dev/ttyUSB0", "/dev/ttyACM0", "/dev/serial/by-id/USB-Test"] {
            assert_eq!(normalize_serial_port(port).unwrap(), port);
        }
        for port in [
            "/dev/",
            "/dev//ttyUSB0",
            "/dev/../tmp/file",
            "/dev/./ttyUSB0",
            "/dev/ttyUSB0/",
            "/dev/tty\0USB0",
            "/tmp/ttyUSB0",
            "dev/ttyUSB0",
        ] {
            assert!(normalize_serial_port(port).is_err(), "{port:?}");
        }
        assert!(normalize_serial_port(&format!("/dev/{}", "a".repeat(4096))).is_err());
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn linux_device_paths_do_not_change_windows_validation() {
        assert!(normalize_serial_port("/dev/ttyUSB0").is_err());
    }
}
