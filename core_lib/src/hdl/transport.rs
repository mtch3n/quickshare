//! Length-prefixed frame transport shared by inbound and outbound connections.
//! Works over any byte stream (TCP today, Bluetooth sockets later).

use anyhow::anyhow;
use futures::StreamExt;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio_util::codec::{FramedRead, LengthDelimitedCodec};

/// Upper bound for a single frame, to avoid allocation attacks.
const MAX_FRAME_LENGTH: usize = 5 * 1024 * 1024;

pub struct Transport {
    reader: FramedRead<Box<dyn AsyncRead + Send + Sync + Unpin>, LengthDelimitedCodec>,
    writer: Box<dyn AsyncWrite + Send + Sync + Unpin>,
}

impl std::fmt::Debug for Transport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Transport").finish_non_exhaustive()
    }
}

impl Transport {
    pub fn new<S: AsyncRead + AsyncWrite + Send + Sync + 'static>(stream: S) -> Self {
        let (reader, writer) = tokio::io::split(stream);
        let codec = LengthDelimitedCodec::builder()
            .length_field_length(4)
            .big_endian()
            .max_frame_length(MAX_FRAME_LENGTH)
            .new_codec();

        Self {
            reader: FramedRead::new(Box::new(reader), codec),
            writer: Box::new(writer),
        }
    }

    /// Reads the next frame. Cancel-safe: a partially received frame stays
    /// buffered, so this can be raced in `select!`.
    pub async fn read_frame(&mut self) -> Result<Vec<u8>, anyhow::Error> {
        match self.reader.next().await {
            Some(frame) => Ok(frame?.to_vec()),
            None => Err(anyhow!("connection closed by peer")),
        }
    }

    pub async fn write_frame(&mut self, data: &[u8]) -> Result<(), anyhow::Error> {
        let mut buf = Vec::with_capacity(4 + data.len());
        buf.extend_from_slice(&(data.len() as u32).to_be_bytes());
        buf.extend_from_slice(data);

        self.writer.write_all(&buf).await?;
        self.writer.flush().await?;
        Ok(())
    }
}
