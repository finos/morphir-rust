//! Wire framing for the Morphir Extension Protocol over byte streams.

use morphir_host::HostError;
use serde::Serialize;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Maximum request or response payload accepted by built-in MEP transports.
pub const MAX_MEP_PAYLOAD_BYTES: u32 = 64 * 1024 * 1024;

/// Write one `Content-Length`-framed JSON message.
pub async fn write_frame<W, T>(writer: &mut W, value: &T) -> Result<(), HostError>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let body = serde_json::to_vec(value)?;
    writer
        .write_all(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes())
        .await
        .map_err(|error| HostError::Invalid(error.to_string()))?;
    writer
        .write_all(&body)
        .await
        .map_err(|error| HostError::Invalid(error.to_string()))?;
    writer
        .flush()
        .await
        .map_err(|error| HostError::Invalid(error.to_string()))?;
    Ok(())
}

/// Read one `Content-Length`-framed message body.
pub async fn read_frame<R>(reader: &mut R) -> Result<Vec<u8>, HostError>
where
    R: AsyncBufRead + Unpin,
{
    let mut content_length = None;
    loop {
        let mut header = String::new();
        if reader
            .read_line(&mut header)
            .await
            .map_err(|error| HostError::Invalid(error.to_string()))?
            == 0
        {
            return Err(HostError::Invalid(
                "Extension process closed stdout before a response frame".to_string(),
            ));
        }
        if header == "\r\n" || header == "\n" {
            break;
        }
        let (name, value) = header.split_once(':').ok_or_else(|| {
            HostError::Invalid(format!(
                "Invalid extension protocol header: {}",
                header.trim_end()
            ))
        })?;
        if name.eq_ignore_ascii_case("content-length") {
            if content_length.is_some() {
                return Err(HostError::Invalid(
                    "Extension frame repeated Content-Length".to_string(),
                ));
            }
            let length = value
                .trim()
                .parse::<usize>()
                .map_err(|error| HostError::Invalid(format!("Invalid Content-Length: {error}")))?;
            if length > MAX_MEP_PAYLOAD_BYTES as usize {
                return Err(HostError::Invalid(format!(
                    "Extension frame exceeds the {} byte limit",
                    MAX_MEP_PAYLOAD_BYTES
                )));
            }
            content_length = Some(length);
        }
    }

    let content_length = content_length
        .ok_or_else(|| HostError::Invalid("Extension frame omitted Content-Length".to_string()))?;
    let mut body = vec![0; content_length];
    reader
        .read_exact(&mut body)
        .await
        .map_err(|error| HostError::Invalid(error.to_string()))?;
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{BufReader, duplex};

    #[tokio::test]
    async fn content_length_frames_round_trip_formatted_json() {
        let (mut writer, reader) = duplex(1024);
        let value = serde_json::json!({ "message": "line one\nline two" });
        let expected = value.clone();
        let writing = tokio::spawn(async move { write_frame(&mut writer, &value).await });
        let body = read_frame(&mut BufReader::new(reader))
            .await
            .expect("the frame should parse");
        writing.await.expect("the writer task should join").unwrap();

        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
            expected
        );
    }

    #[tokio::test]
    async fn stdout_logs_are_rejected_as_protocol_headers() {
        let (mut writer, reader) = duplex(1024);
        writer.write_all(b"accidental log line\n").await.unwrap();
        drop(writer);

        let error = read_frame(&mut BufReader::new(reader))
            .await
            .expect_err("stdout logs must not be treated as protocol data");
        assert!(
            error
                .to_string()
                .contains("Invalid extension protocol header")
        );
    }
}
