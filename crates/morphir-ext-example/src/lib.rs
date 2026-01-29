//! Example Morphir extension - a simple counter.
//!
//! This extension demonstrates the TEA pattern with a counter model.

// Generate guest bindings from WIT
wit_bindgen::generate!({
    world: "extension",
    path: "../morphir-ext-core/wit",
});

use exports::morphir::ext::program::Guest;
use morphir::ext::envelope::{Envelope, Header};
use morphir::ext::runtime;

/// Counter model state.
#[derive(Default, serde::Serialize, serde::Deserialize)]
struct CounterModel {
    count: i64,
}

/// Messages the counter can handle.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(tag = "type")]
enum CounterMsg {
    Increment,
    Decrement,
    Reset,
}

/// Extension implementation.
struct CounterExtension;

impl Guest for CounterExtension {
    fn init(init_data: Envelope) -> (Envelope, Envelope) {
        runtime::log(runtime::LogLevel::Info, "Counter extension initializing");

        // Parse initial count from init_data if provided
        let initial_count = if !init_data.content.is_empty() {
            serde_json::from_slice::<CounterModel>(&init_data.content)
                .map(|m| m.count)
                .unwrap_or(0)
        } else {
            0
        };

        let model = CounterModel {
            count: initial_count,
        };
        let model_envelope = make_json_envelope("model", &model);

        // No initial commands
        let commands_envelope = make_json_envelope("commands", &Vec::<String>::new());

        (model_envelope, commands_envelope)
    }

    fn update(msg: Envelope, model: Envelope) -> (Envelope, Envelope) {
        // Parse current model
        let mut current_model: CounterModel =
            serde_json::from_slice(&model.content).unwrap_or_default();

        // Parse message
        if let Ok(counter_msg) = serde_json::from_slice::<CounterMsg>(&msg.content) {
            match counter_msg {
                CounterMsg::Increment => {
                    current_model.count += 1;
                    runtime::log(
                        runtime::LogLevel::Debug,
                        &format!("Incremented to {}", current_model.count),
                    );
                }
                CounterMsg::Decrement => {
                    current_model.count -= 1;
                    runtime::log(
                        runtime::LogLevel::Debug,
                        &format!("Decremented to {}", current_model.count),
                    );
                }
                CounterMsg::Reset => {
                    current_model.count = 0;
                    runtime::log(runtime::LogLevel::Info, "Counter reset");
                }
            }
        }

        let new_model_envelope = make_json_envelope("model", &current_model);
        let commands_envelope = make_json_envelope("commands", &Vec::<String>::new());

        (new_model_envelope, commands_envelope)
    }

    fn subscriptions(model: Envelope) -> Envelope {
        // No subscriptions for counter
        let _ = model;
        make_json_envelope("subscriptions", &Vec::<String>::new())
    }

    fn get_capabilities() -> Envelope {
        let info = serde_json::json!({
            "name": "Counter Extension",
            "version": "0.1.0",
            "description": "A simple counter demonstrating the TEA pattern",
            "author": "Morphir Team"
        });
        make_json_envelope("capabilities", &info)
    }
}

/// Helper to create a JSON envelope.
fn make_json_envelope<T: serde::Serialize>(kind: &str, data: &T) -> Envelope {
    let content = serde_json::to_vec(data).unwrap_or_default();
    Envelope {
        header: Header {
            seqnum: 0,
            session_id: String::new(),
            kind: Some(kind.to_string()),
        },
        content_type: "application/json".to_string(),
        content,
    }
}

// Export the extension
export!(CounterExtension);
