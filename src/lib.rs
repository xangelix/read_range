#![doc = include_str!("../README.md")]
#![deny(unsafe_code)]

#[cfg(not(target_family = "wasm"))]
pub mod read_at;

#[cfg(not(target_family = "wasm"))]
pub use read_at::*;

/// Progress trait for monitoring read operations.
#[cfg(not(target_family = "wasm"))]
pub trait Progress {
    /// Increase progress by `delta` bytes.
    fn inc(&self, delta: u64);
    /// Signal that the read has finished (success or error).
    fn finish(&self);
}
