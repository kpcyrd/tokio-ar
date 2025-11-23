use crate::header::Header;
use std::cmp;
use std::io::{Error, ErrorKind, SeekFrom};
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncSeek, ReadBuf};

/// Representation of an archive entry.
///
/// `Entry` objects implement the `AsyncRead` trait, and can be used to extract the
/// data from this archive entry.  If the underlying reader supports the `AsyncSeek`
/// trait, then the `Entry` object supports `AsyncSeek` as well.
pub struct Entry<'a, R: 'a + AsyncRead + Unpin> {
    pub(crate) header: &'a Header,
    pub(crate) reader: &'a mut R,
    pub(crate) length: u64,
    pub(crate) position: u64,
    pub(crate) unread_counter: &'a mut u64,
}

impl<'a, R: 'a + AsyncRead + Unpin> Entry<'a, R> {
    /// Returns the header for this archive entry.
    pub fn header(&self) -> &Header {
        self.header
    }
}

impl<'a, R: 'a + AsyncRead + Unpin> AsyncRead for Entry<'a, R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        debug_assert!(self.position <= self.length);
        let remaining = self.length.saturating_sub(self.position);

        if remaining == 0 {
            return Poll::Ready(Ok(()));
        }

        let max_len = cmp::min(remaining, buf.remaining() as u64);

        // Remember the initial filled length
        let filled_before = buf.filled().len() as u64;

        match Pin::new(&mut self.reader.take(max_len)).poll_read(cx, buf) {
            Poll::Ready(Ok(())) => {
                // Calculate how many bytes were read
                let filled_after = buf.filled().len() as u64;
                let bytes_read = filled_after - filled_before;

                // Update position and unread counter
                self.position += bytes_read;
                *self.unread_counter -= bytes_read;
                debug_assert!(self.position <= self.length);
                Poll::Ready(Ok(()))
            }
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl<'a, R: 'a + AsyncRead + AsyncSeek + Unpin> AsyncSeek for Entry<'a, R> {
    fn start_seek(
        mut self: Pin<&mut Self>,
        pos: SeekFrom,
    ) -> std::io::Result<()> {
        let delta = match pos {
            SeekFrom::Start(offset) => offset as i64 - self.position as i64,
            SeekFrom::End(offset) => {
                self.length as i64 + offset - self.position as i64
            }
            SeekFrom::Current(delta) => delta,
        };
        let new_position = self.position as i64 + delta;
        if new_position < 0 {
            let msg = format!(
                "Invalid seek to negative position ({})",
                new_position
            );
            return Err(Error::new(ErrorKind::InvalidInput, msg));
        }
        let new_position = new_position as u64;
        if new_position > self.length {
            let msg = format!(
                "Invalid seek to position past end of entry ({} vs. {})",
                new_position, self.length
            );
            return Err(Error::new(ErrorKind::InvalidInput, msg));
        }
        Pin::new(&mut self.reader).start_seek(SeekFrom::Current(delta))?;
        self.position = new_position;
        *self.unread_counter = self.length - self.position;
        Ok(())
    }

    fn poll_complete(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<std::io::Result<u64>> {
        match Pin::new(&mut self.reader).poll_complete(cx) {
            Poll::Ready(result) => Poll::Ready(result.map(|_| self.position)),
            Poll::Pending => Poll::Pending,
        }
    }
}
