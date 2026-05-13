#![cfg(unix)]
use async_trait::async_trait;
use std::sync::Arc;
use tempfile::tempdir;
use teramind_ipc::server::serve_connection;
use teramind_ipc::transport::{connect, listen};
use teramind_ipc::{
    client::{IpcClient, StreamClient},
    IpcServer, Notify, Request, Response,
};

struct PingHandler;
#[async_trait]
impl IpcServer for PingHandler {
    async fn handle_request(&self, req: Request) -> Response {
        match req {
            Request::Ping => Response::Pong,
            _ => Response::Error("nope".into()),
        }
    }
    async fn handle_notify(&self, _n: Notify) {}
}

#[tokio::test]
async fn uds_ping_pong_end_to_end() {
    let tmp = tempdir().unwrap();
    let sock = tmp.path().join("t.sock");
    let listener = listen(&sock).unwrap();
    let handler = Arc::new(PingHandler);
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        serve_connection(stream, handler).await.unwrap();
    });

    let stream = connect(&sock).await.unwrap();
    let mut client = StreamClient::new(stream);
    let r = client.request(Request::Ping).await.unwrap();
    assert_eq!(r, Response::Pong);
    drop(client);
    let _ = server.await;
}
