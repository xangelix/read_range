# read_range

[![Crates.io](https://img.shields.io/crates/v/read_range.svg)](https://crates.io/crates/read_range)
[![Docs.rs](https://docs.rs/read_range/badge.svg)](https://docs.rs/read_range)

A simple, fast, and portable Rust crate for reading a specific range of bytes from a file. Provides both synchronous and asynchronous APIs with optional progress reporting.

## Overview

`read_range` offers a robust and efficient way to read a segment of a file without needing to read the entire file into memory. It's built for performance and safety, especially in concurrent applications.

On Unix and Windows, it uses high-performance, platform-specific calls (`pread` on Unix / `seek_read` on Windows) that read from a specific offset without mutating the global file cursor. This makes it safe to read from the same file handle across multiple threads simultaneously. For other platforms, it uses a standard `seek` and `read` fallback.

## Key Features

- **Asynchronous & Synchronous:** Provides both `async` and blocking APIs to fit any application.
- **High Performance:** Uses efficient positional I/O on supported platforms (Unix, Windows) to avoid file cursor contention.
- **Concurrent-Safe:** The primary `read_at` strategy is stateless and safe to use across multiple threads without locks.
- **Portable:** Includes a fallback implementation for platforms without specialized positional read APIs.
- **Progress Reporting:** All API variants have a `_with_progress` version to easily integrate with UI components like progress bars.
- **Robust Error Handling:** Distinguishes between recoverable I/O errors (like task cancellation) and unrecoverable bugs (like out-of-memory), providing a predictable and debuggable API contract.

## Installation

Add `read_range` to your `Cargo.toml`:

```sh
cargo add read_range
```

The async API uses `tokio`. Enable with the default `async` feature (on by default):

```toml
read_range = { version = "0.2", default-features = true } # async ON
# or disable:
# read_range = { version = "0.2", default-features = false } # sync-only
```

## Usage

### Basic Synchronous Read

The simplest way to use the crate is to perform a blocking read. This is ideal for command-line tools or simple scripts.

```rust
use std::{
    fs::File,
    io::{self, Write},
};

use read_range::{Progress, read_byte_range};
use tempfile::NamedTempFile;

fn main() -> io::Result<()> {
    // 1. Create a temporary file with some data.
    let mut temp_file = NamedTempFile::new()?;
    temp_file.write_all(b"Hello, world! This is a test file.")?;
    let path = temp_file.path();

    // 2. Read a 5-byte range starting at offset 7.
    // This should read "world".
    let data = read_byte_range(path, 7, 5)?;

    assert_eq!(data, b"world");
    println!("Successfully read: {}", String::from_utf8_lossy(&data));

    Ok(())
}
```

### Asynchronous Read with Tokio

For non-blocking applications, use the `async` variants within a Tokio runtime.

```rust
use std::io;

use read_range::async_read_byte_range;
use tempfile::NamedTempFile;
use tokio::{fs::File, io::AsyncWriteExt as _};

#[tokio::main]
async fn main() -> io::Result<()> {
    // 1. Create a temporary file with some data.
    let mut temp_file = NamedTempFile::new()?;
    let mut file = File::create(temp_file.path()).await?;
    file.write_all(b"Hello, world! This is a test file.").await?;
    let path = temp_file.path().to_path_buf(); // Clone path before file is dropped.

    // 2. Asynchronously read the 5-byte range for "world".
    let data = async_read_byte_range(path, 7, 5).await?;

    assert_eq!(data, b"world");
    println!("Successfully read asynchronously: {}", String::from_utf8_lossy(&data));

    Ok(())
}
```

### Asynchronous Read with Progress Reporting

For long-running reads, you can track progress by implementing the `Progress` trait.

```rust
use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use read_range::{Progress, async_read_byte_range_with_progress};
use tempfile::NamedTempFile;
use tokio::{fs::File, io::AsyncWriteExt as _};

// A simple progress tracker that prints updates.
#[derive(Clone)]
struct MyProgress {
    bytes_read: Arc<AtomicU64>,
}

impl Progress for MyProgress {
    fn inc(&self, delta: u64) {
        let new = self.bytes_read.fetch_add(delta, Ordering::SeqCst) + delta;
        println!("Progress: {new} bytes read");
    }

    fn finish(&self) {
        println!("Read complete!");
    }
}

#[tokio::main]
async fn main() -> io::Result<()> {
    // 1. Create a larger temporary file.
    let mut temp_file = NamedTempFile::new()?;
    let mut file = File::create(temp_file.path()).await?;
    file.write_all(&[0; 1024 * 128]).await?; // 128 KiB
    let path = temp_file.path().to_path_buf();

    // 2. Create an instance of our progress tracker.
    let progress = MyProgress {
        bytes_read: Arc::new(AtomicU64::new(0)),
    };

    // 3. Read 100 KiB and report progress along the way.
    let data = async_read_byte_range_with_progress(path, 0, 1024 * 100, progress).await?;

    assert_eq!(data.len(), 1024 * 100);
    println!("Successfully read {} bytes with progress reporting.", data.len());

    Ok(())
}
```

## API

The crate exposes four main functions:

- `read_byte_range(path, offset, len)`: Synchronously reads a byte range.
- `read_byte_range_with_progress(path, offset, len, progress)`: Synchronously reads a byte range and reports progress.
- `async_read_byte_range(path, offset, len)`: Asynchronously reads a byte range.
- `async_read_byte_range_with_progress(path, offset, len, progress)`: Asynchronously reads a byte range and reports progress.

## Error Handling Philosophy

This crate is designed for robustness and follows standard Rust error handling idioms.

- It returns `io::Result` for all expected I/O failures, invalid arguments (e.g., a read range that would overflow `u64`), or graceful events like async task cancellation.
- It **panics** only when a background task encounters an unrecoverable state, such as an out-of-memory error during buffer allocation. This "fail-fast" approach prevents your application from continuing in a corrupt state and makes critical bugs easier to diagnose. The panic is propagated from the background thread to the calling task.

## Future Work
A streaming API that returns an `impl Read` or `impl AsyncRead` instead of a `Vec<u8>` is being considered for a future release. This would enable efficient processing of very large file segments in memory-constrained environments.

## License

This project is licensed under the MIT license.
