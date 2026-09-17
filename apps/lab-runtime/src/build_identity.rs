//! Content identity of the executable that supplies built-in managed components.
//!
//! The production identity hashes the current process executable once with a fixed
//! streaming buffer. Provenance callers reuse the cached digest and never load the
//! executable into memory as one allocation.

use sha2::{Digest, Sha256};
use std::{
    fmt,
    fs::File,
    io::{self, Read},
    sync::OnceLock,
};

const HASH_BUFFER_BYTES: usize = 64 * 1024;
static RUNTIME_BINARY_SHA256: OnceLock<Result<[u8; 32], RuntimeBinaryIdentityError>> =
    OnceLock::new();

/// Bounded failure class for resolving or reading the current executable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeBinaryIdentityError {
    /// The operating system could not identify the current executable path.
    CurrentExecutable,
    /// The identified executable could not be opened for streaming read.
    Open,
    /// Streaming the executable bytes failed.
    Read,
}

impl fmt::Display for RuntimeBinaryIdentityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::CurrentExecutable => "current executable identity unavailable",
            Self::Open => "current executable could not be opened",
            Self::Read => "current executable could not be hashed",
        })
    }
}

impl std::error::Error for RuntimeBinaryIdentityError {}

/// Return the cached SHA-256 of the exact executable bytes running this process.
pub fn runtime_binary_sha256() -> Result<[u8; 32], RuntimeBinaryIdentityError> {
    *RUNTIME_BINARY_SHA256.get_or_init(compute_runtime_binary_sha256)
}

fn compute_runtime_binary_sha256() -> Result<[u8; 32], RuntimeBinaryIdentityError> {
    let path =
        std::env::current_exe().map_err(|_| RuntimeBinaryIdentityError::CurrentExecutable)?;
    let file = File::open(path).map_err(|_| RuntimeBinaryIdentityError::Open)?;
    hash_reader(file).map_err(|_| RuntimeBinaryIdentityError::Read)
}

fn hash_reader(mut reader: impl Read) -> io::Result<[u8; 32]> {
    let mut hash = Sha256::new();
    let mut buffer = [0u8; HASH_BUFFER_BYTES];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
    }
    Ok(hash.finalize().into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn streaming_build_identity_is_stable_and_distinguishes_exact_bytes() {
        let first = hash_reader(Cursor::new(b"runtime build A")).unwrap();
        let again = hash_reader(Cursor::new(b"runtime build A")).unwrap();
        let second = hash_reader(Cursor::new(b"runtime build B")).unwrap();
        assert_eq!(first, again);
        assert_ne!(first, second);
        let expected: [u8; 32] = Sha256::digest(b"runtime build A").into();
        assert_eq!(first, expected);
    }
}
