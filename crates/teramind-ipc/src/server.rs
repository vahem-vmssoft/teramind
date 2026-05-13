use crate::codec::{read_frame, write_frame};
use crate::proto::{Envelope, Notify, Payload, Request, Response};
use crate::IpcError;
use async_trait::async_trait;
use tokio::io::{AsyncRead, AsyncWrite};

#[async_trait]
pub trait IpcServer: Send + Sync + 'static {
    async fn handle_request(&self, req: Request) -> Response;
    async fn handle_notify(&self, n: Notify);
}

pub async fn serve_connection<S, H>(mut stream: S, handler: std::sync::Arc<H>) -> Result<(), IpcError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
    H: IpcServer,
{
    loop {
        let env = match read_frame(&mut stream).await {
            Ok(e) => e,
            Err(IpcError::Io(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(e) => return Err(e),
        };
        match env.payload {
            Payload::Request(req) => {
                let resp = handler.handle_request(req).await;
                let out = Envelope { id: env.id, payload: Payload::Response(resp) };
                write_frame(&mut stream, &out).await?;
            }
            Payload::Notify(n) => {
                handler.handle_notify(n).await;
            }
            Payload::Response(_) => {
                return Err(IpcError::Protocol("client sent Response".into()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{IpcClient, StreamClient};
    use std::sync::Arc;
    use tokio::io::duplex;

    struct Echo;
    #[async_trait]
    impl IpcServer for Echo {
        async fn handle_request(&self, req: Request) -> Response {
            match req {
                Request::Ping => Response::Pong,
                _ => Response::Error("unsupported".into()),
            }
        }
        async fn handle_notify(&self, _n: Notify) {}
    }

    #[tokio::test]
    async fn ping_pong_roundtrips() {
        let (a, b) = duplex(8 * 1024);
        let handler = Arc::new(Echo);
        let _server = tokio::spawn(serve_connection(b, handler));
        let mut client = StreamClient::new(a);
        let r = client.request(Request::Ping).await.unwrap();
        assert_eq!(r, Response::Pong);
    }
}
