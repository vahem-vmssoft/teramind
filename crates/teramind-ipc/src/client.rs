use crate::codec::{read_frame, write_frame};
use crate::proto::{Envelope, Notify, Payload, Request, Response};
use crate::IpcError;
use async_trait::async_trait;
use tokio::io::{AsyncRead, AsyncWrite};
use uuid::Uuid;

#[async_trait]
pub trait IpcClient: Send + Sync {
    async fn request(&mut self, req: Request) -> Result<Response, IpcError>;
    async fn notify(&mut self, n: Notify) -> Result<(), IpcError>;
}

pub struct StreamClient<S: AsyncRead + AsyncWrite + Unpin + Send + Sync> {
    stream: S,
}

impl<S: AsyncRead + AsyncWrite + Unpin + Send + Sync> StreamClient<S> {
    pub fn new(stream: S) -> Self { Self { stream } }
}

#[async_trait]
impl<S: AsyncRead + AsyncWrite + Unpin + Send + Sync> IpcClient for StreamClient<S> {
    async fn request(&mut self, req: Request) -> Result<Response, IpcError> {
        let env = Envelope { id: Uuid::new_v4(), payload: Payload::Request(req) };
        write_frame(&mut self.stream, &env).await?;
        let back = read_frame(&mut self.stream).await?;
        match back.payload {
            Payload::Response(r) => Ok(r),
            other => Err(IpcError::Protocol(format!("expected Response, got {:?}", other))),
        }
    }
    async fn notify(&mut self, n: Notify) -> Result<(), IpcError> {
        let env = Envelope { id: Uuid::new_v4(), payload: Payload::Notify(n) };
        write_frame(&mut self.stream, &env).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    #[tokio::test]
    async fn client_sends_notify_frame() {
        let (a, mut b) = duplex(8 * 1024);
        let mut client = StreamClient::new(a);
        let h = tokio::spawn(async move {
            crate::codec::read_frame(&mut b).await.unwrap()
        });
        use teramind_core::types::ingest_event::{EventEnvelope, IngestEvent};
        use teramind_core::ids::{ClientEventId, SessionId};
        use time::OffsetDateTime;
        let envelope = EventEnvelope {
            client_event_id: ClientEventId::new(),
            ts: OffsetDateTime::from_unix_timestamp(1_700_000_011).unwrap(),
            event: IngestEvent::UserPrompt {
                session_id: SessionId::new(), turn_ordinal: 0, prompt: "hi".into(),
            },
        };
        client.notify(Notify::Ingest(envelope.clone())).await.unwrap();
        let received = h.await.unwrap();
        match received.payload {
            Payload::Notify(Notify::Ingest(env)) => assert_eq!(env, envelope),
            _ => panic!("unexpected payload"),
        }
    }
}
