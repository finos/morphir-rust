pub mod abi;
use serde::{Deserialize, Serialize};

/// Header containing metadata for the envelope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Header {
    /// Sequence number.
    pub seqnum: u64,
    /// Session ID (UUID).
    pub session_id: String,
    /// Message kind/hint.
    pub kind: String,
}

impl Default for Header {
    fn default() -> Self {
        Self {
            seqnum: 0,
            session_id: String::new(),
            kind: String::new(),
        }
    }
}

/// Log levels for the host log function.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(i32)]
pub enum LogLevel {
    Trace = 0,
    Debug = 1,
    Info = 2,
    Warn = 3,
    Error = 4,
}

/// Environment variable value variant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum EnvValue {
    Text(String),
    TextList(Vec<String>),
    Boolean(bool),
    U8(u8),
    U16(u16),
    U32(u32),
    U64(u64),
    S8(i8),
    S16(i16),
    S32(i32),
    S64(i64),
    F32(f32),
    F64(f64),
}

/// A content-type envelope for passing data across the Wasm boundary.
///
/// This is the fundamental unit of communication in the Morphir Wasm extension architecture.
/// All messages, models, and commands are wrapped in an `Envelope`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Envelope {
    /// The envelope header.
    pub header: Header,

    /// The MIME type or custom content type of the payload.
    /// Examples: "application/json", "application/morphir-ir+json", "text/plain".
    pub content_type: String,
    
    /// The raw byte content.
    pub content: Vec<u8>,
}

impl Envelope {
    /// Create a new envelope.
    pub fn new(content_type: impl Into<String>, content: impl Into<Vec<u8>>) -> Self {
        Self {
            header: Header::default(),
            content_type: content_type.into(),
            content: content.into(),
        }
    }

    /// Create a new envelope with a specific kind.
    pub fn with_kind(kind: impl Into<String>, content_type: impl Into<String>, content: impl Into<Vec<u8>>) -> Self {
        Self {
            header: Header {
                seqnum: 0,
                session_id: String::new(),
                kind: kind.into(),
            },
            content_type: content_type.into(),
            content: content.into(),
        }
    }

    /// Helper to create a JSON envelope.
    pub fn json<T: Serialize>(data: &T) -> Result<Self, serde_json::Error> {
        Ok(Self {
            header: Header::default(),
            content_type: "application/json".to_string(),
            content: serde_json::to_vec(data)?,
        })
    }
    
    /// Helper to decode JSON content.
    pub fn decode_json<'a, T: Deserialize<'a>>(&'a self) -> Result<T, serde_json::Error> {
        serde_json::from_slice(&self.content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Serialize, Deserialize, PartialEq, Debug)]
    struct TestPayload {
        message: String,
        count: i32,
    }

    #[test]
    fn test_envelope_json() {
        let payload = TestPayload {
            message: "Hello".to_string(),
            count: 42,
        };

        let envelope = Envelope::json(&payload).expect("Failed to create envelope");
        
        assert_eq!(envelope.content_type, "application/json");
        assert_eq!(envelope.header.seqnum, 0); // Default
        
        let decoded: TestPayload = envelope.decode_json().expect("Failed to decode envelope");
        assert_eq!(decoded, payload);
    }

    #[test]
    fn test_envelope_with_kind() {
        let envelope = Envelope::with_kind("req-kind", "text/plain", b"hello".to_vec());
        assert_eq!(envelope.header.kind, "req-kind");
        assert_eq!(envelope.content_type, "text/plain");
    }

    #[test]
    fn test_into_raw_parts() {
        let envelope = Envelope::with_kind("test-kind", "text", b"content".to_vec());
        let (hdr_ptr, hdr_len, ct_ptr, ct_len, c_ptr, c_len) = crate::abi::into_raw_parts(envelope);
        
        // Check lengths
        // Header {"seqnum":0,"kind":"test-kind"} -> ~30 bytes
        assert!(hdr_len > 0);
        assert_eq!(ct_len, 4);
        assert_eq!(c_len, 7);
        
        // Safety: Reconstructing `Vec` to avoid memory leak in test
        // unsafe {
        //     let _ = Vec::from_raw_parts(hdr_ptr as *mut u8, hdr_len as usize, hdr_len as usize);
        //     let _ = Vec::from_raw_parts(ct_ptr as *mut u8, ct_len as usize, ct_len as usize);
        //     let _ = Vec::from_raw_parts(c_ptr as *mut u8, c_len as usize, c_len as usize);
        // }
    }

    #[test]
    fn test_env_value_serialization() {
        let v = EnvValue::Text("hello".to_string());
        let json = serde_json::to_string(&v).unwrap();
        assert_eq!(json, r#"{"type":"Text","value":"hello"}"#);

        let v = EnvValue::TextList(vec!["a".to_string(), "b".to_string()]);
        let json = serde_json::to_string(&v).unwrap();
        assert_eq!(json, r#"{"type":"TextList","value":["a","b"]}"#);

        let v = EnvValue::U32(12345);
        let json = serde_json::to_string(&v).unwrap();
        assert_eq!(json, r#"{"type":"U32","value":12345}"#);
    }
}
