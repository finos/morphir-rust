//! Wire framing for the Morphir Extension Protocol over byte streams.

use morphir_host::{GuestResponsePolicy, HostError, ResponseByteLimit};
use serde::Serialize;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Maximum request or response payload accepted by built-in MEP transports.
pub const MAX_MEP_PAYLOAD_BYTES: u32 = 64 * 1024 * 1024;
const MAX_MEP_HEADER_BYTES: usize = 8192;

/// Write one host request or notification, bounded before any stream write.
pub async fn write_frame<W, T>(writer: &mut W, value: &T) -> Result<(), HostError>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let mut body = RequestBody(Vec::new());
    serde_json::to_writer(&mut body, value)?;
    let body = body.0;
    writer
        .write_all(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes())
        .await?;
    writer.write_all(&body).await?;
    writer.flush().await?;
    Ok(())
}

/// Serialize into bounded host-owned memory, including JSON escaping overhead.
struct RequestBody(Vec<u8>);

impl std::io::Write for RequestBody {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > (MAX_MEP_PAYLOAD_BYTES as usize).saturating_sub(self.0.len()) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Extension request exceeds the {MAX_MEP_PAYLOAD_BYTES} byte limit"),
            ));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Read one `Content-Length`-framed message body.
pub async fn read_frame<R>(reader: &mut R) -> Result<Vec<u8>, HostError>
where
    R: AsyncBufRead + Unpin,
{
    read_frame_with_limit(reader, GuestResponsePolicy::Legacy.response_limit("")).await
}

/// Read a response under the bound selected for its originating request.
///
/// Reject an oversized declared length before allocating or reading the body.
pub async fn read_frame_with_limit<R>(
    reader: &mut R,
    limit: ResponseByteLimit,
) -> Result<Vec<u8>, HostError>
where
    R: AsyncBufRead + Unpin,
{
    let mut content_length = None;
    let mut header_bytes = 0;
    loop {
        let mut header = String::new();
        let remaining = MAX_MEP_HEADER_BYTES - header_bytes;
        let read = (&mut *reader)
            .take(remaining as u64 + 1)
            .read_line(&mut header)
            .await?;
        header_bytes += read;
        if header_bytes > MAX_MEP_HEADER_BYTES {
            return Err(HostError::Invalid(format!(
                "Extension frame exceeds the {MAX_MEP_HEADER_BYTES} header byte limit"
            )));
        }
        if read == 0 {
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
            limit.check_len(length)?;
            content_length = Some(length);
        }
    }

    let content_length = content_length
        .ok_or_else(|| HostError::Invalid("Extension frame omitted Content-Length".to_string()))?;
    let mut body = vec![0; content_length];
    reader.read_exact(&mut body).await?;
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{BufReader, duplex};

    #[tokio::test]
    async fn framed_responses_have_one_finite_aggregate_header_budget() {
        for header in [
            "x".repeat(8193),
            format!("{}Content-Length: 0\r\n\r\n", "X: 1\r\n".repeat(1500)),
        ] {
            let error = read_frame(&mut BufReader::new(header.as_bytes()))
                .await
                .unwrap_err();
            assert!(error.to_string().contains("header byte limit"), "{error}");
        }
        let suffix = "\r\nContent-Length: 0\r\n\r\n";
        let exact = format!("X: {}{suffix}", "a".repeat(8192 - 3 - suffix.len()));
        assert_eq!(exact.len(), 8192);
        assert!(
            read_frame(&mut BufReader::new(exact.as_bytes()))
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn request_frames_keep_the_legacy_bound_before_any_write() {
        let exact = "x".repeat(MAX_MEP_PAYLOAD_BYTES as usize - 2);
        let mut output = Vec::new();
        write_frame(&mut output, &exact).await.unwrap();
        let header = format!("Content-Length: {}\r\n\r\n", MAX_MEP_PAYLOAD_BYTES);
        assert_eq!(&output[..header.len()], header.as_bytes());
        assert_eq!(output.len(), header.len() + MAX_MEP_PAYLOAD_BYTES as usize);
        drop(output);
        let one_over = "x".repeat(MAX_MEP_PAYLOAD_BYTES as usize - 1);
        let mut refused = Vec::new();
        assert!(write_frame(&mut refused, &one_over).await.is_err());
        assert!(
            refused.is_empty(),
            "no header or body may be written for an oversized request"
        );
    }

    #[tokio::test]
    async fn an_artifact_header_above_its_bound_is_refused_without_reading_a_body() {
        let bound = morphir_host::GuestResponsePolicy::GeneratedArtifacts
            .response_limit(morphir_extension_sdk::protocol::methods::GENERATE);
        let header = format!("Content-Length: {}\r\n\r\n", bound.bytes() as u64 + 1);
        let mut reader = BufReader::new(header.as_bytes());
        let error = read_frame_with_limit(&mut reader, bound).await.unwrap_err();
        assert!(error.to_string().contains(&bound.bytes().to_string()));
        assert!(!error.to_string().contains("early eof"));
    }

    #[tokio::test]
    async fn artifact_frames_can_exceed_the_frozen_legacy_bound() {
        let bound = morphir_host::GuestResponsePolicy::GeneratedArtifacts
            .response_limit(morphir_extension_sdk::protocol::methods::GENERATE);
        let bytes = MAX_MEP_PAYLOAD_BYTES as usize + 1;
        let (mut writer, reader) = duplex(65536);
        let writing = tokio::spawn(async move {
            writer
                .write_all(format!("Content-Length: {bytes}\r\n\r\n").as_bytes())
                .await
                .unwrap();
            let chunk = [b'x'; 65536];
            for _ in 0..bytes / chunk.len() {
                writer.write_all(&chunk).await.unwrap();
            }
            writer
                .write_all(&chunk[..bytes % chunk.len()])
                .await
                .unwrap();
        });
        let body = read_frame_with_limit(&mut BufReader::new(reader), bound)
            .await
            .unwrap();
        writing.await.unwrap();
        assert_eq!(body.len(), bytes);
        assert!(body.iter().all(|byte| *byte == b'x'));
        let header = format!("Content-Length: {bytes}\r\n\r\n");
        assert!(
            read_frame(&mut BufReader::new(header.as_bytes()))
                .await
                .unwrap_err()
                .to_string()
                .contains("limit")
        );
    }

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
