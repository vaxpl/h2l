use super::{frame, AsyncResult, Link, LinkFlags, Token, Transaction};
use async_std::channel::{self, Receiver, Sender};
use async_std::future;
use async_std::net::TcpListener;
use async_std::prelude::*;
use async_std::task;
use bitflags::bitflags;
use log::{debug, info, warn};
use parking_lot::RwLock;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

pub struct Server {
    exit_accept_loop: AtomicBool,
    links: RwLock<HashMap<Token, Arc<Link>>>,
    addrs: Vec<std::net::SocketAddr>,
    trans_tx: Sender<Transaction>,
    trans_rx: Receiver<Transaction>,
}

impl Server {
    pub fn new<A: std::net::ToSocketAddrs>(addrs: A) -> Self {
        let addrs = addrs
            .to_socket_addrs()
            .map(|x| x.collect::<Vec<std::net::SocketAddr>>())
            .unwrap();
        let (trans_tx, trans_rx) = channel::bounded(4096);
        Self {
            exit_accept_loop: AtomicBool::new(false),
            links: RwLock::new(HashMap::new()),
            addrs,
            trans_tx,
            trans_rx,
        }
    }

    async fn accept_loop(self: Arc<Self>) -> AsyncResult<()> {
        let listener = TcpListener::bind(&self.addrs[..]).await?;
        let mut incoming = listener.incoming();

        'outer: loop {
            match future::timeout(Duration::from_millis(40), incoming.next()).await {
                Ok(Some(Ok(stream))) => {
                    let link = Link::new(stream, self.trans_tx.clone());
                    if let Err(err) = self.trans_tx.send(Transaction::NewLink { link }).await {
                        warn!("Sending NewLink error: {:?}", err);
                    }
                }
                Ok(v) => {
                    debug!("Unknown value {:?}, breaked!", v);
                    break 'outer;
                }
                Err(_) => {
                    if self.exit_accept_loop.load(Ordering::SeqCst) {
                        info!("Accept loop breaked by user.");
                        break;
                    }
                }
            }
        }

        if let Err(err) = self
            .trans_tx
            .send(Transaction::ServerChanged {
                flags: ServerFlags::ACCEPT_LOOP_EXITED,
            })
            .await
        {
            warn!("Sending ACCEPT_LOOP_EXITED error: {:?}", err);
        }

        Ok(())
    }

    async fn forward_loop<F>(
        self: Arc<Self>,
        mut f: F,
    ) -> AsyncResult<impl FnMut(Transaction) + Send + 'static>
    where
        F: FnMut(Transaction) + Send + 'static,
    {
        'outer: while let Ok(tr) = self.trans_rx.recv().await {
            match tr {
                Transaction::Frame { frame, token } => {
                    use frame::Frame::*;
                    let forward = match frame {
                        Data(_) => true,
                        Headers(_) => true,
                        Settings(ref v) => {
                            if !v.is_ack() {
                                self.send_to(
                                    token.unwrap_or(Token(0)),
                                    Transaction::Frame {
                                        frame: frame::Frame::Settings(*v),
                                        token,
                                    },
                                )
                                .unwrap();
                                self.send_to(
                                    token.unwrap_or(Token(0)),
                                    Transaction::Frame {
                                        frame: frame::Frame::Settings(frame::Settings::ack()),
                                        token,
                                    },
                                )
                                .unwrap();
                            }
                            false
                        }
                        _ => false,
                    };
                    if forward {
                        f(Transaction::Frame { frame, token })
                    }
                }
                Transaction::NewLink { link } => {
                    let token = link.token();
                    self.links.write().insert(token, link);
                    f(Transaction::LinkChanged {
                        flags: LinkFlags::ATTACHED,
                        token,
                    });
                }
                Transaction::LinkChanged { flags, token } => match flags {
                    LinkFlags::READ_CLOSED => {
                        match self.send_to(token, Transaction::LinkChanged { flags, token }) {
                            Ok(_) => {}
                            Err(err) => {
                                warn!("Send to {} error: {:?}", token, err);
                            }
                        }
                    }
                    LinkFlags::READ_WRITE_CLOSED => {
                        self.links.write().remove(&token);
                        f(Transaction::LinkChanged {
                            flags: LinkFlags::DETACHED,
                            token,
                        });
                    }
                    _ => {
                        debug!("Unhandled link flags: {:?}", flags);
                    }
                },
                Transaction::ServerChanged { flags } => match flags {
                    ServerFlags::ACCEPT_LOOP_EXITED => {
                        f(Transaction::ServerChanged {
                            flags: ServerFlags::FORWARD_LOOP_EXITED,
                        });
                        break 'outer;
                    }
                    _ => {
                        debug!("Unhandled server flags: {:?}", flags);
                    }
                },
                _ => {
                    f(tr);
                }
            }
        }
        Ok(f)
    }

    /// Find the link related to `token`.
    pub fn find_link<Q: Into<Token>>(&self, token: Q) -> Option<Arc<Link>> {
        self.links.read().get(&token.into()).map(|x| Arc::clone(x))
    }

    /// Returns the list of tokens of the links.
    pub fn link_tokens(&self) -> Vec<Token> {
        self.links.read().keys().map(Clone::clone).collect()
    }

    /// Run server and blocking current thread.
    pub fn run_forever<F>(self: &Arc<Self>, f: F)
    where
        F: FnMut(Transaction) + Send + 'static,
    {
        self.spawn(f).join().unwrap();
    }

    /// Send a transation to link specified by `token`.
    pub fn send_to<Q, T>(&self, token: Q, tr: T) -> AsyncResult<()>
    where
        Q: Into<Token> + Copy,
        T: Into<Transaction>,
    {
        self.find_link(token).map_or(
            Err(format!("Link {} not found!", token.into()).into()),
            |link| link.send(tr).map_err(|e| e.into()),
        )
    }

    /// Shutdown the server.
    pub fn shutdown(&self) {
        self.exit_accept_loop.store(true, Ordering::SeqCst);
    }

    /// Run server in standalone thread.
    pub fn spawn<F>(self: &Arc<Self>, f: F) -> std::thread::JoinHandle<()>
    where
        F: FnMut(Transaction) + Send + 'static,
    {
        let server_cloned1 = Arc::clone(self);
        let server_cloned2 = Arc::clone(self);
        std::thread::spawn(move || {
            let t1 = task::spawn(server_cloned1.accept_loop());
            let t2 = task::spawn(server_cloned2.forward_loop(f));
            task::block_on(async {
                if let Err(err) = t1.await {
                    warn!("Accept Loop returns with error: {:?}", err);
                }
                match t2.await {
                    Ok(mut f) => {
                        f(Transaction::ServerChanged {
                            flags: ServerFlags::TERMINATED,
                        });
                    }
                    Err(err) => {
                        warn!("Forward Loop returns with error: {:?}", err);
                    }
                }
            });
        })
    }
}

bitflags! {
    /// A flags describe the state of the link.
    pub struct ServerFlags: u32 {
        /// The accept loop was exited.
        const ACCEPT_LOOP_EXITED = 0x0001;
        /// The forward loop was exited.
        const FORWARD_LOOP_EXITED = 0x0002;
        /// The server has been shutdown.
        const TERMINATED = Self::ACCEPT_LOOP_EXITED.bits | Self::FORWARD_LOOP_EXITED.bits;
    }
}
