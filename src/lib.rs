/// A specialized Result type for async operations.
pub type AsyncResult<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

mod client;
pub mod consts;
pub mod error;
#[macro_use]
pub mod frame;
mod link;
mod server;
mod stream;
mod token;
mod transaction;

// pub use binary_frame::*;
// pub use error::Errror;
pub use client::{Client, ClientFlags};
pub use link::*;
pub use server::*;
pub use stream::*;
pub use token::Token;
pub use transaction::*;

#[cfg(test)]
mod tests {
    use super::*;
    use async_std::net::TcpStream;
    use async_std::prelude::*;
    use async_std::task;
    use futures::stream::FuturesUnordered;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    fn test_attach_and_detach() {
        let server = Arc::new(Server::new("127.0.0.1:9900"));
        let server_cloned = Arc::clone(&server);

        let attached_links = Arc::new(AtomicUsize::new(0usize));
        let detached_links = Arc::new(AtomicUsize::new(0usize));
        let attached_links_cloned = Arc::clone(&attached_links);
        let detached_links_cloned = Arc::clone(&detached_links);

        let th = Server::spawn(&server, move |tr| {
            match tr {
                Transaction::LinkChanged { flags, token } => match flags {
                    LinkFlags::ATTACHED => {
                        server_cloned
                            .send_to(token, "Hello, Dear H2L client!")
                            .unwrap();
                        attached_links_cloned.fetch_add(1, Ordering::SeqCst);
                    }
                    LinkFlags::DETACHED => {
                        detached_links_cloned.fetch_add(1, Ordering::SeqCst);
                    }
                    _ => {
                        println!("Unhandled LinkChanged {{ falgs: {:?} }}", flags);
                    }
                },
                _ => {
                    println!("Unhandled {:?}", tr);
                }
            }
            println!(
                "ATTACHED={:?}, DETACHED={:?}",
                attached_links_cloned, detached_links_cloned
            );
        });

        async fn client() -> AsyncResult<()> {
            let mut stream = TcpStream::connect("127.0.0.1:9900").await?;
            stream.write_all(b"Hello, Dear H2L server!").await?;

            let mut buf = vec![0u8; 1024];
            let _n = stream.read(&mut buf).await?;
            Ok(())
        }

        task::block_on(async {
            let tasks = FuturesUnordered::new();
            // Running 1000 tasks in background.
            for _ in 0..1000 {
                tasks.push(task::spawn(client()));
            }
            // Wait all tasks done
            for t in tasks {
                t.await.unwrap();
            }
            // Wait all tasks has been executed.
            task::sleep(Duration::from_secs(1)).await;
        });

        assert_eq!(
            attached_links.load(Ordering::SeqCst),
            detached_links.load(Ordering::SeqCst)
        );

        server.shutdown();

        th.join().unwrap();
    }
}
