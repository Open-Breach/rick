//! The SSH agent's socket: `SSH_AUTH_SOCK`, a Unix domain socket, on Linux
//! and macOS; the OpenSSH agent's named pipe, `\\.\pipe\openssh-ssh-agent`,
//! on Windows. The SSH plugin speaks the agent protocol over the stream
//! this module opens, so the one `cfg` stays here.

use std::io;
use std::path::Path;

use tokio::io::{AsyncRead, AsyncWrite};

/// A byte stream to the agent, readable and writable.
pub trait AgentIo: AsyncRead + AsyncWrite + Send + Unpin {}

impl<T: AsyncRead + AsyncWrite + Send + Unpin> AgentIo for T {}

/// The stream to the agent, boxed so one type serves every platform.
pub type AgentStream = Box<dyn AgentIo>;

/// The agent's default endpoint on this platform: `SSH_AUTH_SOCK` where
/// the environment names one, the OpenSSH named pipe on Windows; `None`
/// when there is none to try.
#[must_use]
pub fn default_endpoint() -> Option<std::path::PathBuf> {
    #[cfg(windows)]
    {
        Some(std::path::PathBuf::from(r"\\.\pipe\openssh-ssh-agent"))
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("SSH_AUTH_SOCK")
            .filter(|s| !s.is_empty())
            .map(std::path::PathBuf::from)
    }
}

/// Opens the agent at `endpoint`, or at [`default_endpoint`] when `None`.
///
/// # Errors
///
/// `NotFound` when no endpoint is known or nothing listens there;
/// otherwise the I/O error of opening it.
pub async fn connect(endpoint: Option<&Path>) -> io::Result<AgentStream> {
    let endpoint = match endpoint {
        Some(path) => path.to_path_buf(),
        None => default_endpoint().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "no SSH agent: SSH_AUTH_SOCK is not set",
            )
        })?,
    };
    open(&endpoint).await
}

#[cfg(unix)]
async fn open(endpoint: &Path) -> io::Result<AgentStream> {
    let stream = tokio::net::UnixStream::connect(endpoint).await?;
    Ok(Box::new(stream))
}

#[cfg(windows)]
async fn open(endpoint: &Path) -> io::Result<AgentStream> {
    /// The pipe exists but every instance is busy: wait and try again.
    const ERROR_PIPE_BUSY: i32 = 231;
    loop {
        match tokio::net::windows::named_pipe::ClientOptions::new().open(endpoint) {
            Ok(client) => return Ok(Box::new(client)),
            Err(error) if error.raw_os_error() == Some(ERROR_PIPE_BUSY) => {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            Err(error) => return Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_agent_is_not_found() {
        let result = runtime::block_on(connect(Some(Path::new(
            "/nonexistent/rick-agent-socket-for-the-test",
        ))));
        let kind = result.err().map(|e| e.kind());
        assert_eq!(kind, Some(io::ErrorKind::NotFound));
    }
}
