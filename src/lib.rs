#[cfg(not(target_family = "wasm"))]
pub mod read_at;

#[cfg(not(target_family = "wasm"))]
pub use read_at::*;

#[cfg(not(target_family = "wasm"))]
pub trait Progress {
    fn inc(&self, delta: u64);
    fn finish(&self);
}
