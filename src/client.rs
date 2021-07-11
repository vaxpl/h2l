use super::frame::{
    self, Data, Error as FrameError, Frame, Head, Kind, Result as FrameResult, Settings,
};
use super::{AsyncResult, Token, Transaction};
use async_std::channel::{self, Receiver, Sender, TrySendError};
use async_std::future;
use async_std::net::{Shutdown, TcpStream};
use async_std::prelude::*;
use async_std::task;
use bitflags::bitflags;
use bytes::{Buf, BytesMut};
use log::{debug, warn};

use std::fmt::{self, Display};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const RECONNECT_DELAY_MS: u64 = 3000;

pub struct Client {
    addrs: Vec<std::net::SocketAddr>,
    auto_reconnect: bool,
    exit_loop: AtomicBool,
    flags: AtomicU32,
    raw_token: AtomicUsize,
    incoming_tx: Sender<Transaction>,
    incoming_rx: Receiver<Transaction>,
    outgoing_tx: Sender<Transaction>,
    outgoing_rx: Receiver<Transaction>,
}

impl Client {
    pub fn new<A: std::net::ToSocketAddrs>(addrs: A, auto_reconnect: bool) -> Self {
        let addrs = addrs
            .to_socket_addrs()
            .map(|x| x.collect::<Vec<std::net::SocketAddr>>())
            .unwrap();
        let (incoming_tx, incoming_rx) = channel::bounded(4096);
        let (outgoing_tx, outgoing_rx) = channel::bounded(4096);
        Self {
            addrs,
            auto_reconnect,
            exit_loop: AtomicBool::new(false),
            flags: AtomicU32::new(0),
            raw_token: AtomicUsize::new(0),
            incoming_tx,
            incoming_rx,
            outgoing_tx,
            outgoing_rx,
        }
    }

    /// Returns true if the client should be shutdown.
    pub(crate) fn is_exit_loop(&self) -> bool {
        self.exit_loop.load(Ordering::SeqCst)
    }

    /// Mark the client should be shutdown if true.
    pub(crate) fn set_exit_loop(&self, val: bool) {
        self.exit_loop.store(val, Ordering::SeqCst);
    }

    /// Returns the flags of the client.
    pub fn flags(&self) -> ClientFlags {
        ClientFlags::from_bits_truncate(self.flags.load(Ordering::SeqCst))
    }

    /// Add a new flag(s) to the flags of the client with `val`.
    pub fn add_flags(&self, val: ClientFlags) {
        let new_val = self.flags() | val;
        self.flags.store(new_val.bits, Ordering::SeqCst);
    }

    /// Remove a new flag(s) to the flags of the client with `val`.
    pub fn remove_flags(&self, val: ClientFlags) {
        let mut new_val = self.flags();
        new_val.remove(val);
        self.flags.store(new_val.bits, Ordering::SeqCst);
    }

    /// Clear the flags of the client.
    pub fn clear_flags(&self) {
        self.flags.store(0, Ordering::SeqCst);
    }

    /// Replace the flags of the client with new `val`.
    pub fn set_flags(&self, val: ClientFlags) {
        self.flags.store(val.bits, Ordering::SeqCst);
    }

    pub(crate) fn raw_token(&self) -> usize {
        self.raw_token.load(Ordering::SeqCst)
    }

    pub(crate) fn set_raw_token(&self, val: usize) {
        self.raw_token.store(val, Ordering::SeqCst)
    }

    async fn handshake(&self, stream: &mut TcpStream) -> Result<Settings, HandshakeError> {
        stream.write_all(super::consts::HTTP_2_0_PREFACE).await?;
        stream.write_all(super::consts::HTTP_2_0_SETTINGS).await?;
        let mut buffer: [u8; 1024] = [0; 1024];
        stream.read_exact(&mut buffer[0..9]).await?;
        let length = (&buffer[..]).get_uint(3) as usize;
        let head = Head::parse(&buffer[..]);
        match head.kind() {
            Kind::Settings => {
                let data = &mut buffer[0..length];
                stream.read_exact(data).await?;
                let frame = Settings::load(head, data)?;
                Ok(frame)
            }
            _ => Err(HandshakeError::UnknownProtocol),
        }
    }

    async fn read_frame_head(&self, stream: &mut TcpStream) -> Result<(Head, usize), FrameError> {
        let mut buffer: [u8; 9] = [0; 9];
        match stream.read_exact(&mut buffer).await {
            Ok(_) => {
                let length = (&buffer[..]).get_uint(3) as usize;
                let head = frame::Head::parse(&buffer);
                Ok((head, length))
            }
            Err(err) => Err(FrameError::Io(err)),
        }
    }

    async fn discard_unknown_frame(
        &self,
        stream: &mut TcpStream,
        _head: Head,
        length: usize,
    ) -> FrameResult {
        let mut data = BytesMut::with_capacity(length);
        unsafe {
            data.set_len(length);
        }
        match stream.read_exact(data.as_mut()).await {
            Ok(_) => Ok(Frame::Unknown),
            Err(err) => Err(FrameError::Io(err)),
        }
    }

    async fn read_data_frame(
        &self,
        stream: &mut TcpStream,
        head: Head,
        length: usize,
    ) -> FrameResult {
        let mut data = BytesMut::with_capacity(length);
        unsafe {
            data.set_len(length);
        }
        match stream.read_exact(data.as_mut()).await {
            Ok(_) => Data::load(head, data).map(Frame::Data),
            Err(err) => Err(FrameError::Io(err)),
        }
    }

    async fn read_headers_frame(
        &self,
        stream: &mut TcpStream,
        head: Head,
        length: usize,
    ) -> FrameResult {
        let mut data = BytesMut::with_capacity(length);
        unsafe {
            data.set_len(length);
        }
        match stream.read_exact(data.as_mut()).await {
            Ok(_) => Ok(Frame::with_headers(head.stream_id(), data)),
            Err(err) => Err(FrameError::Io(err)),
        }
    }

    async fn read_settings_frame(
        &self,
        stream: &mut TcpStream,
        head: Head,
        length: usize,
    ) -> FrameResult {
        let mut buffer: [u8; 256] = [0; 256];
        match stream.read_exact(&mut buffer[0..length]).await {
            Ok(_) => frame::Settings::load(head, &buffer[0..length]).map(Frame::Settings),
            Err(err) => Err(FrameError::Io(err)),
        }
    }

    async fn read_go_away_frame(
        &self,
        stream: &mut TcpStream,
        _head: Head,
        length: usize,
    ) -> FrameResult {
        let mut data = BytesMut::with_capacity(length);
        unsafe {
            data.set_len(length);
        }
        match stream.read_exact(data.as_mut()).await {
            Ok(_) => frame::GoAway::load(&data).map(Frame::GoAway),
            Err(err) => Err(FrameError::Io(err)),
        }
    }

    async fn read_window_update_frame(
        &self,
        stream: &mut TcpStream,
        head: Head,
        _length: usize,
    ) -> FrameResult {
        let mut buffer: [u8; 4] = [0; 4];
        match stream.read_exact(&mut buffer).await {
            Ok(_) => frame::WindowUpdate::load(head, &buffer).map(Frame::WindowUpdate),
            Err(err) => Err(FrameError::Io(err)),
        }
    }

    async fn read_loop(self: Arc<Self>, mut stream: TcpStream) {
        'outer: loop {
            match self.read_frame_head(&mut stream).await {
                Ok((head, length)) => {
                    let frame = match head.kind() {
                        Kind::Data => self.read_data_frame(&mut stream, head, length).await,
                        Kind::Headers => self.read_headers_frame(&mut stream, head, length).await,
                        Kind::Settings => self.read_settings_frame(&mut stream, head, length).await,
                        Kind::GoAway => self.read_go_away_frame(&mut stream, head, length).await,
                        Kind::WindowUpdate => {
                            self.read_window_update_frame(&mut stream, head, length)
                                .await
                        }
                        _ => {
                            warn!("#{} Unknown kind: {:?}", self.token(), head.kind());
                            self.discard_unknown_frame(&mut stream, head, length).await
                        }
                    };
                    match frame {
                        Ok(frame) => {
                            let tr = Transaction::Frame {
                                frame,
                                token: Some(self.token()),
                            };
                            if let Err(err) = self.incoming_tx.send(tr).await {
                                warn!("Sending OwnedBytes error: {:?}", err);
                            }
                        }
                        Err(err) => {
                            println!("Error={:?}", err);
                            break 'outer;
                        }
                    }
                }
                Err(_err) => {
                    break 'outer;
                }
            }
        }

        stream.shutdown(Shutdown::Both).unwrap();

        self.add_flags(ClientFlags::READ_CLOSED);

        let tr = Transaction::ClientChanged {
            flags: self.flags(),
            token: self.token(),
        };

        if let Err(err) = self.incoming_tx.send(tr).await {
            warn!("Sending READ_CLOSED error: {:?}", err);
        }

        debug!("{:?} Exited from read loop", self.token());
    }

    async fn write_loop(self: Arc<Self>, mut stream: TcpStream) {
        'outer: while let Ok(tr) = self.outgoing_rx.recv().await {
            match tr {
                Transaction::Bytes { data, token } => {
                    if let Err(err) = stream.write_all(&data).await {
                        warn!("Write StaticBytes to {:?} error: {}", token, err);
                        break 'outer;
                    }
                }
                Transaction::Frame { frame, token } => {
                    if let Err(err) = frame.write_to(&mut stream).await {
                        warn!("Write {:?} to {:?} error: {}", frame, token, err);
                        break 'outer;
                    }
                }
                Transaction::ClientChanged { flags, token: _ } => {
                    if flags.contains(ClientFlags::READ_CLOSED) {
                        debug!("{:?} Break write loop when read loop exited", self.token());
                        break 'outer;
                    }
                }
                _ => {
                    println!("Todo!");
                }
            }
        }

        // Flush the outgoing channel.
        while self.outgoing_rx.try_recv().is_ok() {
            // Nothing to do.
        }

        self.add_flags(ClientFlags::WRITE_CLOSED);

        let tr = Transaction::ClientChanged {
            flags: self.flags(),
            token: self.token(),
        };

        if let Err(err) = self.incoming_tx.send(tr).await {
            warn!("{:?} Sending WRITE_CLOSED error: {:?}", self.token(), err);
        }

        debug!("{:?} Exited from write loop", self.token());
    }

    async fn delay_until_exit_loop(&self, delay: Duration) {
        let start = Instant::now();
        while !self.is_exit_loop() && start.elapsed() < delay {
            task::sleep(Duration::from_millis(40)).await;
        }
    }

    async fn mantain_loop(self: Arc<Self>) -> AsyncResult<()> {
        'outer: while !self.is_exit_loop() {
            match future::timeout(
                Duration::from_millis(40),
                TcpStream::connect(&self.addrs[..]),
            )
            .await
            {
                Ok(Ok(mut stream)) => match self.handshake(&mut stream).await {
                    Ok(settings) => {
                        // Export the raw token.
                        self.set_raw_token(tcp_stream_id(&stream));
                        debug!("Connect to {:?} okay, {:?}", self.addrs, self.token());
                        // Mark with HANDSHAKED flag.
                        self.add_flags(ClientFlags::HANDSHAKED);
                        let tr = Transaction::ClientChanged {
                            flags: self.flags(),
                            token: self.token(),
                        };
                        self.incoming_tx.send(tr).await.unwrap();
                        // Forward the Settings frame.
                        let tr = Transaction::Frame {
                            frame: Frame::Settings(settings),
                            token: Some(self.token()),
                        };
                        self.incoming_tx.send(tr).await.unwrap();
                        // Enter read and write loop.
                        let client1 = Arc::clone(&self);
                        let client2 = Arc::clone(&self);
                        let t1 = task::spawn(client1.read_loop(stream.clone()));
                        let t2 = task::spawn(client2.write_loop(stream.clone()));
                        // Wait for both exited.
                        t1.await;
                        t2.await;
                        // Remove the READ_CLOSED and WRITE_CLOSED flags.
                        self.remove_flags(ClientFlags::READ_WRITE_CLOSED);
                        // Unset the raw token.
                        self.set_raw_token(0);
                        if !self.auto_reconnect {
                            break 'outer;
                        }
                        if !self.flags().contains(ClientFlags::RECONNECTED) {
                            self.add_flags(ClientFlags::RECONNECTED);
                        }
                    }
                    Err(err) => {
                        debug!(
                            "Handshake to {:?} failed: {}, mantain loop breaked",
                            self.addrs, err
                        );
                        break 'outer;
                    }
                },
                Ok(Err(err)) => {
                    let delay = Duration::from_millis(RECONNECT_DELAY_MS);
                    debug!(
                        "Connect to {:?} failed: {}, try again {:?} later",
                        self.addrs, err, delay
                    );
                    self.delay_until_exit_loop(delay).await;
                }
                Err(_) => {
                    // Continue on timeout.
                }
            }
        }
        self.set_exit_loop(true);
        self.add_flags(ClientFlags::MANTAIN_LOOP_EXITED);
        Ok(())
    }

    async fn on_client_changed(&self, tr: Transaction) -> ForwardFlow {
        if let Transaction::ClientChanged { flags, token: _ } = tr {
            if flags.contains(ClientFlags::MANTAIN_LOOP_EXITED) {
                return ForwardFlow::Break;
            }
            if flags.contains(ClientFlags::READ_CLOSED)
                && !flags.contains(ClientFlags::WRITE_CLOSED)
            {
                let tr = Transaction::ClientChanged {
                    flags: self.flags(),
                    token: self.token(),
                };
                self.outgoing_tx.send(tr).await.unwrap();
            }
        }
        ForwardFlow::Forward(tr)
    }

    async fn forward_loop<F>(
        self: Arc<Self>,
        mut f: F,
    ) -> AsyncResult<impl FnMut(Transaction) + Send + 'static>
    where
        F: FnMut(Transaction) + Send + 'static,
    {
        'outer: while let Ok(tr) = self.incoming_rx.recv().await {
            let flow = match tr {
                Transaction::ClientChanged { .. } => self.on_client_changed(tr).await,
                _ => ForwardFlow::Forward(tr),
            };
            match flow {
                ForwardFlow::Break => {
                    break 'outer;
                }
                ForwardFlow::Continue => {}
                ForwardFlow::Forward(tr) => f(tr),
            }
        }
        debug!("#{} Exited from Forward Loop!", self.token());
        Ok(f)
    }

    /// Run client and blocking current thread.
    pub fn run_forever<F>(self: &Arc<Self>, f: F)
    where
        F: FnMut(Transaction) + Send + 'static,
    {
        self.spawn(f).join().unwrap();
    }

    /// Shutdown the client.
    pub fn shutdown(&self) {
        self.set_exit_loop(true);
    }

    /// Run client in standalone thread.
    pub fn spawn<F>(self: &Arc<Self>, f: F) -> std::thread::JoinHandle<()>
    where
        F: FnMut(Transaction) + Send + 'static,
    {
        let client = Arc::clone(self);
        std::thread::spawn(move || {
            let clone1 = Arc::clone(&client);
            let clone2 = Arc::clone(&client);
            let t1 = task::spawn(clone1.mantain_loop());
            let t2 = task::spawn(clone2.forward_loop(f));
            task::block_on(async {
                if let Err(err) = t1.await {
                    warn!("Mantain Loop returns with error: {:?}", err);
                }
                match t2.await {
                    Ok(mut f) => {
                        f(Transaction::ClientChanged {
                            flags: ClientFlags::TERMINATED,
                            token: client.token(),
                        });
                    }
                    Err(err) => {
                        warn!("Forward Loop returns with error: {:?}", err);
                    }
                }
            });
        })
    }

    /// Returns the token of the client.
    pub fn token(&self) -> Token {
        Token(self.raw_token())
    }

    /// Attempts to send a transaction into the client outgoing channel.
    ///
    /// If the channel is full or closed, this method returns an error.
    pub fn try_send<T>(&self, tr: T) -> Result<(), TrySendError<Transaction>>
    where
        T: Into<Transaction>,
    {
        self.outgoing_tx.try_send(tr.into())
    }
}

bitflags! {
    /// A flags describe the state of the client.
    pub struct ClientFlags: u32 {
        /// The mantain loop was exited.
        const MANTAIN_LOOP_EXITED = 0x0001;
        /// The forward loop was exited.
        const FORWARD_LOOP_EXITED = 0x0002;
        /// The client has been shutdown.
        const TERMINATED = Self::MANTAIN_LOOP_EXITED.bits | Self::FORWARD_LOOP_EXITED.bits;
        /// The connection was handshaked.
        const HANDSHAKED = 0x0004;
        /// The reader was closed.
        const READ_CLOSED = 0x0008;
        /// The writer was closed.
        const WRITE_CLOSED = 0x0010;
        /// The reader and writer are both closed.
        const READ_WRITE_CLOSED = Self::READ_CLOSED.bits | Self::WRITE_CLOSED.bits;
        /// The connections was auto reconnected.
        const RECONNECTED = 0x0020;
    }
}

impl ClientFlags {
    /// Returns true if client at newly handshaked state.
    pub fn is_newly_handshaked(&self) -> bool {
        *self == Self::HANDSHAKED || *self == Self::HANDSHAKED | Self::RECONNECTED
    }
}

#[derive(Debug)]
enum ForwardFlow {
    Break,
    Continue,
    Forward(Transaction),
}

impl Default for ForwardFlow {
    fn default() -> Self {
        ForwardFlow::Continue
    }
}

#[derive(Debug)]
pub enum HandshakeError {
    Frame(frame::Error),
    UnknownProtocol,
    Io(std::io::Error),
}

impl Display for HandshakeError {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        use HandshakeError::*;
        match self {
            Frame(err) => {
                write!(fmt, "{}", err)
            }
            UnknownProtocol => {
                write!(fmt, "UnknownProtocol")
            }
            Io(err) => {
                write!(fmt, "{}", err)
            }
        }
    }
}

impl std::error::Error for HandshakeError {}

impl From<frame::Error> for HandshakeError {
    fn from(val: frame::Error) -> Self {
        Self::Frame(val)
    }
}

impl From<std::io::Error> for HandshakeError {
    fn from(val: std::io::Error) -> Self {
        Self::Io(val)
    }
}

/// Returns the identifer of the TcpStream.
fn tcp_stream_id(stream: &TcpStream) -> usize {
    #[cfg(unix)]
    {
        use async_std::os::unix::io::AsRawFd;
        stream.as_raw_fd() as usize
    }
    #[cfg(windows)]
    {
        use async_std::os::windows::io::AsRawSocket;
        stream.as_raw_socket() as usize
    }
    #[cfg(not(any(unix, windows)))]
    unimplemented!()
}
