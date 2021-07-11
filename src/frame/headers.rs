use super::{Flags, Frame, Head, Kind, StreamId};
use async_std::io::{Result as AsyncIoResult, Write};
use async_std::prelude::*;

use std::fmt;
use std::sync::Arc;

/// Header frame
///
/// This could be either a request or a response.
pub struct Headers {
    /// The ID of the stream with which this frame is associated.
    stream_id: StreamId,
    /// The associated flags.
    flags: Flags,
    /// Raw headers data.
    raw_data: Arc<dyn AsRef<[u8]> + Send + Sync>,
}

impl Headers {
    /// Create a new HEADERS frame
    pub fn new<T>(stream_id: StreamId, raw_data: T) -> Self
    where
        T: AsRef<[u8]> + Send + Sync + 'static,
    {
        Headers {
            stream_id,
            flags: Flags::empty(),
            raw_data: Arc::new(raw_data),
        }
    }

    /// Returns the stream identifier that this frame is associated with.
    ///
    /// This cannot be a zero stream identifier.
    pub fn stream_id(&self) -> StreamId {
        self.stream_id
    }

    /// Build a header for this frame.
    pub fn head(&self) -> Head {
        Head::new(Kind::Headers, self.flags.into(), self.stream_id)
    }

    /// Converts the raw data to a byte slice.
    pub fn as_bytes(&self) -> &[u8] {
        (*self.raw_data).as_ref()
    }

    /// Returns true if raw data has a length of zero bytes.
    pub fn is_empty(&self) -> bool {
        self.as_bytes().is_empty()
    }

    /// Returns the length of raw data size.
    pub fn len(&self) -> usize {
        self.as_bytes().len()
    }

    /// Write encoded raw bytes to specified `writer`.
    pub(crate) async fn write_to<W: Write + Unpin>(&self, writer: &mut W) -> AsyncIoResult<()> {
        let data = self.as_bytes();
        self.head().write_to(data.len(), writer).await?;
        writer.write_all(data).await?;
        Ok(())
    }
}

impl fmt::Debug for Headers {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut builder = f.debug_struct("Headers");
        builder.field("stream_id", &self.stream_id);

        // `fields` and `pseudo` purposefully not included
        builder.finish()
    }
}

impl From<Headers> for Frame {
    fn from(src: Headers) -> Self {
        Frame::Headers(src)
    }
}
