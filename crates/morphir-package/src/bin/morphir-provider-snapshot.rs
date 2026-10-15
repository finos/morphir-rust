//! One request on stdin, one binary Ion response on stdout. The supervising host
//! owns deadlines, process-tree teardown, explicit grants and scratch disposal.
use morphir_package::provider_snapshot::service::{MAX_REQUEST_BYTES, serve_request};
use std::io::{Read, Write};

#[tokio::main]
async fn main() -> std::process::ExitCode {
    if std::env::args_os().len() != 1 {
        eprintln!("metadata.restore_invalid_arguments");
        return std::process::ExitCode::FAILURE;
    }
    let mut request = Vec::new();
    if std::io::stdin()
        .take((MAX_REQUEST_BYTES + 1) as u64)
        .read_to_end(&mut request)
        .is_err()
    {
        eprintln!("metadata.restore_input_failed");
        return std::process::ExitCode::FAILURE;
    }
    match serve_request(&request).await {
        Ok(response) => match std::io::stdout().lock().write_all(&response) {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(_) => {
                eprintln!("metadata.restore_output_failed");
                std::process::ExitCode::FAILURE
            }
        },
        Err(error) => {
            eprintln!("{error}");
            std::process::ExitCode::FAILURE
        }
    }
}
