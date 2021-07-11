use super::{Error, Frame, Head, Kind, Reason, StreamId};
use async_std::io::{Result as AsyncIoResult, Write};
use async_std::prelude::*;
use bytes::{BufMut, BytesMut};
use log::trace;

use std::fmt;

/// Go away frame.
#[derive(Clone, Eq, PartialEq)]
pub struct GoAway {
    last_stream_id: StreamId,
    error_code: Reason,
}

impl GoAway {
    pub fn new(last_stream_id: StreamId, reason: Reason) -> Self {
        GoAway {
            last_stream_id,
            error_code: reason,
        }
    }

    pub fn last_stream_id(&self) -> StreamId {
        self.last_stream_id
    }

    pub fn reason(&self) -> Reason {
        self.error_code
    }

    pub fn load(payload: &[u8]) -> Result<GoAway, Error> {
        if payload.len() < 8 {
            return Err(Error::BadFrameSize);
        }

        let (last_stream_id, _) = StreamId::parse(&payload[..4]);
        let error_code = unpack_octets_4!(payload, 4, u32);

        Ok(GoAway {
            last_stream_id,
            error_code: error_code.into(),
        })
    }

    pub fn encode<B: BufMut>(&self, dst: &mut B) {
        trace!("encoding GO_AWAY; code={:?}", self.error_code);
        let head = Head::new(Kind::GoAway, 0, StreamId::zero());
        head.encode(8, dst);
        dst.put_u32(self.last_stream_id.into());
        dst.put_u32(self.error_code.into());
    }

    /// Write encoded raw bytes to specified `writer`.
    pub(crate) async fn write_to<W: Write + Unpin>(&self, writer: &mut W) -> AsyncIoResult<()> {
        let mut buffer = BytesMut::with_capacity(64);
        self.encode(&mut buffer);
        writer.write_all(&buffer).await
    }
}

impl fmt::Debug for GoAway {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut builder = f.debug_struct("GoAway");
        builder.field("error_code", &self.error_code);
        builder.field("last_stream_id", &self.last_stream_id);

        builder.finish()
    }
}

impl From<GoAway> for Frame {
    fn from(src: GoAway) -> Self {
        Frame::GoAway(src)
    }
}
