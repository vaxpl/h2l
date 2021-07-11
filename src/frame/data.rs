use super::{Error, Frame, Head, Kind, StreamId};
use async_std::io::{Result as AsyncIoResult, Write};
use async_std::prelude::*;
use bitflags::bitflags;

use std::fmt;
use std::sync::Arc;

/// Data frame
///
/// Data frames convey arbitrary, variable-length sequences of octets associated
/// with a stream. One or more DATA frames are used, for instance, to carry HTTP
/// request or response payloads.
pub struct Data {
    stream_id: StreamId,
    data: Arc<dyn AsRef<[u8]> + Send + Sync>,
    flags: DataFlags,
    pad_len: Option<u8>,
}

impl Data {
    /// Creates a new DATA frame.
    pub fn new<T>(stream_id: StreamId, payload: T) -> Self
    where
        T: AsRef<[u8]> + Send + Sync + 'static,
    {
        assert!(!stream_id.is_zero());

        Data {
            stream_id,
            data: Arc::new(payload),
            flags: DataFlags::default(),
            pad_len: None,
        }
    }

    fn check_padding(payload: &[u8]) -> Result<u8, Error> {
        let payload_len = payload.len();
        if payload_len == 0 {
            // If this is the case, the frame is invalid as no padding length can be
            // extracted, even though the frame should be padded.
            return Err(Error::TooMuchPadding);
        }

        let pad_len = payload[0] as usize;

        if pad_len >= payload_len {
            // This is invalid: the padding length MUST be less than the
            // total frame size.
            return Err(Error::TooMuchPadding);
        }

        Ok(pad_len as u8)
    }

    pub(crate) fn load<T>(head: Head, payload: T) -> Result<Self, Error>
    where
        T: AsRef<[u8]> + Send + Sync + 'static,
    {
        let flags = DataFlags::load(head.flags());

        // The stream identifier must not be zero
        if head.stream_id().is_zero() {
            return Err(Error::InvalidStreamId);
        }

        let pad_len = if flags.is_padded() {
            let data: &[u8] = payload.as_ref();
            let len = Self::check_padding(data)?;
            Some(len)
        } else {
            None
        };

        Ok(Data {
            stream_id: head.stream_id(),
            data: Arc::new(payload),
            flags,
            pad_len,
        })
    }

    /// Returns the stream identifier that this frame is associated with.
    ///
    /// This cannot be a zero stream identifier.
    pub fn stream_id(&self) -> StreamId {
        self.stream_id
    }

    /// Gets the value of the `END_STREAM` flag for this frame.
    ///
    /// If true, this frame is the last that the endpoint will send for the
    /// identified stream.
    ///
    /// Setting this flag causes the stream to enter one of the "half-closed"
    /// states or the "closed" state (Section 5.1).
    pub fn is_end_stream(&self) -> bool {
        self.flags.is_end_stream()
    }

    /// Sets the value for the `END_STREAM` flag on this frame.
    pub fn set_end_stream(&mut self, val: bool) {
        if val {
            self.flags.set_end_stream();
        } else {
            self.flags.unset_end_stream();
        }
    }

    /// Returns whether the `PADDED` flag is set on this frame.
    #[cfg(feature = "unstable")]
    pub fn is_padded(&self) -> bool {
        self.flags.is_padded()
    }

    /// Sets the value for the `PADDED` flag on this frame.
    #[cfg(feature = "unstable")]
    pub fn set_padded(&mut self) {
        self.flags.set_padded();
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
        Head::new(Kind::Data, self.flags.into(), self.stream_id)
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

impl fmt::Debug for Data {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        let mut f = fmt.debug_struct("Data");
        f.field("stream_id", &self.stream_id);
        if !self.flags.is_empty() {
            f.field("flags", &self.flags);
        }
        if let Some(ref pad_len) = self.pad_len {
            f.field("pad_len", pad_len);
        }
        // `data` bytes purposefully excluded
        f.finish()
    }
}

impl From<Data> for Frame {
    fn from(src: Data) -> Self {
        Frame::Data(src)
    }
}

bitflags! {
    struct DataFlags: u8 {
        const END_STREAM = 0x1;
        const PADDED = 0x8;
        const ALL = DataFlags::END_STREAM.bits | DataFlags::PADDED.bits;
    }
}

impl DataFlags {
    fn load(bits: u8) -> DataFlags {
        DataFlags::from_bits_truncate(bits)
    }

    fn is_end_stream(&self) -> bool {
        self.intersects(DataFlags::END_STREAM)
    }

    fn set_end_stream(&mut self) {
        self.insert(DataFlags::END_STREAM);
    }

    fn unset_end_stream(&mut self) {
        self.remove(DataFlags::END_STREAM)
    }

    fn is_padded(&self) -> bool {
        self.intersects(DataFlags::PADDED)
    }

    #[cfg(feature = "unstable")]
    fn set_padded(&mut self) {
        self.insert(DataFlags::PADDED);
    }
}

impl Default for DataFlags {
    fn default() -> Self {
        DataFlags::empty()
    }
}

impl From<DataFlags> for u8 {
    fn from(src: DataFlags) -> u8 {
        src.bits
    }
}
