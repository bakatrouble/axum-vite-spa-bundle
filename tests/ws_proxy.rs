//! Debug builds proxy WebSocket upgrades (Vite HMR) to the dev server.
//!
//! A raw-socket fake upstream answers the handshake with `101` and then echoes
//! bytes; the test drives the router over a real socket and checks both steps.
#![cfg(debug_assertions)]

use std::net::SocketAddr;
use std::time::Duration;

use axum_vite_spa_bundle::{SpaConfig, spa_router};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[derive(Clone, Copy, rust_embed::Embed)]
#[folder = "examples/vite-app/dist/"]
struct Dist;

/// Reads one HTTP head (through the blank line) from a socket.
async fn read_head(sock: &mut TcpStream) -> Option<String> {
    let mut head = Vec::new();
    let mut byte = [0_u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if sock.read(&mut byte).await.ok()? == 0 {
            return None;
        }
        head.push(byte[0]);
    }
    String::from_utf8(head).ok()
}

/// Fake Vite: accepts the upgrade, then echoes every byte it receives.
async fn spawn_upstream() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind upstream");
    let addr = listener.local_addr().expect("upstream addr");
    tokio::spawn(async move {
        while let Ok((mut sock, _)) = listener.accept().await {
            tokio::spawn(async move {
                let Some(head) = read_head(&mut sock).await else {
                    return;
                };
                assert!(
                    head.to_ascii_lowercase().contains("upgrade: websocket"),
                    "{head}"
                );
                let response = "HTTP/1.1 101 Switching Protocols\r\n\
                    Upgrade: websocket\r\nConnection: Upgrade\r\n\
                    Sec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=\r\n\r\n";
                if sock.write_all(response.as_bytes()).await.is_err() {
                    return;
                }
                let mut buf = [0_u8; 64];
                loop {
                    match sock.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if sock.write_all(&buf[..n]).await.is_err() {
                                break;
                            }
                        }
                    }
                }
            });
        }
    });
    addr
}

#[tokio::test]
async fn websocket_upgrade_is_proxied_and_bytes_flow_both_ways() {
    let upstream = spawn_upstream().await;
    // The env var is not set in tests, so the default dev URL applies.
    let config = SpaConfig::new(
        Dist,
        "WEB_BUNDLE_WS_TEST_UNSET_VAR",
        format!("http://{upstream}"),
        ["/api/"],
    );
    let app = spa_router::<_, ()>(config);
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind router");
    let addr = listener.local_addr().expect("router addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let exchange = async {
        let mut client = TcpStream::connect(addr).await.expect("connect router");
        client
            .write_all(
                b"GET /@vite/client HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\n\
                  Connection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
                  Sec-WebSocket-Version: 13\r\n\r\n",
            )
            .await
            .expect("write upgrade request");
        let head = read_head(&mut client).await.expect("upgrade response head");
        assert!(head.starts_with("HTTP/1.1 101"), "{head}");

        client.write_all(b"ping").await.expect("write payload");
        let mut echoed = [0_u8; 4];
        client.read_exact(&mut echoed).await.expect("read echo");
        assert_eq!(&echoed, b"ping");
    };
    tokio::time::timeout(Duration::from_secs(10), exchange)
        .await
        .expect("websocket exchange finished in time");
}
