use h2l::{LinkFlags, Server, Transaction};
use log::{info, warn};
use std::env;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

fn main() {
    env::set_var("RUST_LOG", "debug");
    env_logger::init();

    let server = Arc::new(Server::new("0.0.0.0:9900"));
    // let server_cloned = Arc::clone(&server);

    let attached_links = Arc::new(AtomicUsize::new(0usize));
    let detached_links = Arc::new(AtomicUsize::new(0usize));

    let attached_links_cloned = Arc::clone(&attached_links);
    let detached_links_cloned = Arc::clone(&detached_links);

    let th = Server::spawn(&server, move |tr| {
        info!("Transaction={:?}", tr);
        if let Transaction::LinkChanged { flags, token: _ } = tr {
            match flags {
                LinkFlags::ATTACHED => {
                    // server_cloned
                    //     .send_to(token, "Hello, Dear H2L client!")
                    //     .unwrap();
                    attached_links_cloned.fetch_add(1, Ordering::SeqCst);
                }
                LinkFlags::DETACHED => {
                    detached_links_cloned.fetch_add(1, Ordering::SeqCst);
                }
                _ => {
                    warn!("Unhandled LinkChanged {{ falgs: {:?} }}", flags);
                }
            }
        }
        info!(
            "ATTACHED={:?}, DETACHED={:?}",
            attached_links_cloned, detached_links_cloned
        );
    });

    th.join().unwrap();
}
