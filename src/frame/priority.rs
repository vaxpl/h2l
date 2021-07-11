use super::{Error, Frame, Head, Kind, StreamId};
use async_std::io::{Result as AsyncIoResult, Write};
use async_std::prelude::*;
use bytes::{BufMut, BytesMut};

#[derive(Debug, PartialEq, Eq)]
pub struct Priority {
    stream_id: StreamId,
    dependency: StreamDependency,
}

impl Priority {
    pub fn load(head: Head, payload: &[u8]) -> Result<Self, Error> {
        let dependency = StreamDependency::load(payload)?;

        if dependency.dependency_id() == head.stream_id() {
            return Err(Error::InvalidDependencyId);
        }

        Ok(Priority {
            stream_id: head.stream_id(),
            dependency,
        })
    }

    pub fn encode<T: BufMut>(&self, dst: &mut T) {
        let head = Head::new(Kind::Priority, 0, self.stream_id);
        head.encode(5, dst);
        self.dependency.encode(dst);
    }

    /// Write encoded raw bytes to specified `writer`.
    pub(crate) async fn write_to<W: Write + Unpin>(&self, writer: &mut W) -> AsyncIoResult<()> {
        let mut buffer = BytesMut::with_capacity(64);
        self.encode(&mut buffer);
        writer.write_all(&buffer).await
    }
}

impl From<Priority> for Frame {
    fn from(src: Priority) -> Self {
        Frame::Priority(src)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct StreamDependency {
    /// The ID of the stream dependency target
    dependency_id: StreamId,

    /// The weight for the stream. The value exposed (and set) here is always in
    /// the range [0, 255], instead of [1, 256] (as defined in section 5.3.2.)
    /// so that the value fits into a `u8`.
    weight: u8,

    /// True if the stream dependency is exclusive.
    is_exclusive: bool,
}

impl StreamDependency {
    pub fn new(dependency_id: StreamId, weight: u8, is_exclusive: bool) -> Self {
        StreamDependency {
            dependency_id,
            weight,
            is_exclusive,
        }
    }

    pub fn load(src: &[u8]) -> Result<Self, Error> {
        if src.len() != 5 {
            return Err(Error::InvalidPayloadLength);
        }

        // Parse the stream ID and exclusive flag
        let (dependency_id, is_exclusive) = StreamId::parse(&src[..4]);

        // Read the weight
        let weight = src[4];

        Ok(StreamDependency::new(dependency_id, weight, is_exclusive))
    }

    pub fn dependency_id(&self) -> StreamId {
        self.dependency_id
    }

    pub fn encode<T: BufMut>(&self, dst: &mut T) {
        let mut stream_id: u32 = self.dependency_id.into();
        if self.is_exclusive {
            stream_id |= 0x8000_0000u32;
        }
        dst.put_u32(stream_id);
        dst.put_u8(self.weight);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_dependency() {
        let samples = [
            (1, 0, false),
            (1, 0, true),
            (1, 255, false),
            (1, 255, true),
            (0x7fff_ffff, 0, false),
            (0x7fff_ffff, 0, true),
            (0x7fff_ffff, 255, false),
            (0x7fff_ffff, 255, true),
        ];
        for (i, w, x) in samples {
            let x1 = StreamDependency::new(StreamId(i), w, x);
            let mut buffer = BytesMut::with_capacity(64);
            x1.encode(&mut buffer);
            let x2 = StreamDependency::load(&buffer).unwrap();
            assert_eq!(x1, x2);
        }
    }
}
