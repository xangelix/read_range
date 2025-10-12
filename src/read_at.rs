//! Provides the core implementation for reading specific byte ranges from files.
//!
//! On supported platforms (Unix, Windows), this module leverages efficient and
//! concurrent-safe positional I/O syscalls. For other targets, it uses a
//! portable `seek` and `read` fallback.
//!
//! The public API includes synchronous and asynchronous functions, each with a
//! corresponding `_with_progress` variant for monitoring long-running reads.

use std::{io, path::Path};

use super::Progress;

/// Handles the result of a `spawn_blocking` call that returns an `io::Result`.
///
/// This helper correctly propagates panics from the background task, ensuring that
/// critical failures like out-of-memory are not hidden. It also gracefully handles
/// task cancellations by converting them into an `io::Error` of kind `Interrupted`,
/// which is the standard behavior for async operations that are cancelled.
#[cfg(any(unix, windows))]
fn handle_blocking_io_task_result<T>(
    result: Result<io::Result<T>, tokio::task::JoinError>,
) -> io::Result<T> {
    match result {
        Ok(inner_result) => inner_result,
        Err(e) => {
            if e.is_panic() {
                // The blocking task panicked. Propagate the panic to the caller.
                std::panic::resume_unwind(e.into_panic());
            } else {
                // The task was cancelled. This is a normal event in async,
                // not a bug. We translate it into an error.
                Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "blocking task was cancelled",
                ))
            }
        }
    }
}

/// Reads a specific range of bytes from a file asynchronously.
///
/// This function provides a portable and efficient way to read a segment of a file.
/// On supported platforms (Unix, Windows), it uses the most efficient underlying
/// OS syscall (`pread` or `ReadFileScatter`) by running the blocking
/// `std::fs::File::read_at` call on Tokio's blocking thread pool. This avoids
/// mutating the file handle's cursor, making it safe for concurrent use.
///
/// For other platforms, it falls back to using Tokio's async-native seek-and-read
/// operations.
///
/// # Parameters
///
/// * `path`: The path to the file to read from.
/// * `offset`: The starting position (in bytes) from the beginning of the file.
/// * `len`: The number of bytes to read.
///
/// # Returns
///
/// A `Result` containing a `Vec<u8>` with the bytes read. The vector's length
/// will be equal to `len` unless the read operation reached the end of the file.
///
/// # Errors
///
/// This function will return an `io::Error` if:
/// - The file cannot be opened or the underlying read operation fails.
/// - The blocking I/O task is cancelled.
///
/// # Panics
///
/// This function will panic if the blocking task responsible for file I/O panics
/// (e.g., due to an out-of-memory error when allocating the read buffer).
pub async fn async_read_byte_range(
    path: impl AsRef<Path>,
    offset: u64,
    len: usize,
) -> io::Result<Vec<u8>> {
    #[cfg(any(unix, windows))]
    {
        let path_buf = path.as_ref().to_path_buf();
        let result = tokio::task::spawn_blocking(move || {
            read_at_internal(path_buf, offset, len as u64, None::<&dyn Progress>)
        })
        .await;
        handle_blocking_io_task_result(result)
    }

    #[cfg(not(any(unix, windows)))]
    {
        seek_read_async_internal(path, offset, len as u64, None::<&dyn Progress>).await
    }
}

/// Reads a specific range of bytes from a file asynchronously with progress reporting.
///
/// This function provides a portable and efficient way to read a segment of a file
/// while reporting progress. To display progress, the file is read in chunks.
///
/// On supported platforms (Unix, Windows), it uses efficient `read_at` syscalls
/// on Tokio's blocking thread pool. For other platforms, it falls back to
/// Tokio's async-native seek-and-read operations.
///
/// # Parameters
///
/// * `path`: The path to the file to read from.
/// * `offset`: The starting position (in bytes) from the beginning of the file.
/// * `len`: The total number of bytes to read.
/// * `pb`: A progress tracking structure.
///
/// # Returns
///
/// A `Result` containing a `Vec<u8>` with the bytes read. The vector's length
/// will be equal to `len` unless the read operation reached the end of the file.
///
/// # Errors
///
/// This function will return an `io::Error` if:
/// - The file cannot be opened or the underlying read operation fails.
/// - `len` is too large to fit in memory (`> usize::MAX`).
/// - The sum of `offset` and `len` overflows a `u64`.
/// - The blocking I/O task is cancelled.
///
/// # Panics
///
/// This function will panic if the blocking task responsible for file I/O panics
/// (e.g., due to an out-of-memory error when allocating the read buffer).
pub async fn async_read_byte_range_with_progress(
    path: impl AsRef<Path>,
    offset: u64,
    len: u64,
    pb: impl Progress + Send + 'static,
) -> io::Result<Vec<u8>> {
    #[cfg(any(unix, windows))]
    {
        let path_buf = path.as_ref().to_path_buf();
        let result =
            tokio::task::spawn_blocking(move || read_at_internal(path_buf, offset, len, Some(&pb)))
                .await;
        handle_blocking_io_task_result(result)
    }

    #[cfg(not(any(unix, windows)))]
    {
        seek_read_async_internal(path, offset, len, Some(&pb)).await
    }
}

/// Reads a specific range of bytes from a file synchronously.
///
/// # Errors
///
/// This function will return an `io::Error` if:
/// - The file cannot be opened or the underlying read operation fails.
/// - The sum of `offset` and `len` overflows a `u64`.
pub fn read_byte_range(path: impl AsRef<Path>, offset: u64, len: usize) -> io::Result<Vec<u8>> {
    #[cfg(any(unix, windows))]
    {
        read_at_internal(path, offset, len as u64, None::<&dyn Progress>)
    }

    #[cfg(not(any(unix, windows)))]
    {
        seek_read_blocking_internal(path, offset, len as u64, None::<&dyn Progress>)
    }
}

/// Reads a specific range of bytes from a file synchronously with progress reporting.
///
/// # Errors
///
/// This function will return an `io::Error` if:
/// - The file cannot be opened or the underlying read operation fails.
/// - `len` is too large to fit in memory (`> usize::MAX`).
/// - The sum of `offset` and `len` overflows a `u64`.
pub fn read_byte_range_with_progress(
    path: impl AsRef<Path>,
    offset: u64,
    len: u64,
    ps: &impl Progress,
) -> io::Result<Vec<u8>> {
    #[cfg(any(unix, windows))]
    {
        read_at_internal(path, offset, len, Some(ps))
    }

    #[cfg(not(any(unix, windows)))]
    {
        seek_read_blocking_internal(path, offset, len, Some(ps))
    }
}

//================================================================================
// Internal Implementation Details
//================================================================================

/// Checks if a u64 length can safely be converted to usize for buffer allocation.
#[inline]
fn validate_len_for_buffer(len: u64) -> io::Result<usize> {
    len.try_into().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "length is too large for memory buffer",
        )
    })
}

/// Internal implementation using positional reads for POSIX-compliant systems and Windows.
#[cfg(any(unix, windows))]
pub fn read_at_internal(
    path: impl AsRef<Path>,
    offset: u64,
    len: u64,
    pb: Option<&(impl Progress + ?Sized)>,
) -> io::Result<Vec<u8>> {
    /// Compatibility helper for `read_at` on unix and `seek_read` on windows.
    #[inline]
    fn read_at_compat(file: &std::fs::File, buf: &mut [u8], offset: u64) -> io::Result<usize> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::FileExt as _;
            file.read_at(buf, offset)
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::FileExt as _;
            file.seek_read(buf, offset)
        }
    }

    let file = std::fs::File::open(path.as_ref())?;

    // Prevent silent data corruption from integer overflow when calculating read offset.
    if offset.checked_add(len).is_none() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "read range offset + length overflows a u64",
        ));
    }

    let capacity = validate_len_for_buffer(len)?;
    let mut buffer = vec![0; capacity];

    let mut total_bytes_read = 0;
    let read_result = loop {
        if total_bytes_read >= capacity {
            break Ok(());
        }
        let current_slice = &mut buffer[total_bytes_read..];
        let current_offset = offset + total_bytes_read as u64;

        match read_at_compat(&file, current_slice, current_offset) {
            Ok(0) => break Ok(()), // End of file reached.
            Ok(bytes_read) => {
                total_bytes_read += bytes_read;
                if let Some(pb) = pb {
                    pb.inc(bytes_read as u64);
                }
            }
            Err(e) => break Err(e),
        }
    };

    // Always finish the progress bar, even if an error occurred during read.
    if let Some(pb) = pb {
        pb.finish();
    }

    read_result?;

    buffer.truncate(total_bytes_read);
    Ok(buffer)
}

/// Macro to generate nearly identical sync and async `seek_read` functions.
#[cfg(not(any(unix, windows)))]
macro_rules! define_seek_read_internal {
    (
        $vis:vis,
        $name:ident,
        $doc:expr,
        $($async:ident)?,
        $($await:tt)?,
        $file:ty,
        $read_trait:path,
        $seek_trait:path
    ) => {
        #[doc = $doc]
        $vis $($async)? fn $name(
            path: impl AsRef<Path>,
            offset: u64,
            len: u64,
            pb: Option<&(impl Progress + ?Sized)>,
        ) -> io::Result<Vec<u8>> {
            use $read_trait;
            use $seek_trait;

            let capacity = validate_len_for_buffer(len)?;

            let mut file = <$file>::open(path)$(.$await)?;

            // The seek_read fallback does not need an explicit overflow check because
            // `seek` itself will return an `InvalidInput` error on overflow.
            file.seek(io::SeekFrom::Start(offset))$(.$await)?;

            if let Some(pb) = pb {
                // Progress reporting path: read in chunks.
                let mut reader = file.take(len);
                let mut buffer = Vec::with_capacity(capacity);
                // 64 KiB is a common and reasonably performant chunk size for I/O.
                let mut read_buf = vec![0; 64 * 1024];

                let result = loop {
                    match reader.read(&mut read_buf)$(.$await) {
                        Ok(0) => break Ok(buffer), // EOF
                        Ok(n) => {
                            buffer.extend_from_slice(&read_buf[..n]);
                            pb.inc(n as u64);
                        }
                        Err(ref e) if e.kind() == io::ErrorKind::Interrupted => continue,
                        Err(e) => break Err(e),
                    }
                };
                // Always finish the progress bar, even if an error occurred.
                pb.finish();
                result
            } else {
                // No progress reporting: read all bytes up to `len`.
                let mut reader = file.take(len);
                let mut buffer = Vec::with_capacity(capacity);
                reader.read_to_end(&mut buffer)$(.$await)?;
                Ok(buffer)
            }
        }
    };
}

#[cfg(not(any(unix, windows)))]
define_seek_read_internal!(
    , // private visibility
    seek_read_async_internal,
    "Internal async implementation using `seek` and `read` for other platforms.",
    async,
    await,
    tokio::fs::File,
    tokio::io::AsyncReadExt,
    tokio::io::AsyncSeekExt
);

#[cfg(not(any(unix, windows)))]
define_seek_read_internal!(
    pub, // public visibility
    seek_read_blocking_internal,
    "Internal blocking implementation using `seek` and `read` for other platforms.",
    , // no async
    , // no await
    std::fs::File,
    std::io::Read,
    std::io::Seek
);
