use morphir_ext::{
    ExtensionRuntime, ExtensionActor, 
    InitMsg, UpdateMsg,
    WitEnvelope, WitHeader
};
use kameo::actor::Spawn;
use std::path::PathBuf;

#[tokio::test]
async fn test_counter_extension_integration() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Locate the built Wasm module
    let mut wasm_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    wasm_path.pop(); // crates/
    wasm_path.pop(); // root
    wasm_path.push("target");
    wasm_path.push("wasm32-unknown-unknown");
    wasm_path.push("debug");
    wasm_path.push("morphir_ext_example.wasm");

    if !wasm_path.exists() {
        println!("Skipping test: Wasm module not found at {:?}", wasm_path);
        return Ok(());
    }

    let module_bytes = std::fs::read(&wasm_path).expect("Failed to read wasm module");

    // 2. Encode module into component
    let mut wit_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    wit_path.pop();
    wit_path.push("morphir-ext-core");
    wit_path.push("wit");

    // Resolve WIT world
    let mut resolve = wit_parser::Resolve::default();
    let pkg = resolve.push_dir(&wit_path)?.0;
    let world_id = resolve.select_world(&[pkg], Some("extension"))?;

    // Embed metadata into the core module
    let mut module_bytes = module_bytes;
    wit_component::embed_component_metadata(
        &mut module_bytes,
        &resolve,
        world_id,
        wit_component::StringEncoding::UTF8,
    )?;

    // Encode module into component
    let component_bytes = wit_component::ComponentEncoder::default()
        .module(&module_bytes)?
        .validate(true)
        .encode()?;

    // 3. Initialize Runtime
    let runtime = ExtensionRuntime::new().expect("Failed to create runtime");

    // 4. Load into Actor
    let actor = ExtensionActor::from_bytes(&runtime, &component_bytes).expect("Failed to load actor");
    let actor_ref = ExtensionActor::spawn(actor); 

    // 5. Initialize Extension (Init)
    let init_data = WitEnvelope {
        header: WitHeader {
            seqnum: 0,
            session_id: "test-session".to_string(),
            kind: None,
        },
        content_type: "application/json".to_string(),
        content: vec![], 
    };

    let model_cmds = actor_ref.ask(InitMsg { init_data }).await.unwrap();
    let model = model_cmds.model;

    // Verify initial model (count: 0)
    let model_json: serde_json::Value = serde_json::from_slice(&model.content).expect("Failed to parse model JSON");
    assert_eq!(model_json["count"], 0);

    // 6. Send Increment Message (Update)
    let increment_msg = WitEnvelope {
        header: WitHeader {
            seqnum: 1,
            session_id: "test-session".to_string(),
            kind: Some("update".to_string()),
        },
        content_type: "application/json".to_string(),
        content: serde_json::to_vec(&serde_json::json!({"type": "Increment"})).expect("Failed to serialize msg"),
    };

    let model_cmds = actor_ref.ask(UpdateMsg { msg: increment_msg, model: model.clone() }).await.unwrap();
    let new_model = model_cmds.model;

    // Verify new model (count: 1)
    let new_model_json: serde_json::Value = serde_json::from_slice(&new_model.content).expect("Failed to parse model JSON");
    assert_eq!(new_model_json["count"], 1);

    // 7. Send Reset Message
    let reset_msg = WitEnvelope {
        header: WitHeader {
            seqnum: 2,
            session_id: "test-session".to_string(),
            kind: Some("update".to_string()),
        },
        content_type: "application/json".to_string(),
        content: serde_json::to_vec(&serde_json::json!({"type": "Reset"})).expect("Failed to serialize msg"),
    };

    let model_cmds = actor_ref.ask(UpdateMsg { msg: reset_msg, model: new_model }).await.unwrap();
    let final_model = model_cmds.model;

    // Verify final model (count: 0)
    let final_model_json: serde_json::Value = serde_json::from_slice(&final_model.content).expect("Failed to parse model JSON");
    assert_eq!(final_model_json["count"], 0);

    Ok(())
}
