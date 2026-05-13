use crate::proto::{Envelope, Payload, Request};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub async fn write_frame<W: AsyncWrite + Unpin>(w: &mut W, env: &Envelope) -> Result<(), crate::IpcError> {
    let bytes = serde_json::to_vec(env)?;
    let len = (bytes.len() as u32).to_be_bytes();
    w.write_all(&len).await?;
    w.write_all(&bytes).await?;
    w.flush().await?;
    Ok(())
}

pub async fn read_frame<R: AsyncRead + Unpin>(r: &mut R) -> Result<Envelope, crate::IpcError> {
    let mut len_buf = [0u8; 4];
    r.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > 16 * 1024 * 1024 {
        return Err(crate::IpcError::Protocol(format!("frame too large: {len}")));
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf).await?;
    Ok(serde_json::from_slice(&buf)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;
    use uuid::Uuid;
    #[tokio::test]
    async fn frame_roundtrip() {
        let (mut a, mut b) = duplex(64 * 1024);
        let env = Envelope { id: Uuid::new_v4(), payload: Payload::Request(Request::Status) };
        write_frame(&mut a, &env).await.unwrap();
        drop(a);
        let back = read_frame(&mut b).await.unwrap();
        assert_eq!(env, back);
    }
}
