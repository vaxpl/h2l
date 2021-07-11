use async_std::io::{Result as AsyncIoResult, Write};

use std::fmt;

/// A helper macro that unpacks a sequence of 4 bytes found in the buffer with
/// the given identifier, starting at the given offset, into the given integer
/// type. Obviously, the integer type should be able to support at least 4
/// bytes.
///
/// # Examples
///
/// ```rust
/// let buf: [u8; 4] = [0, 0, 0, 1];
/// assert_eq!(1u32, unpack_octets_4!(buf, 0, u32));
/// ```
#[macro_escape]
macro_rules! unpack_octets_4 {
    // TODO: Get rid of this macro
    ($buf:expr, $offset:expr, $tip:ty) => {
        (($buf[$offset + 0] as $tip) << 24)
            | (($buf[$offset + 1] as $tip) << 16)
            | (($buf[$offset + 2] as $tip) << 8)
            | (($buf[$offset + 3] as $tip) << 0)
    };
}

mod bin_code;
pub mod consts;
mod continuation;
mod data;
mod go_away;
mod head;
mod headers;
// mod ping;
mod priority;
// mod push_promise;
mod reason;
mod reset;
mod settings;
mod stream_id;
mod util;
mod window_update;

pub use self::bin_code::{BinCode, BinCodeFlags};
pub use self::continuation::Continuation;
pub use self::data::Data;
pub use self::go_away::GoAway;
pub use self::head::{Flags, Head, Kind};
pub use self::headers::Headers;
// pub use self::ping::Ping;
pub use self::priority::Priority;
// pub use self::push_promise::{PushPromise};
pub use self::reason::Reason;
pub use self::reset::Reset;
pub use self::settings::Settings;
pub use self::stream_id::{StreamId, StreamIdOverflow};
pub use self::window_update::WindowUpdate;

pub type FrameSize = u32;

pub enum Frame {
    Data(Data),
    Headers(Headers),
    Priority(Priority),
    Reset(Reset),
    Settings(Settings),
    // PushPromise(PushPromise),
    // Ping(Ping),
    GoAway(GoAway),
    WindowUpdate(WindowUpdate),
    Continuation(Continuation),
    BinCode(BinCode),
    Unknown,
}

impl Frame {
    /// Construct a Data frame.
    pub fn with_data<T>(stream_id: StreamId, payload: T) -> Self
    where
        T: AsRef<[u8]> + Send + Sync + 'static,
    {
        Self::Data(Data::new(stream_id, payload))
    }

    /// Construct a Headers frame.
    pub fn with_headers<T>(stream_id: StreamId, raw_data: T) -> Self
    where
        T: AsRef<[u8]> + Send + Sync + 'static,
    {
        Self::Headers(Headers::new(stream_id, raw_data))
    }

    /// Write encoded raw bytes to specified `writer`.
    pub async fn write_to<W: Write + Unpin>(&self, writer: &mut W) -> AsyncIoResult<()> {
        use self::Frame::*;

        match *self {
            Data(ref frame) => frame.write_to(writer).await,
            Headers(ref frame) => frame.write_to(writer).await,
            Priority(ref frame) => frame.write_to(writer).await,
            Reset(ref frame) => frame.write_to(writer).await,
            Settings(ref frame) => frame.write_to(writer).await,
            // PushPromise(ref frame) => frame.write_to(writer),
            // Ping(ref frame) => frame.write_to(writer),
            GoAway(ref frame) => frame.write_to(writer).await,
            WindowUpdate(ref frame) => frame.write_to(writer).await,
            Continuation(ref frame) => frame.write_to(writer).await,
            BinCode(ref frame) => frame.write_to(writer).await,
            Unknown => Ok(()),
        }
    }
}

impl fmt::Debug for Frame {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        use self::Frame::*;

        match *self {
            Data(ref frame) => fmt::Debug::fmt(frame, fmt),
            Headers(ref frame) => fmt::Debug::fmt(frame, fmt),
            Priority(ref frame) => fmt::Debug::fmt(frame, fmt),
            Reset(ref frame) => fmt::Debug::fmt(frame, fmt),
            Settings(ref frame) => fmt::Debug::fmt(frame, fmt),
            // PushPromise(ref frame) => fmt::Debug::fmt(frame, fmt),
            // Ping(ref frame) => fmt::Debug::fmt(frame, fmt),
            GoAway(ref frame) => fmt::Debug::fmt(frame, fmt),
            WindowUpdate(ref frame) => fmt::Debug::fmt(frame, fmt),
            Continuation(ref frame) => fmt::Debug::fmt(frame, fmt),
            BinCode(ref frame) => fmt::Debug::fmt(frame, fmt),
            Unknown => write!(fmt, "Frame::Unknown"),
        }
    }
}

/// Errors that can occur during parsing an HTTP/2 frame.
#[derive(Debug)]
pub enum Error {
    /// A length value other than 8 was set on a PING message.
    BadFrameSize,

    /// The padding length was larger than the frame-header-specified
    /// length of the payload.
    TooMuchPadding,

    /// An invalid setting value was provided
    InvalidSettingValue,

    /// An invalid window update value
    InvalidWindowUpdateValue,

    /// The payload length specified by the frame header was not the
    /// value necessary for the specific frame type.
    InvalidPayloadLength,

    /// Received a payload with an ACK settings frame
    InvalidPayloadAckSettings,

    /// An invalid stream identifier was provided.
    ///
    /// This is returned if a SETTINGS or PING frame is received with a stream
    /// identifier other than zero.
    InvalidStreamId,

    /// A request or response is malformed.
    MalformedMessage,

    /// An invalid stream dependency ID was provided
    ///
    /// This is returned if a HEADERS or PRIORITY frame is received with an
    /// invalid stream identifier.
    InvalidDependencyId,
    // /// Failed to perform HPACK decoding
    // Hpack(hpack::DecoderError),
    Io(std::io::Error),
}

impl std::error::Error for Error {}

impl fmt::Display for Error {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        use Error::*;

        match self {
            Io(err) => write!(fmt, "{}", err),
            _ => write!(fmt, "{:?}", self),
        }
    }
}

pub type Result = std::result::Result<Frame, Error>;
