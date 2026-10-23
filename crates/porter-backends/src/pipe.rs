//! Streams over block-based remote I/O (SMB, NFS): a background task moves the data
//! through an in-memory pipe, and its outcome is reported through the stream, so a
//! failed transfer can never look like a short file.

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll, ready};

use tokio::io::{AsyncRead, AsyncWrite, DuplexStream, ReadBuf};
use tokio::task::JoinHandle;

/// Bytes buffered between the caller and the background task.
const PIPE_CAPACITY: usize = 4 << 20;

/// A reader fed by `produce`, which writes the data into the given pipe end.
pub fn reader<F, Fut>(produce: F) -> TaskReader
where
    F: FnOnce(DuplexStream) -> Fut,
    Fut: Future<Output = io::Result<()>> + Send + 'static,
{
    let (ours, theirs) = tokio::io::duplex(PIPE_CAPACITY);
    TaskReader {
        pipe: ours,
        task: Some(tokio::spawn(produce(theirs))),
    }
}

/// A writer drained by `consume`, which reads the data from the given pipe end.
/// `shutdown()` completes only once `consume` has finished, with its result.
pub fn writer<F, Fut>(consume: F) -> TaskWriter
where
    F: FnOnce(DuplexStream) -> Fut,
    Fut: Future<Output = io::Result<()>> + Send + 'static,
{
    let (ours, theirs) = tokio::io::duplex(PIPE_CAPACITY);
    TaskWriter {
        pipe: ours,
        task: Some(tokio::spawn(consume(theirs))),
    }
}

pub struct TaskReader {
    pipe: DuplexStream,
    task: Option<JoinHandle<io::Result<()>>>,
}

impl AsyncRead for TaskReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before = buf.filled().len();
        ready!(Pin::new(&mut self.pipe).poll_read(cx, buf))?;
        if buf.filled().len() > before {
            return Poll::Ready(Ok(()));
        }
        // End of the pipe: only a real end of file if the producer succeeded.
        match self.task.as_mut() {
            Some(task) => {
                let outcome = ready!(Pin::new(task).poll(cx));
                self.task = None;
                Poll::Ready(joined(outcome))
            }
            None => Poll::Ready(Ok(())),
        }
    }
}

pub struct TaskWriter {
    pipe: DuplexStream,
    task: Option<JoinHandle<io::Result<()>>>,
}

impl AsyncWrite for TaskWriter {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match Pin::new(&mut self.pipe).poll_write(cx, buf) {
            // The consumer stopped early; surface why.
            Poll::Ready(Err(e)) if e.kind() == io::ErrorKind::BrokenPipe => self.finish(cx),
            other => other,
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.pipe).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        ready!(Pin::new(&mut self.pipe).poll_shutdown(cx))?;
        match self.task.as_mut() {
            Some(task) => {
                let outcome = ready!(Pin::new(task).poll(cx));
                self.task = None;
                Poll::Ready(joined(outcome))
            }
            None => Poll::Ready(Ok(())),
        }
    }
}

impl TaskWriter {
    fn finish<T>(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<T>> {
        let Some(task) = self.task.as_mut() else {
            return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
        };
        let outcome = ready!(Pin::new(task).poll(cx));
        self.task = None;
        Poll::Ready(match joined(outcome) {
            Err(e) => Err(e),
            Ok(()) => Err(io::Error::other("remote writer stopped early")),
        })
    }
}

fn joined(outcome: Result<io::Result<()>, tokio::task::JoinError>) -> io::Result<()> {
    outcome.map_err(io::Error::other)?
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;

    #[tokio::test]
    async fn reader_passes_data_and_errors() {
        let mut ok = reader(|mut pipe| async move { pipe.write_all(b"hello").await });
        let mut data = String::new();
        ok.read_to_string(&mut data).await.unwrap();
        assert_eq!(data, "hello");

        let mut failing = reader(|mut pipe| async move {
            pipe.write_all(b"part").await?;
            Err(io::Error::other("connection reset"))
        });
        let mut data = Vec::new();
        let err = failing.read_to_end(&mut data).await.unwrap_err();
        assert_eq!(err.to_string(), "connection reset");
    }

    #[tokio::test]
    async fn writer_waits_for_consumer() {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let mut w = writer(|mut pipe| async move {
            let mut data = Vec::new();
            pipe.read_to_end(&mut data).await?;
            let _ = tx.send(data);
            Ok(())
        });
        w.write_all(b"payload").await.unwrap();
        w.shutdown().await.unwrap();
        assert_eq!(rx.await.unwrap(), b"payload");

        let mut failing = writer(|_pipe| async move { Err(io::Error::other("disk full")) });
        let err = async {
            failing.write_all(&vec![0; 8 << 20]).await?;
            failing.shutdown().await
        }
        .await
        .unwrap_err();
        assert_eq!(err.to_string(), "disk full");
    }
}
