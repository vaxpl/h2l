use crate::frame::{Frame, Head, Kind, StreamId};
use async_std::io::{Result as AsyncIoResult, Write};
use async_std::prelude::*;
use bitflags::bitflags;

use std::fmt;
use std::sync::Arc;

/// Bincode encoded frame.
pub struct BinCode {
    stream_id: StreamId,
    flags: BinCodeFlags,
    data: Arc<dyn AsRef<[u8]> + Send + Sync>,
}

impl BinCode {
    /// Construct a new BinCode frame.
    pub fn new<T>(stream_id: StreamId, payload: T) -> Self
    where
        T: AsRef<[u8]> + Send + Sync + 'static,
    {
        Self {
            stream_id,
            flags: BinCodeFlags::empty(),
            data: Arc::new(payload),
        }
    }

    /// Returns the stream identifier that this frame is associated with.
    ///
    /// This cannot be a zero stream identifier.
    pub fn stream_id(&self) -> StreamId {
        self.stream_id
    }

    /// Returns the flags that this frame is associated with.
    pub fn flags(&self) -> BinCodeFlags {
        self.flags
    }

    /// Returns a reference to this frame's payload.
    ///
    /// This does **not** include any padding that might have been originally
    /// included.
    pub fn payload(&self) -> &Arc<dyn AsRef<[u8]> + Send + Sync> {
        &self.data
    }

    /// Returns a mutable reference to this frame's payload.
    ///
    /// This does **not** include any padding that might have been originally
    /// included.
    pub fn payload_mut(&mut self) -> &mut Arc<dyn AsRef<[u8]> + Send + Sync> {
        &mut self.data
    }

    /// Consumes `self` and returns the frame's payload.
    ///
    /// This does **not** include any padding that might have been originally
    /// included.
    pub fn into_payload(self) -> Arc<dyn AsRef<[u8]> + Send + Sync> {
        self.data
    }

    /// Build a header for this frame.
    pub(crate) fn head(&self) -> Head {
        Head::new(Kind::BinCode, self.flags.into(), self.stream_id)
    }

    /// Converts the payload to a byte slice.
    pub fn as_bytes(&self) -> &[u8] {
        (*self.data).as_ref()
    }

    /// Returns true if payload has a length of zero bytes.
    pub fn is_empty(&self) -> bool {
        self.as_bytes().is_empty()
    }

    /// Returns the length of payload size.
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

impl fmt::Debug for BinCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut builder = f.debug_struct("BinCode");
        builder.field("stream_id", &self.stream_id);
        builder.field("flags", &self.flags);
        builder.finish()
    }
}

impl From<BinCode> for Frame {
    fn from(val: BinCode) -> Self {
        Frame::BinCode(val)
    }
}

bitflags! {
    /// The Flags of the BinCode frame.
    pub struct BinCodeFlags: u8 {
        const ACK = 0x01;
    }
}

impl From<BinCodeFlags> for u8 {
    fn from(val: BinCodeFlags) -> u8 {
        val.bits
    }
}
