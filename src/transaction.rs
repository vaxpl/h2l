use super::frame::{Frame, StreamId};
use super::{ClientFlags, Link, LinkFlags, ServerFlags, Token};

use bytes::Bytes;
use std::sync::Arc;

#[derive(Debug)]
pub enum Transaction {
    // General transactions.
    Bytes { data: Bytes, token: Option<Token> },
    Frame { frame: Frame, token: Option<Token> },
    // Transactions for client only.
    ClientChanged { flags: ClientFlags, token: Token },
    // Transactions for server only.
    LinkChanged { flags: LinkFlags, token: Token },
    NewLink { link: Arc<Link> },
    ServerChanged { flags: ServerFlags },
}

impl Transaction {
    pub fn with_data_frame<T>(stream_id: StreamId, payload: T, token: Option<Token>) -> Self
    where
        T: AsRef<[u8]> + Send + Sync + 'static,
    {
        Self::Frame {
            frame: Frame::with_data(stream_id, payload),
            token,
        }
    }
}

impl From<Arc<Link>> for Transaction {
    fn from(val: Arc<Link>) -> Self {
        Transaction::NewLink { link: val }
    }
}

impl From<&'static str> for Transaction {
    fn from(val: &'static str) -> Self {
        Transaction::Bytes {
            data: Bytes::from(val),
            token: None,
        }
    }
}

impl From<&'static [u8]> for Transaction {
    fn from(val: &'static [u8]) -> Self {
        Transaction::Bytes {
            data: Bytes::from(val),
            token: None,
        }
    }
}

impl From<Vec<u8>> for Transaction {
    fn from(val: Vec<u8>) -> Self {
        Transaction::Bytes {
            data: Bytes::from(val),
            token: None,
        }
    }
}
