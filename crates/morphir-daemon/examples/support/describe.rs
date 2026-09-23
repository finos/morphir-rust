//! Strict lifecycle fixture for process description tests.

use morphir_extension_sdk::protocol::{
    ExtensionRequest, ExtensionResponse, RpcError, error_codes, methods,
};
use serde_json::{Value, json};

pub struct DescribeFixture {
    mode: String,
    next: usize,
}

impl DescribeFixture {
    pub fn from_environment() -> Option<Self> {
        std::env::var("MEP_FIXTURE_DESCRIBE")
            .ok()
            .map(|mode| Self { mode, next: 0 })
    }

    pub fn accept(&mut self, message: &Value) -> Result<(), Box<dyn std::error::Error>> {
        let sequence: &[&str] = if matches!(
            self.mode.as_str(),
            "method-not-found" | "not-initialized" | "legacy-not-initialized"
        ) {
            &[
                methods::DESCRIBE,
                methods::INITIALIZE,
                methods::INITIALIZED,
                methods::CAPABILITIES,
                methods::SHUTDOWN,
                methods::EXIT,
            ]
        } else {
            &[methods::DESCRIBE, methods::EXIT]
        };
        let expected = sequence.get(self.next).ok_or("unexpected extra request")?;
        if message["method"].as_str() != Some(expected) {
            return Err(format!("expected {expected}, received {}", message["method"]).into());
        }
        let notification = matches!(*expected, methods::INITIALIZED | methods::EXIT);
        if notification == message.get("id").is_some() {
            return Err("incorrect request/notification shape".into());
        }
        self.next += 1;
        Ok(())
    }

    pub fn adjust(&self, request: &ExtensionRequest, response: &mut ExtensionResponse) {
        if request.method == methods::CAPABILITIES {
            response.result.as_mut().unwrap()["backend"]["future"] = json!("preserved");
        }
        if request.method != methods::DESCRIBE {
            return;
        }
        let error = match self.mode.as_str() {
            "method-not-found" => Some(RpcError::method_not_found(methods::DESCRIBE)),
            "not-initialized" => Some(RpcError {
                code: error_codes::NOT_INITIALIZED,
                message: "morphir.extension.describe is not allowed yet".into(),
                data: None,
            }),
            "legacy-not-initialized" => {
                Some(RpcError::invalid_request("Extension is not initialized"))
            }
            "internal-error" => Some(RpcError::internal_error("deliberate failure")),
            _ => None,
        };
        if let Some(error) = error {
            *response = ExtensionResponse::error(request.id, error);
            return;
        }
        let result = response.result.as_mut().unwrap();
        match self.mode.as_str() {
            "critical" => result["critical"] = json!(["capabilities.backend.future"]),
            "version" => result["statementVersion"] = json!("0.1.0-draft.2"),
            "wrong-id" => result["extension"]["id"] = json!("different"),
            "requires-host" => {
                result["requires"] = json!({"host":[">=99.0.0"]});
                result["critical"] = json!(["requires.host"]);
            }
            _ => {}
        }
    }
}
