/// Length-prefixed protobuf framing for the notification socket.
///
/// Wire format: 4-byte big-endian length + protobuf body.

use bytes::BufMut;
use prost::Message;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const MAX_MESSAGE_SIZE: usize = 1024 * 1024;
const HEADER_SIZE: usize = 4;

/// Encode a protobuf message with length prefix.
pub fn encode_message<M: Message>(msg: &M) -> Result<Vec<u8>, String> {
    let body_len = msg.encoded_len();
    if body_len > MAX_MESSAGE_SIZE {
        return Err(format!("message too large: {body_len}"));
    }
    let mut buf = Vec::with_capacity(HEADER_SIZE + body_len);
    buf.put_u32(body_len as u32);
    msg.encode(&mut buf).map_err(|e| format!("encode: {e}"))?;
    Ok(buf)
}

/// Read a single length-prefixed message. Returns None on EOF.
pub async fn read_message<R, M>(reader: &mut R) -> Result<Option<M>, String>
where
    R: AsyncReadExt + Unpin,
    M: Message + Default,
{
    let mut len_buf = [0u8; HEADER_SIZE];
    match reader.read_exact(&mut len_buf).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(format!("read header: {e}")),
    }
    let body_len = u32::from_be_bytes(len_buf) as usize;
    if body_len > MAX_MESSAGE_SIZE {
        return Err(format!("message too large: {body_len}"));
    }
    let mut body = vec![0u8; body_len];
    reader
        .read_exact(&mut body)
        .await
        .map_err(|e| format!("read body: {e}"))?;
    M::decode(&body[..]).map(Some).map_err(|e| format!("decode: {e}"))
}

/// Write a length-prefixed message.
pub async fn write_message<W, M>(writer: &mut W, msg: &M) -> Result<(), String>
where
    W: AsyncWriteExt + Unpin,
    M: Message,
{
    let buf = encode_message(msg)?;
    writer
        .write_all(&buf)
        .await
        .map_err(|e| format!("write: {e}"))?;
    writer
        .flush()
        .await
        .map_err(|e| format!("flush: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use notification_proto::proto;

    #[test]
    fn test_encode_decode() {
        let msg = proto::ClientMessage {
            msg: Some(proto::client_message::Msg::Hello(proto::ClientHello {
                client_name: "test".into(),
            })),
        };
        let encoded = encode_message(&msg).unwrap();
        assert_eq!(
            u32::from_be_bytes(encoded[..4].try_into().unwrap()) as usize,
            msg.encoded_len()
        );
        let decoded = proto::ClientMessage::decode(&encoded[HEADER_SIZE..]).unwrap();
        match decoded.msg {
            Some(proto::client_message::Msg::Hello(h)) => assert_eq!(h.client_name, "test"),
            _ => panic!("wrong type"),
        }
    }

    #[tokio::test]
    async fn test_read_write_roundtrip() {
        let msg = proto::ClientMessage {
            msg: Some(proto::client_message::Msg::Dismiss(
                proto::DismissNotification { id: 42 },
            )),
        };
        let mut buf = Vec::new();
        write_message(&mut buf, &msg).await.unwrap();

        let mut cursor = std::io::Cursor::new(buf);
        let decoded: Option<proto::ClientMessage> = read_message(&mut cursor).await.unwrap();
        match decoded.unwrap().msg {
            Some(proto::client_message::Msg::Dismiss(d)) => assert_eq!(d.id, 42),
            _ => panic!("wrong type"),
        }
    }
}
