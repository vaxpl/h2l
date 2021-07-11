use super::StreamId;
use async_std::io::{Result as AsyncIoResult, Write};
use async_std::prelude::*;
use bitflags::bitflags;
use bytes::{BufMut, BytesMut};

bitflags! {
    pub struct Flags: u8 {
        const END_STREAM = 0x1;
        const END_HEADERS = 0x4;
        const PADDED = 0x8;
        const PRIORITY = 0x20;
        const ALL = Self::END_STREAM.bits | Self::END_HEADERS.bits | Self::PADDED.bits | Self::PRIORITY.bits;
    }
}

impl From<Flags> for u8 {
    fn from(val: Flags) -> Self {
        val.bits
    }
}

/// Frame header.
#[derive(Copy, Clone, Debug)]
pub struct Head {
    pub kind: Kind,
    pub flags: u8,
    pub stream_id: StreamId,
}

impl Head {
    /// Construct a new frame header.
    pub fn new(kind: Kind, flags: u8, stream_id: StreamId) -> Head {
        Head {
            kind,
            flags,
            stream_id,
        }
    }

    /// Parse an HTTP/2.0 frame header
    pub fn parse(header: &[u8]) -> Head {
        let (stream_id, _) = StreamId::parse(&header[5..]);

        Head {
            kind: Kind::new(header[3]),
            flags: header[4],
            stream_id,
        }
    }

    pub fn stream_id(&self) -> StreamId {
        self.stream_id
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }

    pub fn flags(&self) -> u8 {
        self.flags
    }

    pub fn encode_len(&self) -> usize {
        super::consts::HEADER_LEN
    }

    pub(crate) fn encode<T: BufMut>(&self, payload_len: usize, dst: &mut T) {
        debug_assert!(self.encode_len() <= dst.remaining_mut());

        dst.put_uint(payload_len as u64, 3);
        dst.put_u8(self.kind as u8);
        dst.put_u8(self.flags);
        dst.put_u32(self.stream_id.into());
    }

    /// Write encoded raw bytes to specified `writer`.
    pub(crate) async fn write_to<W: Write + Unpin>(
        &self,
        payload_len: usize,
        writer: &mut W,
    ) -> AsyncIoResult<()> {
        let mut buffer = BytesMut::with_capacity(64);
        self.encode(payload_len, &mut buffer);
        writer.write_all(&buffer).await
    }
}

#[repr(u8)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Data = 0,
    Headers = 1,
    Priority = 2,
    Reset = 3,
    Settings = 4,
    PushPromise = 5,
    Ping = 6,
    GoAway = 7,
    WindowUpdate = 8,
    Continuation = 9,
    BinCode = 10,
    Unknown,
}

impl Kind {
    pub fn new(kind: u8) -> Self {
        Self::from(kind)
    }
}

impl From<u8> for Kind {
    fn from(val: u8) -> Self {
        Self::from(val as usize)
    }
}

impl From<u16> for Kind {
    fn from(val: u16) -> Self {
        Self::from(val as usize)
    }
}

impl From<u32> for Kind {
    fn from(val: u32) -> Self {
        Self::from(val as usize)
    }
}

impl From<usize> for Kind {
    fn from(val: usize) -> Self {
        use Kind::*;
        match val {
            0 => Data,
            1 => Headers,
            2 => Priority,
            3 => Reset,
            4 => Settings,
            5 => PushPromise,
            6 => Ping,
            7 => GoAway,
            8 => WindowUpdate,
            9 => Continuation,
            _ => Unknown,
        }
    }
}
