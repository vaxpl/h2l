use crate::frame::{Error, Frame, Head, Kind, StreamId};
use async_std::io::{Result as AsyncIoResult, Write};
use async_std::prelude::*;
use bytes::{BufMut, BytesMut};

const SIZE_INCREMENT_MASK: u32 = 1 << 31;

/// Window update frame.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct WindowUpdate {
    stream_id: StreamId,
    size_increment: u32,
}

impl WindowUpdate {
    pub fn new(stream_id: StreamId, size_increment: u32) -> WindowUpdate {
        WindowUpdate {
            stream_id,
            size_increment,
        }
    }

    pub fn stream_id(&self) -> StreamId {
        self.stream_id
    }

    pub fn size_increment(&self) -> u32 {
        self.size_increment
    }

    /// Builds a `WindowUpdate` frame from a raw frame.
    pub fn load(head: Head, payload: &[u8]) -> Result<WindowUpdate, Error> {
        debug_assert_eq!(head.kind(), super::Kind::WindowUpdate);
        if payload.len() != 4 {
            return Err(Error::BadFrameSize);
        }

        // Clear the most significant bit, as that is reserved and MUST be ignored
        // when received.
        let size_increment = unpack_octets_4!(payload, 0, u32) & !SIZE_INCREMENT_MASK;

        if size_increment == 0 {
            return Err(Error::InvalidWindowUpdateValue);
        }

        Ok(WindowUpdate {
            stream_id: head.stream_id(),
            size_increment,
        })
    }

    /// Encoding to raw bytes.
    pub fn encode<B: BufMut>(&self, dst: &mut B) {
        let head = Head::new(Kind::WindowUpdate, 0, self.stream_id);
        head.encode(4, dst);
        dst.put_u32(self.size_increment);
    }

    /// Write encoded raw bytes to specified `writer`.
    pub(crate) async fn write_to<W: Write + Unpin>(&self, writer: &mut W) -> AsyncIoResult<()> {
        let mut buffer = BytesMut::with_capacity(64);
        self.encode(&mut buffer);
        writer.write_all(&buffer).await
    }
}

impl From<WindowUpdate> for Frame {
    fn from(src: WindowUpdate) -> Self {
        Self::WindowUpdate(src)
    }
}
