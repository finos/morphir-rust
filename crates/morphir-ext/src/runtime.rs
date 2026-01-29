//! Runtime types and traits for Morphir extensions.

use morphir_ext_core::Envelope;
use anyhow::Result;
use serde::{Deserialize, Serialize};

/// WIT-compatible envelope type alias.
/// Used for interfacing with WebAssembly Component Model extensions.
pub type WitEnvelope = Envelope;

/// Environment variable value types supported by the runtime.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "value")]
pub enum EnvValue {
    Text(String),
    TextList(Vec<String>),
    Boolean(bool),
    U8(u8),
    U16(u16),
    U32(u32),
    U64(u64),
    I8(i8),
    I16(i16),
    I32(i32),
    I64(i64),
    F32(f32),
    F64(f64),
}

/// Log levels for extension logging.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

/// Trait for extension runtime implementations.
///
/// This abstracts over different WebAssembly runtime engines (Extism, Wasmtime, etc.)
/// and provides a common interface for executing extension programs.
pub trait ExtensionRuntime: Send {
    /// Call a function in the extension with an envelope input.
    fn call_envelope(&mut self, func: &str, input: &Envelope) -> Result<Envelope>;

    /// Initialize the extension with startup flags.
    fn init(&mut self, flags: Envelope) -> Result<(Envelope, Envelope)> {
        let output = self.call_envelope("init", &flags)?;
        // Parse the output as JSON containing model and cmds
        let result: InitResult = output.as_json()?;
        Ok((
            Envelope::json(&result.model)?,
            Envelope::json(&result.cmds)?,
        ))
    }

    /// Update extension state with a message.
    fn update(&mut self, msg: Envelope, model: Envelope) -> Result<(Envelope, Envelope)> {
        // Create input envelope containing both msg and model
        let input = Envelope::json(&UpdateInput { msg, model })?;
        let output = self.call_envelope("update", &input)?;
        let result: UpdateResult = output.as_json()?;
        Ok((
            Envelope::json(&result.model)?,
            Envelope::json(&result.cmds)?,
        ))
    }

    /// Get active subscriptions for the current model.
    fn subscriptions(&mut self, model: Envelope) -> Result<Envelope> {
        self.call_envelope("subscriptions", &model)
    }
}

/// Result from init function.
#[derive(Debug, Serialize, Deserialize)]
struct InitResult {
    model: serde_json::Value,
    cmds: serde_json::Value,
}

/// Input for update function.
#[derive(Debug, Serialize, Deserialize)]
struct UpdateInput {
    msg: Envelope,
    model: Envelope,
}

/// Result from update function.
#[derive(Debug, Serialize, Deserialize)]
struct UpdateResult {
    model: serde_json::Value,
    cmds: serde_json::Value,
}

/// Represents a loaded extension instance.
///
/// This wraps a runtime and provides TEA-style state management.
pub struct ExtensionInstance {
    runtime: Box<dyn ExtensionRuntime>,
    current_model: Option<Envelope>,
    env_vars: std::collections::HashMap<String, EnvValue>,
}

impl ExtensionInstance {
    /// Create a new extension instance with the given runtime.
    pub fn new(runtime: Box<dyn ExtensionRuntime>) -> Self {
        Self {
            runtime,
            current_model: None,
            env_vars: std::collections::HashMap::new(),
        }
    }

    /// Initialize the extension, returning both model and commands.
    pub fn init(&mut self, flags: Envelope) -> Result<(Envelope, Envelope)> {
        let (model, cmds) = self.runtime.init(flags)?;
        self.current_model = Some(model.clone());
        Ok((model, cmds))
    }

    /// Send a message to the extension, returning both model and commands.
    pub fn update(&mut self, msg: Envelope, model: Envelope) -> Result<(Envelope, Envelope)> {
        let (new_model, cmds) = self.runtime.update(msg, model)?;
        self.current_model = Some(new_model.clone());
        Ok((new_model, cmds))
    }

    /// Get current subscriptions for the given model.
    pub fn subscriptions(&mut self, model: Envelope) -> Result<Envelope> {
        self.runtime.subscriptions(model)
    }

    /// Get extension capabilities/info.
    pub fn info(&mut self) -> Result<Envelope> {
        self.runtime.call_envelope("get_capabilities", &Envelope::json(&serde_json::json!({}))?)
    }

    /// Set an environment variable.
    pub fn set_env_var(&mut self, name: String, value: EnvValue) {
        self.env_vars.insert(name, value);
    }

    /// Get an environment variable.
    pub fn get_env_var(&self, name: &str) -> Option<&EnvValue> {
        self.env_vars.get(name)
    }

    /// Get the current model.
    pub fn model(&self) -> Option<&Envelope> {
        self.current_model.as_ref()
    }
}
