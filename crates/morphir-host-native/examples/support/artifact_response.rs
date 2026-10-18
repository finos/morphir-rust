//! Large SDK response fixtures. This is not a production backend target.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use morphir_extension_sdk::{Artifact, GenerateResult};

pub fn generate(kind: &str) -> Option<GenerateResult> {
    let artifacts = match kind {
        "binary" => [0, 37, 83, 199]
            .into_iter()
            .enumerate()
            .map(|(index, seed)| {
                let bytes: Vec<u8> = (0..16 * 1024 * 1024)
                    .map(|offset| ((offset % 256) ^ ((offset / 65536 + seed) % 256)) as u8)
                    .collect();
                Artifact {
                    path: format!("part-{index}.bin"),
                    content: STANDARD.encode(bytes),
                    binary: true,
                }
            })
            .collect(),
        "escaped-text" => (0..4)
            .map(|index| Artifact {
                path: format!("part-{index}.txt"),
                content: "\0".repeat(16 * 1024 * 1024),
                binary: false,
            })
            .collect(),
        _ => return None,
    };
    Some(GenerateResult {
        success: true,
        artifacts,
        diagnostics: vec![],
    })
}
