//! The parity frontend as a stdio MEP guest.
//!
//! Requests go to the same `NativeExtension` protocol endpoint that an
//! in-process `NativeChannel` uses, so this process and the built-in answer a
//! compile call with the same code.

use morphir_extension_sdk::protocol::{ExtensionRequest, methods};
use std::io;

#[path = "support/frame.rs"]
mod frame;
#[path = "support/parity_frontend.rs"]
mod parity_frontend;

use frame::{read_frame, write_frame};
use parity_frontend::ParityFrontend;

fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let protocol = ParityFrontend::native().open_protocol();
    let stdin = io::stdin();
    let mut reader = stdin.lock();
    let stdout = io::stdout();
    let mut writer = stdout.lock();

    while let Some(body) = read_frame(&mut reader)? {
        let message: serde_json::Value = serde_json::from_slice(&body)?;
        if message.get("id").is_none() {
            if message["method"] == methods::EXIT {
                break;
            }
            continue;
        }
        let request: ExtensionRequest = serde_json::from_value(message)?;
        write_frame(&mut writer, &protocol.handle(request))?;
    }

    Ok(())
}
