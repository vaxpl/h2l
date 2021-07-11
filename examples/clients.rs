use async_std::net::TcpStream;
use async_std::prelude::*;
use async_std::task;
use futures::stream::FuturesUnordered;
use h2l::AsyncResult;
use std::time::Duration;

async fn client() -> AsyncResult<()> {
    let mut stream = TcpStream::connect("127.0.0.1:9900").await?;
    stream.write_all(b"hello world").await?;

    let mut buf = vec![0u8; 1024];
    let _n = stream.read(&mut buf).await?;

    Ok(())
}

fn main() {
    task::block_on(async {
        for _ in 0..100 {
            let tasks = FuturesUnordered::new();
            for _ in 0..1000 {
                tasks.push(task::spawn(client()));
            }
            for t in tasks {
                if let Err(err) = t.await {
                    println!("Client Error {:?}", err);
                }
            }
            task::sleep(Duration::from_millis(100)).await;
        }
    });
}
