use super::{Error, Frame, Head, Kind, Reason, StreamId};
use async_std::io::{Result as AsyncIoResult, Write};
use async_std::prelude::*;
use bytes::{BufMut, BytesMut};

#[derive(Debug, PartialEq, Eq)]
pub struct Reset {
    stream_id: StreamId,
    error_code: Reason,
}

impl Reset {
    pub fn new(stream_id: StreamId, error: Reason) -> Reset {
        Reset {
            stream_id,
            error_code: error,
        }
    }

    pub fn stream_id(&self) -> StreamId {
        self.stream_id
    }

    pub fn reason(&self) -> Reason {
        self.error_code
    }

    pub fn load(head: Head, payload: &[u8]) -> Result<Reset, Error> {
        if payload.len() != 4 {
            return Err(Error::InvalidPayloadLength);
        }

        let error_code = unpack_octets_4!(payload, 0, u32);

        Ok(Reset {
            stream_id: head.stream_id(),
            error_code: error_code.into(),
        })
    }

    pub fn encode<T: BufMut>(&self, dst: &mut T) {
        let head = Head::new(Kind::Reset, 0, self.stream_id);
        head.encode(4, dst);
        dst.put_u32(self.error_code.into());
    }

    /// Write encoded raw bytes to specified `writer`.
    pub(crate) async fn write_to<W: Write + Unpin>(&self, writer: &mut W) -> AsyncIoResult<()> {
        let mut buffer = BytesMut::with_capacity(64);
        self.encode(&mut buffer);
        writer.write_all(&buffer).await
    }
}

impl From<Reset> for Frame {
    fn from(src: Reset) -> Self {
        Self::Reset(src)
    }
}
