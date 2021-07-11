use super::{Flags, Head, Kind, StreamId};
use async_std::io::{Result as AsyncIoResult, Write};
use async_std::prelude::*;

use std::fmt::{self, Debug};
use std::sync::Arc;

pub struct Continuation {
    /// Stream ID of continuation frame.
    stream_id: StreamId,
    /// Raw headers data.
    raw_data: Arc<dyn AsRef<[u8]> + Send + Sync>,
}

impl Continuation {
    pub fn new<T>(stream_id: StreamId, raw_data: T) -> Self
    where
        T: AsRef<[u8]> + Send + Sync + 'static,
    {
        Self {
            stream_id,
            raw_data: Arc::new(raw_data),
        }
    }

    /// Build a header for this frame.
    pub fn head(&self) -> Head {
        Head::new(
            Kind::Continuation,
            Flags::END_HEADERS.into(),
            self.stream_id,
        )
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

impl Debug for Continuation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut builder = f.debug_struct("Continuation");
        builder.field("stream_id", &self.stream_id);

        builder.finish()
    }
}
