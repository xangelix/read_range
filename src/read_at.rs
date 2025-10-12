use std::{io, path::Path};

use super::Progress;

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
/// may be less than `len` if the read operation reached the end of the file.
///
/// # Errors
///
/// This function will return an `io::Error` if:
/// - The file cannot be opened.
/// - The underlying read operation fails.
/// - A blocking task panics or is cancelled when offloading to a thread pool.
pub async fn async_read_byte_range(
    path: impl AsRef<Path>,
    offset: u64,
    len: usize,
) -> io::Result<Vec<u8>> {
    #[cfg(any(unix, windows))]
    {
        let path_buf = path.as_ref().to_path_buf();
        tokio::task::spawn_blocking(move || {
            read_at_internal(path_buf, offset, len as u64, None::<&dyn Progress>)
        })
        .await
        .map_err(io::Error::other)?
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
/// * `ps`: A reference to the progress tracking structure.
///
/// # Returns
///
/// A `Result` containing a `Vec<u8>` with the bytes read. The vector's length
/// will be less than `len` if the read operation reached the end of the file.
///
/// # Errors
///
/// This function will return an `io::Error` if:
/// - The file cannot be opened.
/// - The underlying read operation fails.
/// - `len` is too large to fit in memory (`> usize::MAX`).
/// - A blocking task panics or is cancelled.
pub async fn async_read_byte_range_with_progress(
    path: impl AsRef<Path>,
    offset: u64,
    len: u64,
    pb: impl Progress + Send + 'static,
) -> io::Result<Vec<u8>> {
    #[cfg(any(unix, windows))]
    {
        let path_buf = path.as_ref().to_path_buf();
        tokio::task::spawn_blocking(move || read_at_internal(path_buf, offset, len, Some(&pb)))
            .await
            .map_err(io::Error::other)?
    }

    #[cfg(not(any(unix, windows)))]
    {
        seek_read_async_internal(path, offset, len, Some(&pb)).await
    }
}

/// Reads a specific range of bytes from a file synchronously.
///
/// This function provides a portable and efficient way to read a segment of a file.
/// On supported platforms (Unix, Windows), it uses the most efficient underlying
/// OS syscall (`pread` or `ReadFileScatter`). This avoids mutating the file
/// handle's cursor, making it safe for concurrent use.
///
/// For other platforms, it falls back to standard seek-and-read operations.
///
/// **Note:** This is a blocking operation and will block the current thread.
/// Do not call it from within an asynchronous context without offloading it to a
/// blocking-aware thread pool (like `tokio::task::spawn_blocking`).
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
/// may be less than `len` if the read operation reached the end of the file.
///
/// # Errors
///
/// This function will return an `io::Error` if the file cannot be opened or the
/// underlying read operation fails.
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
/// This function provides a portable and efficient way to read a segment of a file
/// while reporting progress. To display progress, the file is read in chunks.
///
/// On supported platforms (Unix, Windows), it uses efficient `read_at` syscalls.
/// For other platforms, it falls back to standard seek-and-read operations.
///
/// **Note:** This is a blocking operation and will block the current thread.
///
/// # Parameters
///
/// * `path`: The path to the file to read from.
/// * `offset`: The starting position (in bytes) from the beginning of the file.
/// * `len`: The total number of bytes to read.
/// * `ps`: A reference to the progress tracking structure.
///
/// # Returns
///
/// A `Result` containing a `Vec<u8>` with the bytes read. The vector's length
/// will be less than `len` if the read operation reached the end of the file.
///
/// # Errors
///
/// This function will return an `io::Error` if:
/// - The file cannot be opened.
/// - The underlying read operation fails.
/// - `len` is too large to fit in memory (`> usize::MAX`).
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

/// Internal implementation using `read_at` for POSIX-compliant systems and Windows.
/// This is a blocking operation.
#[cfg(any(unix, windows))]
pub fn read_at_internal(
    path: impl AsRef<Path>,
    offset: u64,
    len: u64,
    pb: Option<&(impl Progress + ?Sized)>,
) -> io::Result<Vec<u8>> {
    let file = std::fs::File::open(path.as_ref())?;
    let capacity: usize = len.try_into().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "length is too large for memory buffer",
        )
    })?;

    let mut buffer = vec![0; capacity];
    if let Some(pb) = pb {
        let mut total_bytes_read = 0;

        while total_bytes_read < capacity {
            let current_slice = &mut buffer[total_bytes_read..];
            let current_offset = offset + total_bytes_read as u64;

            let bytes_read = {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::FileExt as _;
                    file.read_at(current_slice, current_offset)?
                }
                #[cfg(windows)]
                {
                    use std::os::windows::fs::FileExt as _;
                    file.seek_read(current_slice, current_offset)?
                }
            };

            if bytes_read == 0 {
                // End of file reached.
                break;
            }
            total_bytes_read += bytes_read;
            pb.inc(bytes_read as u64);
        }

        buffer.truncate(total_bytes_read);
        pb.finish();
    } else {
        // No progress reporting: perform a single, more efficient read.
        let bytes_read = {
            #[cfg(unix)]
            {
                use std::os::unix::fs::FileExt as _;
                file.read_at(&mut buffer, offset)?
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::FileExt as _;
                file.seek_read(&mut buffer, offset)?
            }
        };
        buffer.truncate(bytes_read);
    }
    Ok(buffer)
}

/// Internal async implementation using `seek` and `read` for other platforms.
#[cfg(not(any(unix, windows)))]
async fn seek_read_async_internal(
    path: impl AsRef<Path>,
    offset: u64,
    len: u64,
    pb: Option<&(impl Progress + ?Sized)>,
) -> io::Result<Vec<u8>> {
    use tokio::{
        fs::File,
        io::{AsyncReadExt, AsyncSeekExt},
    };

    let capacity: usize = len.try_into().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "length is too large for memory buffer",
        )
    })?;

    let mut file = File::open(path).await?;
    file.seek(io::SeekFrom::Start(offset)).await?;

    if let Some(pb) = pb {
        // Progress reporting path.
        let mut reader = file.take(len);
        let mut buffer = Vec::with_capacity(capacity);
        let mut read_buf = vec![0; 64 * 1024]; // 64 KiB chunk

        loop {
            match reader.read(&mut read_buf).await {
                Ok(0) => break, // EOF
                Ok(n) => {
                    buffer.extend_from_slice(&read_buf[..n]);
                    pb.inc(n as u64);
                }
                Err(ref e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
        pb.finish();
        Ok(buffer)
    } else {
        // No progress reporting.
        let mut buffer = vec![0; capacity];
        let bytes_read = file.read(&mut buffer).await?;
        buffer.truncate(bytes_read);
        Ok(buffer)
    }
}

/// Internal blocking implementation using `seek` and `read` for other platforms.
#[cfg(not(any(unix, windows)))]
pub fn seek_read_blocking_internal(
    path: impl AsRef<Path>,
    offset: u64,
    len: u64,
    pb: Option<&(impl Progress + ?Sized)>,
) -> io::Result<Vec<u8>> {
    use std::io::{Read, Seek};

    let capacity: usize = len.try_into().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "length is too large for memory buffer",
        )
    })?;

    let mut file = std::fs::File::open(path)?;
    file.seek(io::SeekFrom::Start(offset))?;

    if let Some(pb) = pb {
        // Progress reporting path.
        let mut reader = file.take(len);
        let mut buffer = Vec::with_capacity(capacity);
        let mut read_buf = vec![0; 64 * 1024]; // 64 KiB chunk

        loop {
            match reader.read(&mut read_buf) {
                Ok(0) => break, // EOF
                Ok(n) => {
                    buffer.extend_from_slice(&read_buf[..n]);
                    pb.inc(n as u64);
                }
                Err(ref e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
        pb.finish();
        Ok(buffer)
    } else {
        // No progress reporting.
        let mut reader = file.take(len);
        let mut buffer = vec![0; capacity];
        // A single read may not fill the buffer, but we preserve the original's
        // behavior of performing one read attempt.
        let bytes_read = reader.read(&mut buffer)?;
        buffer.truncate(bytes_read);
        Ok(buffer)
    }
}
