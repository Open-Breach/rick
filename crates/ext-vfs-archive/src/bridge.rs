//! From the archive crates' synchronous readers to the `AsyncRead` the
//! provider hands out: the decompression runs on the blocking pool and
//! sends chunks over a bounded channel, so a slow reader holds it back.

use std::io::{self, Write};
use std::pin::Pin;
use std::task::{Context, Poll};

use ext::ReadHandle;
use tokio::io::{AsyncRead, ReadBuf};
use tokio::sync::mpsc;

/// Bytes per chunk sent over the channel.
const CHUNK: usize = 64 * 1024;
/// Chunks buffered ahead of the reader.
const DEPTH: usize = 8;

/// A writer that sends what it is given down the channel, in chunks.
struct ChannelWriter {
    tx: mpsc::Sender<io::Result<Vec<u8>>>,
    pending: Vec<u8>,
}

impl ChannelWriter {
    fn send(&mut self, chunk: Vec<u8>) -> io::Result<()> {
        self.tx
            .blocking_send(Ok(chunk))
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "the reader went away"))
    }
}

impl Write for ChannelWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.pending.extend_from_slice(buf);
        if self.pending.len() >= CHUNK {
            let chunk = std::mem::take(&mut self.pending);
            self.send(chunk)?;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if !self.pending.is_empty() {
            let chunk = std::mem::take(&mut self.pending);
            self.send(chunk)?;
        }
        Ok(())
    }
}

/// The reading end: chunks from the channel, handed out as asked.
struct ChannelReader {
    rx: mpsc::Receiver<io::Result<Vec<u8>>>,
    current: Vec<u8>,
    offset: usize,
    done: bool,
}

impl AsyncRead for ChannelReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        loop {
            if self.offset < self.current.len() {
                let take = (self.current.len() - self.offset).min(buf.remaining());
                let start = self.offset;
                buf.put_slice(&self.current[start..start + take]);
                self.offset += take;
                return Poll::Ready(Ok(()));
            }
            if self.done {
                return Poll::Ready(Ok(()));
            }
            match self.rx.poll_recv(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(None) => {
                    self.done = true;
                    return Poll::Ready(Ok(()));
                }
                Poll::Ready(Some(Err(error))) => {
                    self.done = true;
                    return Poll::Ready(Err(error));
                }
                Poll::Ready(Some(Ok(chunk))) => {
                    self.current = chunk;
                    self.offset = 0;
                }
            }
        }
    }
}

/// Runs `produce` on the blocking pool with a writer whose bytes arrive at
/// the returned reader; an error from `produce` ends the reader with it.
/// Dropping the reader stops `produce` at its next write.
pub fn reader(
    produce: impl FnOnce(&mut dyn Write) -> io::Result<()> + Send + 'static,
) -> ReadHandle {
    let (tx, rx) = mpsc::channel(DEPTH);
    let failures = tx.clone();
    ext::runtime::handle().spawn_blocking(move || {
        let mut writer = ChannelWriter {
            tx,
            pending: Vec::with_capacity(CHUNK),
        };
        let outcome = produce(&mut writer).and_then(|()| writer.flush());
        if let Err(error) = outcome
            && error.kind() != io::ErrorKind::BrokenPipe
        {
            let _ = failures.blocking_send(Err(error));
        }
    });
    Box::new(ChannelReader {
        rx,
        current: Vec::new(),
        offset: 0,
        done: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt as _;

    #[test]
    fn bytes_cross_the_bridge_in_order_and_errors_arrive_last() {
        let text: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        let expected = text.clone();
        let mut handle = reader(move |sink| sink.write_all(&text));
        let out = runtime::block_on(async {
            let mut out = Vec::new();
            handle
                .read_to_end(&mut out)
                .await
                .unwrap_or_else(|e| panic!("{e}"));
            out
        });
        assert_eq!(out, expected);

        let mut failing = reader(|sink| {
            sink.write_all(b"partial")?;
            Err(io::Error::other("boom"))
        });
        let result = runtime::block_on(async {
            let mut out = Vec::new();
            failing.read_to_end(&mut out).await
        });
        assert!(result.is_err());
    }
}
