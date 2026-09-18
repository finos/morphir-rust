//! Print a fixture's IR and generated Python using the same MEP endpoint as a host.
use morphir_extension_sdk::{prelude::*, protocol::methods};
use morphir_python_binding::PythonExtension;

fn main() -> Result<()> {
    let extension = NativeExtension::frontend_backend(PythonExtension)?;
    let (uri, text) = match std::env::args().nth(1) {
        Some(path) => {
            let text = std::fs::read_to_string(&path)
                .map_err(|error| ExtensionError::ExecutionFailed(format!("{path}: {error}")))?;
            (path, text)
        }
        None => (
            "models.py".into(),
            format!(
                "{}\n{}\n{}",
                include_str!("../tests/fixtures/models.py"),
                include_str!("../tests/fixtures/conditionals.py"),
                include_str!("../tests/fixtures/tuples.py")
            ),
        ),
    };
    let request = CompileRequest {
        language_id: "python".into(),
        documents: vec![SourceDocument {
            uri,
            language_id: "python".into(),
            version: 1,
            text,
        }],
        package: CompilePackage {
            name: "acme/example".into(),
            exposed_modules: None,
        },
        dependencies: vec![],
        options: CompileOptions {
            types_only: false,
            ir_version: "4".into(),
            ..Default::default()
        },
        baseline: None,
    };
    let compiled =
        extension
            .protocol()
            .handle(ExtensionRequest::new(methods::COMPILE, request, 1)?);
    let compiled: CompileResult = serde_json::from_value(
        compiled
            .result
            .ok_or_else(|| ExtensionError::ExecutionFailed(format!("{:?}", compiled.error)))?,
    )?;
    let ir = compiled
        .ir
        .ok_or_else(|| ExtensionError::ExecutionFailed(format!("{:?}", compiled.diagnostics)))?;
    let generated = extension.protocol().handle(ExtensionRequest::new(
        methods::GENERATE,
        GenerateRequest {
            ir: ir.clone(),
            target: "python".into(),
            options: Default::default(),
        },
        2,
    )?);
    let generated: GenerateResult = serde_json::from_value(
        generated
            .result
            .ok_or_else(|| ExtensionError::ExecutionFailed(format!("{:?}", generated.error)))?,
    )?;
    if !generated.success {
        return Err(ExtensionError::ExecutionFailed(format!(
            "{:?}",
            generated.diagnostics
        )));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(
            &serde_json::json!({ "ir": ir, "artifacts": generated.artifacts })
        )?
    );
    Ok(())
}
