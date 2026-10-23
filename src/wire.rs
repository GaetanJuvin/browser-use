use serde_json::Value;
use std::io::{self, Read, Write};
pub const MAX_FRAME: usize = 1024 * 1024;
fn checked_length(header: [u8; 4]) -> io::Result<usize> {
    let n = u32::from_ne_bytes(header) as usize;
    if n == 0 || n > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame length outside 1..=1048576 bytes",
        ));
    }
    Ok(n)
}

pub fn read_frame(reader: &mut impl Read) -> io::Result<Option<Value>> {
    let mut header = [0; 4];
    if reader.read(&mut header[..1])? == 0 {
        return Ok(None);
    }
    reader.read_exact(&mut header[1..])?;
    let mut payload = vec![0; checked_length(header)?];
    reader.read_exact(&mut payload)?;
    Ok(Some(serde_json::from_slice(&payload)?))
}

fn encode(value: &Value) -> io::Result<Vec<u8>> {
    let payload = serde_json::to_vec(value)?;
    if payload.len() > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "frame exceeds 1 MiB",
        ));
    }
    let mut frame = (payload.len() as u32).to_ne_bytes().to_vec();
    frame.extend(payload);
    Ok(frame)
}

pub fn write_frame(writer: &mut impl Write, value: &Value) -> io::Result<()> {
    writer.write_all(&encode(value)?)?;
    writer.flush()
}

pub async fn read_async(
    reader: &mut (impl tokio::io::AsyncRead + Unpin),
) -> io::Result<Option<Value>> {
    use tokio::io::AsyncReadExt;
    let mut header = [0; 4];
    if reader.read(&mut header[..1]).await? == 0 {
        return Ok(None);
    }
    reader.read_exact(&mut header[1..]).await?;
    let mut payload = vec![0; checked_length(header)?];
    reader.read_exact(&mut payload).await?;
    Ok(Some(serde_json::from_slice(&payload)?))
}

pub async fn write_async(
    writer: &mut (impl tokio::io::AsyncWrite + Unpin),
    value: &Value,
) -> io::Result<()> {
    use tokio::io::AsyncWriteExt;
    writer.write_all(&encode(value)?).await?;
    writer.flush().await
}
