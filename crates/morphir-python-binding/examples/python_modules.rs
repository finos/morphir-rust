//! Compile and generate multiple modules with imported ADTs and tuple aliases through MEP.
use morphir_extension_sdk::{prelude::*, protocol::methods};
use morphir_python_binding::PythonExtension;

fn main() -> Result<()> {
    let extension = NativeExtension::frontend_backend(PythonExtension)?;
    let request = CompileRequest {
        language_id: "python".into(),
        documents: vec![
            SourceDocument {
                uri: "models.py".into(),
                language_id: "python".into(),
                version: 1,
                text: format!(
                    "{}\n{}",
                    include_str!("../tests/fixtures/models.py"),
                    include_str!("../tests/fixtures/tuples.py")
                ),
            },
            SourceDocument {
                uri: "rules.py".into(),
                language_id: "python".into(),
                version: 1,
                text: include_str!("../tests/fixtures/modules/rules.py").into(),
            },
        ],
        package: CompilePackage {
            name: "acme/example".into(),
            exposed_modules: None,
        },
        dependencies: vec![],
        options: CompileOptions {
            types_only: false,
            ir_version: std::env::args().nth(1).unwrap_or_else(|| "4".into()),
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
