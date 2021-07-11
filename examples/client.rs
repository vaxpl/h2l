use bytes::Bytes;
use h2l::frame::StreamId;
use h2l::{Client, Transaction};
use log::info;
use std::env;
use std::sync::Arc;

fn main() {
    env::set_var("RUST_LOG", "debug");
    env_logger::init();

    let client = Arc::new(Client::new("127.0.0.1:9900", true));
    let client_cloned = Arc::clone(&client);

    let th = client.spawn(move |tr| {
        info!("Transaction={:?}", tr);
        match tr {
            Transaction::ClientChanged { flags, token: _ } => {
                if flags.is_newly_handshaked() {
                    let tr = Transaction::with_data_frame(
                        StreamId(1),
                        Bytes::from_static(b"Hello, H2L!"),
                        None,
                    );
                    client_cloned.try_send(tr).unwrap();
                }
            }
            Transaction::Frame { frame: _, token: _ } => {
                // Add you code here to processing the frame
            }
            _ => {}
        }
    });

    th.join().unwrap();
}
