use super::frame::{
    self, Error as FrameError, Frame, Head, Kind as FrameKind, Result as FrameResult,
};
use super::{Token, Transaction};
use async_std::channel::{bounded, Receiver, Sender, TrySendError};
use async_std::net::TcpStream;
use async_std::prelude::*;
use async_std::task;
use bitflags::bitflags;
use bytes::{Buf, BytesMut};
use log::{debug, trace, warn};

use std::io::{Error as IoError, ErrorKind as IoErrorKind};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

/// A link represents the connection.
#[derive(Debug)]
pub struct Link {
    flags: AtomicU32,
    stream: TcpStream,
    outgoing_tx: Sender<Transaction>,
}

impl Link {
    async fn handshake_http_1_1_upgrade(&self, fetched: &[u8]) -> Result<(), IoError> {
        let mut buffer = [0; 4098];
        let (left, mut right) = buffer.split_at_mut(fetched.len());
        left.copy_from_slice(fetched);
        match self.stream().read(&mut right).await {
            Ok(_len) => {
                let mut headers = [httparse::EMPTY_HEADER; 16];
                let mut req = httparse::Request::new(&mut headers);
                let res = req.parse(&buffer).unwrap();
                if res.is_complete() {
                    for header in req.headers {
                        if header.name == "Upgrade" && header.value == b"h2c" {
                            self.stream()
                                .write_all(&super::consts::HTTP_1_1_SWITCHING_PROTOCOLS)
                                .await?;
                            return Ok(());
                        }
                    }
                }
                Err(IoError::new(
                    IoErrorKind::Unsupported,
                    "Not HTTP/1.1 Upgrade: h2c",
                ))
            }
            Err(err) => Err(err),
        }
    }

    async fn handshake_http2_prior_knowledge(&self) -> Result<(), (IoError, Option<[u8; 24]>)> {
        let mut magic: [u8; 24] = [0; 24];
        match self.stream().read_exact(&mut magic).await {
            Ok(()) => {
                if magic == super::consts::HTTP_2_0_PREFACE {
                    Ok(())
                } else {
                    Err((
                        IoError::new(IoErrorKind::Unsupported, "Not HTTP/2.0"),
                        Some(magic),
                    ))
                }
            }
            Err(err) => {
                debug!("Hanshake HTTP/2.0 PRI Error: {}", err);
                Err((err, None))
            }
        }
    }

    async fn handshake(&self, _incoming_tx: &Sender<Transaction>) -> Result<(), std::io::Error> {
        // Try HTTP/2.0 Prior Knowledge first
        if let Err((_err, Some(data))) = self.handshake_http2_prior_knowledge().await {
            // Try HTTP/1.1 Upgrade
            self.handshake_http_1_1_upgrade(&data).await?;
            // Try HTTP/2.0 Prior Knowledge again
            self.handshake_http2_prior_knowledge()
                .await
                .map_err(|(err, _data)| err)?;
        }
        Ok(())
    }

    async fn read_frame_head(&self) -> Result<(Head, usize), FrameError> {
        let mut buffer: [u8; 9] = [0; 9];
        match self.stream().read_exact(&mut buffer).await {
            Ok(_) => {
                let length = (&buffer[..]).get_uint(3) as usize;
                let head = frame::Head::parse(&buffer);
                Ok((head, length))
            }
            Err(err) => Err(FrameError::Io(err)),
        }
    }

    async fn read_data_frame(&self, head: Head, length: usize) -> FrameResult {
        let mut data = BytesMut::with_capacity(length);
        unsafe {
            data.set_len(length);
        }
        match self.stream().read_exact(data.as_mut()).await {
            Ok(_) => Ok(Frame::with_data(head.stream_id(), data)),
            Err(err) => Err(FrameError::Io(err)),
        }
    }

    async fn read_headers_frame(&self, head: Head, length: usize) -> FrameResult {
        let mut data = BytesMut::with_capacity(length);
        unsafe {
            data.set_len(length);
        }
        match self.stream().read_exact(data.as_mut()).await {
            Ok(_) => Ok(Frame::with_headers(head.stream_id(), data)),
            Err(err) => Err(FrameError::Io(err)),
        }
    }

    async fn read_settings_frame(&self, head: Head, length: usize) -> FrameResult {
        let mut buffer: [u8; 256] = [0; 256];
        match self.stream().read_exact(&mut buffer[0..length]).await {
            Ok(_) => frame::Settings::load(head, &buffer[0..length]).map(Frame::Settings),
            Err(err) => Err(FrameError::Io(err)),
        }
    }

    async fn read_go_away_frame(&self, _head: Head, length: usize) -> FrameResult {
        let mut data = BytesMut::with_capacity(length);
        unsafe {
            data.set_len(length);
        }
        match self.stream().read_exact(data.as_mut()).await {
            Ok(_) => frame::GoAway::load(&data).map(Frame::GoAway),
            Err(err) => Err(FrameError::Io(err)),
        }
    }

    async fn read_window_update_frame(&self, head: Head, _length: usize) -> FrameResult {
        let mut buffer: [u8; 4] = [0; 4];
        match self.stream().read_exact(&mut buffer).await {
            Ok(_) => frame::WindowUpdate::load(head, &buffer).map(Frame::WindowUpdate),
            Err(err) => Err(FrameError::Io(err)),
        }
    }

    async fn read_loop(self: Arc<Link>, incoming_tx: Sender<Transaction>) {
        match self.handshake(&incoming_tx).await {
            Ok(_) => {}
            Err(err) => {
                debug!("Handshake failed: {:?}", err);
            }
        }

        'outer: loop {
            match self.read_frame_head().await {
                Ok((head, length)) => {
                    let frame = match head.kind() {
                        FrameKind::Data => self.read_data_frame(head, length).await,
                        FrameKind::Headers => self.read_headers_frame(head, length).await,
                        FrameKind::Settings => self.read_settings_frame(head, length).await,
                        FrameKind::GoAway => self.read_go_away_frame(head, length).await,
                        FrameKind::WindowUpdate => {
                            self.read_window_update_frame(head, length).await
                        }
                        _ => {
                            unreachable!()
                        }
                    };
                    match frame {
                        Ok(frame) => {
                            let tr = Transaction::Frame {
                                frame,
                                token: Some(self.token()),
                            };
                            if let Err(err) = incoming_tx.send(tr).await {
                                warn!("Sending OwnedBytes error: {:?}", err);
                            }
                        }
                        Err(_err) => {
                            break 'outer;
                        }
                    }
                }
                Err(_err) => {
                    break 'outer;
                }
            }
        }

        self.add_flags(LinkFlags::READ_CLOSED);

        let tr = Transaction::LinkChanged {
            flags: self.flags(),
            token: self.token(),
        };

        if let Err(err) = incoming_tx.send(tr).await {
            warn!("Sending READ_CLOSED error: {:?}", err);
        }
    }

    async fn write_loop(
        self: Arc<Link>,
        incoming_tx: Sender<Transaction>,
        outgoing_rx: Receiver<Transaction>,
    ) {
        'outer: while let Ok(tr) = outgoing_rx.recv().await {
            match tr {
                Transaction::Bytes { data, token } => {
                    if let Err(err) = self.stream().write_all(&data).await {
                        warn!("Write StaticBytes to {:?} error: {}", token, err);
                        break 'outer;
                    }
                }
                Transaction::Frame { frame, token } => {
                    trace!("Write {:?} to {:?}", frame, token);
                    if let Err(err) = frame.write_to(&mut self.stream()).await {
                        warn!("Write {:?} to {:?} error: {}", frame, token, err);
                        break 'outer;
                    }
                }
                Transaction::LinkChanged { flags, token: _ } => {
                    if flags.contains(LinkFlags::READ_CLOSED) {
                        break;
                    }
                }
                _ => {
                    warn!("Unhandled {:?}", tr);
                }
            }
        }

        self.add_flags(LinkFlags::WRITE_CLOSED);
        let tr = Transaction::LinkChanged {
            flags: self.flags(),
            token: self.token(),
        };

        match incoming_tx.send(tr).await {
            Ok(_) => {}
            Err(err) => {
                warn!("Sending WRITE_CLOSED error: {:?}", err);
            }
        }
    }

    /// Construct a new link on connected stream.
    pub fn new(stream: TcpStream, incoming_tx: Sender<Transaction>) -> Arc<Self> {
        let (outgoing_tx, outgoing_rx) = bounded(1024);
        let link = Arc::new(Self {
            flags: AtomicU32::new(0),
            stream,
            outgoing_tx,
        });

        task::spawn(Self::read_loop(Arc::clone(&link), incoming_tx.clone()));
        task::spawn(Self::write_loop(
            Arc::clone(&link),
            incoming_tx,
            outgoing_rx,
        ));

        link
    }

    /// Returns the flags of the link.
    pub fn flags(&self) -> LinkFlags {
        match self.flags.load(Ordering::SeqCst) {
            0x0001 => LinkFlags::READ_CLOSED,
            0x0002 => LinkFlags::WRITE_CLOSED,
            0x0003 => LinkFlags::READ_WRITE_CLOSED,
            0x0004 => LinkFlags::ATTACHED,
            0x0008 => LinkFlags::DETACHED,
            _ => LinkFlags::NONE,
        }
    }

    /// Add a new flag(s) to the flags of the link with `val`.
    pub fn add_flags(&self, val: LinkFlags) {
        let new_val = self.flags() | val;
        self.flags.store(new_val.bits, Ordering::SeqCst);
    }

    /// Clear the flags of the link.
    pub fn clear_flags(&self) {
        self.flags.store(0, Ordering::SeqCst);
    }

    /// Replace the flags of the link with new `val`.
    pub fn set_flags(&self, val: LinkFlags) {
        self.flags.store(val.bits, Ordering::SeqCst);
    }

    /// Send a transaction to the link.
    pub fn send<T>(&self, tr: T) -> Result<(), TrySendError<Transaction>>
    where
        T: Into<Transaction>,
    {
        self.outgoing_tx.try_send(tr.into())
    }

    /// Returns the tcp stream of the link.
    pub fn stream(&self) -> &TcpStream {
        &self.stream
    }

    /// Returns a cloned tcp stream of the link.
    pub fn stream_cloned(&self) -> TcpStream {
        self.stream.clone()
    }

    /// Returns the token of the link.
    pub fn token(&self) -> Token {
        #[cfg(unix)]
        {
            use async_std::os::unix::io::AsRawFd;
            Token(self.stream.as_raw_fd() as usize)
        }
        #[cfg(windows)]
        {
            use async_std::os::windows::io::AsRawSocket;
            Token(self.stream.as_raw_socket() as usize)
        }
        #[cfg(not(any(unix, windows)))]
        unimplemented!()
    }
}

bitflags! {
    /// A flags describe the state of the link.
    pub struct LinkFlags: u32 {
        /// Nothing to represents.
        const NONE = 0x0000;
        /// The reader was closed.
        const READ_CLOSED = 0x0001;
        /// The writer was closed.
        const WRITE_CLOSED = 0x0002;
        /// The reader and writer are both closed.
        const READ_WRITE_CLOSED = Self::READ_CLOSED.bits | Self::WRITE_CLOSED.bits;
        /// Attached to the link manager.
        const ATTACHED = 0x0004;
        /// Detached from the link manager.
        const DETACHED = 0x0008;
    }
}
