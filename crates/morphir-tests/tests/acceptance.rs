#![allow(clippy::get_first)]
use anyhow::Result;
use cucumber::{World, given, then, when};
use indexmap::IndexMap;
use morphir_common::config::MorphirConfig;
use morphir_common::loader::{self, LoadedDistribution};
use morphir_common::vfs::{MemoryVfs, OsVfs, Vfs};
use morphir_ir::converter;
use morphir_ir::ir::v4;
use std::path::{Path, PathBuf};

#[derive(Debug, World)]
pub struct TestWorld {
    input_path: PathBuf,
    loaded_content: Option<String>,
    intermediate_content: Option<String>,
    last_result: Option<Result<()>>,
    memory_vfs: Option<MemoryVfs>,
    glob_results: Vec<PathBuf>,
    visitor_count: usize,
    temp_dir: Option<tempfile::TempDir>,
    loaded_config: Option<MorphirConfig>,
}

impl Default for TestWorld {
    fn default() -> Self {
        Self {
            input_path: PathBuf::new(),
            loaded_content: None,
            intermediate_content: None,
            last_result: None,
            memory_vfs: None,
            glob_results: Vec::new(),
            visitor_count: 0,
            temp_dir: None,
            loaded_config: None,
        }
    }
}

// Configuration Steps

#[given(expr = "I have a {string} file with:")]
async fn i_have_a_config_file_with(
    w: &mut TestWorld,
    filename: String,
    step: &cucumber::gherkin::Step,
) {
    let dir = tempfile::tempdir().expect("Failed to create temp dir");
    let file_path = dir.path().join(&filename);
    let content = step.docstring.as_ref().expect("Docstring required").clone();
    std::fs::write(&file_path, content).expect("Failed to write config file");
    w.input_path = file_path;
    w.temp_dir = Some(dir);
}

#[when(expr = "I load the configuration")]
async fn i_load_configuration(w: &mut TestWorld) {
    match MorphirConfig::load(&w.input_path) {
        Ok(c) => {
            w.loaded_config = Some(c);
            w.last_result = Some(Ok(()));
        }
        Err(e) => {
            w.last_result = Some(Err(e));
        }
    }
}

#[then(expr = "it should be a workspace configuration")]
async fn it_should_be_workspace(w: &mut TestWorld) {
    let config = w.loaded_config.as_ref().expect("Config not loaded");
    assert!(config.is_workspace(), "Expected workspace configuration");
}

#[then(expr = "it should be a project configuration")]
async fn it_should_be_project(w: &mut TestWorld) {
    let config = w.loaded_config.as_ref().expect("Config not loaded");
    assert!(config.is_project(), "Expected project configuration");
}

#[then(expr = "the workspace should have {int} members")]
async fn workspace_should_have_members(w: &mut TestWorld, count: usize) {
    let config = w.loaded_config.as_ref().expect("Config not loaded");
    let actual = config.workspace.as_ref().unwrap().members.len();
    assert_eq!(actual, count);
}

#[then(expr = "the project name should be {string}")]
async fn project_name_should_be(w: &mut TestWorld, name: String) {
    let config = w.loaded_config.as_ref().expect("Config not loaded");
    let actual = &config.project.as_ref().unwrap().name;
    assert_eq!(actual, &name);
}

#[then(expr = "the source directory should be {string}")]
async fn source_directory_should_be(w: &mut TestWorld, dir: String) {
    let config = w.loaded_config.as_ref().expect("Config not loaded");
    let actual = &config.project.as_ref().unwrap().source_directory;
    assert_eq!(actual, &dir);
}

// Existing Steps

#[given(expr = "I have a {string} IR file named {string}")]
async fn i_have_an_ir_file(w: &mut TestWorld, _version: String, filename: String) {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    w.input_path = PathBuf::from(manifest_dir)
        .join("tests/features")
        .join(filename);
    if !w.input_path.exists() {
        panic!("Fixture file not found: {:?}", w.input_path);
    }
}

#[when(expr = "I load the distribution from the directory")]
async fn i_load_distribution_from_dir(w: &mut TestWorld) {
    let vfs = w.memory_vfs.as_ref().expect("MemoryVfs not initialized");
    let path = Path::new(".");
    match loader::load_distribution(vfs, path) {
        Ok(dist) => {
            let content = match dist {
                LoadedDistribution::V4(d) => serde_json::to_string(&d).unwrap(),
                LoadedDistribution::Classic(d) => serde_json::to_string(&d).unwrap(),
            };
            w.loaded_content = Some(content);
            w.last_result = Some(Ok(()));
        }
        Err(e) => {
            w.last_result = Some(Err(e));
        }
    }
}

#[when(expr = "I load the distribution from the file")]
async fn i_load_distribution_from_file(w: &mut TestWorld) {
    let vfs = OsVfs;
    let content = match vfs.read_to_string(&w.input_path) {
        Ok(c) => c,
        Err(e) => {
            w.last_result = Some(Err(e.into()));
            return;
        }
    };
    match loader::load_distribution(&vfs, &w.input_path) {
        Ok(_dist) => {
            w.loaded_content = Some(content);
            w.last_result = Some(Ok(()));
        }
        Err(e) => {
            println!("Loading Error for {:?}: {:?}", w.input_path, e);
            w.last_result = Some(Err(e));
        }
    }
}

#[when(expr = "I run \"morphir ir migrate\" to version {string}")]
async fn i_run_migrate(w: &mut TestWorld, target_version: String) {
    let vfs = OsVfs;
    match loader::load_distribution(&vfs, &w.input_path) {
        Ok(dist) => {
            let target_v4 = target_version == "v4";
            let result_content = match dist {
                LoadedDistribution::Classic(classic_dist) => {
                    if target_v4 {
                        let morphir_ir::ir::classic::DistributionBody::Library(_, pkg_path, _, pkg) =
                            classic_dist.distribution;
                        let v4_pkg = converter::classic_to_v4(pkg);
                        let v4_ir = v4::IRFile {
                            format_version: v4::FormatVersion::default(),
                            distribution: v4::Distribution::Library(v4::LibraryContent {
                                package_name: morphir_ir::naming::PackageName::from(pkg_path),
                                dependencies: IndexMap::new(),
                                def: v4_pkg,
                            }),
                        };
                        serde_json::to_string(&v4_ir)
                    } else {
                        serde_json::to_string(&classic_dist)
                    }
                }
                LoadedDistribution::V4(ir_file) => {
                    if !target_v4 {
                        let v4::Distribution::Library(lib_content) = ir_file.distribution else {
                            return;
                        };
                        let classic_pkg = converter::v4_to_classic(lib_content.def);
                        let classic_dist = morphir_ir::ir::classic::Distribution {
                            format_version: 2024,
                            distribution: morphir_ir::ir::classic::DistributionBody::Library(
                                morphir_ir::ir::classic::LibraryTag::Library,
                                lib_content.package_name.into_path(),
                                vec![],
                                classic_pkg,
                            ),
                        };
                        serde_json::to_string(&classic_dist)
                    } else {
                        serde_json::to_string(&ir_file)
                    }
                }
            };
            match result_content {
                Ok(content) => {
                    w.loaded_content = Some(content);
                    w.last_result = Some(Ok(()));
                }
                Err(e) => w.last_result = Some(Err(e.into())),
            };
        }
        Err(e) => {
            w.last_result = Some(Err(e));
        }
    }
}

#[then(expr = "I should get a valid {string} IR distribution")]
async fn i_should_get_valid_ir(w: &mut TestWorld, version: String) {
    if let Some(res) = &w.last_result {
        if res.is_err() {
            panic!("Last command failed: {:?}", res);
        }
    } else {
        panic!("Last command did not populate last_result");
    }
    let content = w.loaded_content.as_ref().expect("No loaded content found");
    if version == "v4" {
        let _ir_file: v4::IRFile =
            serde_json::from_str(content).expect("Failed to parse as V4 IR file");
    } else {
        let _dist: morphir_ir::ir::classic::Distribution =
            serde_json::from_str(content).expect("Failed to parse as Classic Distribution");
    }
}

#[then(expr = "the output file should be a valid {string} IR distribution")]
async fn output_should_be_valid(w: &mut TestWorld, version: String) {
    i_should_get_valid_ir(w, version).await;
}

// Migration steps for fixtures

#[given(expr = "I have a {string} IR file from fixtures {string}")]
async fn i_have_ir_from_fixtures(w: &mut TestWorld, _version: String, fixture_path: String) {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    w.input_path = PathBuf::from(manifest_dir)
        .join("fixtures")
        .join(fixture_path);
    if !w.input_path.exists() {
        panic!("Fixture file not found: {:?}", w.input_path);
    }
}

#[when(expr = "I save the result as intermediate")]
async fn save_result_as_intermediate(w: &mut TestWorld) {
    w.intermediate_content = w.loaded_content.clone();
}

#[when(expr = "I run \"morphir ir migrate\" on intermediate to version {string}")]
async fn run_migrate_on_intermediate(w: &mut TestWorld, target_version: String) {
    let content = w
        .intermediate_content
        .as_ref()
        .expect("No intermediate content");
    let target_v4 = target_version == "v4";

    // Try to parse as V4 first, then Classic
    let result_content = if let Ok(ir_file) = serde_json::from_str::<v4::IRFile>(content) {
        if !target_v4 {
            let v4::Distribution::Library(lib_content) = ir_file.distribution else {
                panic!("Expected Library distribution");
            };
            let classic_pkg = converter::v4_to_classic(lib_content.def);
            let classic_dist = morphir_ir::ir::classic::Distribution {
                format_version: 2024,
                distribution: morphir_ir::ir::classic::DistributionBody::Library(
                    morphir_ir::ir::classic::LibraryTag::Library,
                    lib_content.package_name.into_path(),
                    vec![],
                    classic_pkg,
                ),
            };
            serde_json::to_string(&classic_dist)
        } else {
            serde_json::to_string(&ir_file)
        }
    } else if let Ok(classic_dist) =
        serde_json::from_str::<morphir_ir::ir::classic::Distribution>(content)
    {
        if target_v4 {
            let morphir_ir::ir::classic::DistributionBody::Library(_, pkg_path, _, pkg) =
                classic_dist.distribution;
            let v4_pkg = converter::classic_to_v4(pkg);
            let v4_ir = v4::IRFile {
                format_version: v4::FormatVersion::default(),
                distribution: v4::Distribution::Library(v4::LibraryContent {
                    package_name: morphir_ir::naming::PackageName::from(pkg_path),
                    dependencies: IndexMap::new(),
                    def: v4_pkg,
                }),
            };
            serde_json::to_string(&v4_ir)
        } else {
            serde_json::to_string(&classic_dist)
        }
    } else {
        panic!("Could not parse intermediate content as V4 or Classic");
    };

    match result_content {
        Ok(content) => {
            w.loaded_content = Some(content);
            w.last_result = Some(Ok(()));
        }
        Err(e) => w.last_result = Some(Err(e.into())),
    }
}

// V4 Format Validation Steps

#[then(expr = "all module names should use kebab-case format")]
async fn all_module_names_kebab_case(w: &mut TestWorld) {
    let content = w.loaded_content.as_ref().expect("No loaded content");
    let v: serde_json::Value = serde_json::from_str(content).unwrap();

    if let Some(modules) = v
        .pointer("/distribution/Library/def/modules")
        .and_then(|m| m.as_object())
    {
        for (name, _) in modules {
            assert!(
                is_kebab_case(name),
                "Module name '{}' is not in kebab-case format",
                name
            );
        }
    }
}

#[then(expr = "all type names should use kebab-case format")]
async fn all_type_names_kebab_case(w: &mut TestWorld) {
    let content = w.loaded_content.as_ref().expect("No loaded content");
    let v: serde_json::Value = serde_json::from_str(content).unwrap();

    if let Some(modules) = v
        .pointer("/distribution/Library/def/modules")
        .and_then(|m| m.as_object())
    {
        for (_, module) in modules {
            if let Some(types) = module.pointer("/value/types").and_then(|t| t.as_object()) {
                for (name, _) in types {
                    assert!(
                        is_kebab_case(name),
                        "Type name '{}' is not in kebab-case format",
                        name
                    );
                }
            }
        }
    }
}

#[then(expr = "all value names should use kebab-case format")]
async fn all_value_names_kebab_case(w: &mut TestWorld) {
    let content = w.loaded_content.as_ref().expect("No loaded content");
    let v: serde_json::Value = serde_json::from_str(content).unwrap();

    if let Some(modules) = v
        .pointer("/distribution/Library/def/modules")
        .and_then(|m| m.as_object())
    {
        for (_, module) in modules {
            if let Some(values) = module.pointer("/value/values").and_then(|v| v.as_object()) {
                for (name, _) in values {
                    assert!(
                        is_kebab_case(name),
                        "Value name '{}' is not in kebab-case format",
                        name
                    );
                }
            }
        }
    }
}

#[then(expr = "all constructor names should use kebab-case format")]
async fn all_constructor_names_kebab_case(w: &mut TestWorld) {
    let content = w.loaded_content.as_ref().expect("No loaded content");
    let v: serde_json::Value = serde_json::from_str(content).unwrap();

    if let Some(modules) = v
        .pointer("/distribution/Library/def/modules")
        .and_then(|m| m.as_object())
    {
        for (_, module) in modules {
            if let Some(types) = module.pointer("/value/types").and_then(|t| t.as_object()) {
                for (_, type_def) in types {
                    if let Some(constructors) = type_def
                        .pointer("/CustomTypeDefinition/constructors/value")
                        .and_then(|c| c.as_array())
                    {
                        for ctor in constructors {
                            if let Some(name) = ctor.get("name").and_then(|n| n.as_str()) {
                                assert!(
                                    is_kebab_case(name),
                                    "Constructor name '{}' is not in kebab-case format",
                                    name
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

#[then(expr = "type references should use the V4 object wrapper format")]
async fn type_refs_use_object_wrapper(w: &mut TestWorld) {
    let content = w.loaded_content.as_ref().expect("No loaded content");
    let v: serde_json::Value = serde_json::from_str(content).unwrap();

    // Check that type expressions use object wrapper format (e.g., {"Reference": {...}})
    fn check_type_expr(value: &serde_json::Value) -> bool {
        if let Some(obj) = value.as_object() {
            // Valid V4 type expression wrappers
            let valid_tags = [
                "Reference",
                "Variable",
                "Tuple",
                "Record",
                "Function",
                "Unit",
                "ExtensibleRecord",
            ];
            obj.keys().any(|k| valid_tags.contains(&k.as_str()))
        } else {
            // Arrays are Classic format
            !value.is_array()
        }
    }

    if let Some(modules) = v
        .pointer("/distribution/Library/def/modules")
        .and_then(|m| m.as_object())
    {
        for (_, module) in modules {
            if let Some(types) = module.pointer("/value/types").and_then(|t| t.as_object()) {
                for (_, type_def) in types {
                    if let Some(type_exp) = type_def.pointer("/TypeAliasDefinition/typeExp") {
                        assert!(
                            check_type_expr(type_exp),
                            "Type expression is not in V4 object wrapper format: {:?}",
                            type_exp
                        );
                    }
                }
            }
        }
    }
}

#[then(expr = "FQNames should use canonical format")]
async fn fqnames_use_canonical_format(w: &mut TestWorld) {
    let content = w.loaded_content.as_ref().expect("No loaded content");

    // Canonical FQName format: "package/path:module#name"
    let fqname_pattern = regex::Regex::new(r#""fqname"\s*:\s*"([^"]+)""#).unwrap();

    for cap in fqname_pattern.captures_iter(content) {
        let fqname = &cap[1];
        assert!(
            fqname.contains(':') && fqname.contains('#'),
            "FQName '{}' is not in canonical format (expected 'package:module#name')",
            fqname
        );
    }
}

#[then(expr = "record type fields should use kebab-case names")]
async fn record_fields_kebab_case(w: &mut TestWorld) {
    let content = w.loaded_content.as_ref().expect("No loaded content");
    let v: serde_json::Value = serde_json::from_str(content).unwrap();

    fn check_record_fields(value: &serde_json::Value) {
        if let Some(obj) = value.as_object() {
            // Compact Record format: {"Record": {field1: type1, ...}}
            if let Some(record) = obj.get("Record")
                && let Some(fields) = record.as_object()
            {
                for (name, _) in fields {
                    assert!(
                        is_kebab_case(name),
                        "Record field name '{}' is not in kebab-case format",
                        name
                    );
                }
            }
            // Recursively check nested objects
            for (_, v) in obj {
                check_record_fields(v);
            }
        } else if let Some(arr) = value.as_array() {
            for v in arr {
                check_record_fields(v);
            }
        }
    }

    check_record_fields(&v);
}

#[then(expr = "value definitions should have non-null body content")]
async fn value_defs_have_body(w: &mut TestWorld) {
    let content = w.loaded_content.as_ref().expect("No loaded content");
    let v: serde_json::Value = serde_json::from_str(content).unwrap();

    if let Some(modules) = v
        .pointer("/distribution/Library/def/modules")
        .and_then(|m| m.as_object())
    {
        for (mod_name, module) in modules {
            if let Some(values) = module.pointer("/value/values").and_then(|v| v.as_object()) {
                for (val_name, val_def) in values {
                    let body = val_def.pointer("/body/ExpressionBody/body");
                    assert!(
                        body.is_some() && !body.unwrap().is_null(),
                        "Value '{}::{}' has null body",
                        mod_name,
                        val_name
                    );
                }
            }
        }
    }
}

#[then(expr = "value definitions should have properly converted inputTypes")]
async fn value_defs_have_input_types(w: &mut TestWorld) {
    let content = w.loaded_content.as_ref().expect("No loaded content");
    let v: serde_json::Value = serde_json::from_str(content).unwrap();

    if let Some(modules) = v
        .pointer("/distribution/Library/def/modules")
        .and_then(|m| m.as_object())
    {
        for (_, module) in modules {
            if let Some(values) = module.pointer("/value/values").and_then(|v| v.as_object()) {
                for (val_name, val_def) in values {
                    if let Some(input_types) = val_def.get("inputTypes").and_then(|i| i.as_object())
                    {
                        for (param_name, param_def) in input_types {
                            // Check that input type uses V4 format
                            assert!(
                                param_def.get("type").is_some()
                                    || param_def.get("input_type").is_some(),
                                "Value '{}' parameter '{}' missing type field",
                                val_name,
                                param_name
                            );
                        }
                    }
                }
            }
        }
    }
}

#[then(expr = "value definitions should have properly converted outputType")]
async fn value_defs_have_output_type(w: &mut TestWorld) {
    let content = w.loaded_content.as_ref().expect("No loaded content");
    let v: serde_json::Value = serde_json::from_str(content).unwrap();

    if let Some(modules) = v
        .pointer("/distribution/Library/def/modules")
        .and_then(|m| m.as_object())
    {
        for (mod_name, module) in modules {
            if let Some(values) = module.pointer("/value/values").and_then(|v| v.as_object()) {
                for (val_name, val_def) in values {
                    let output_type = val_def.get("outputType");
                    assert!(
                        output_type.is_some() && !output_type.unwrap().is_null(),
                        "Value '{}::{}' has null outputType",
                        mod_name,
                        val_name
                    );
                }
            }
        }
    }
}

// Helper function to check if a string is in kebab-case format
fn is_kebab_case(s: &str) -> bool {
    // Kebab-case: lowercase letters and hyphens, no underscores or uppercase
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

#[then(expr = "the package name should be {string}")]
async fn package_name_should_be(w: &mut TestWorld, name: String) {
    let content = w.loaded_content.as_ref().expect("No loaded content found");
    let v: serde_json::Value = serde_json::from_str(content).unwrap();
    let pkg_name = if let Some(dist) = v.get("distribution") {
        if dist.is_array() {
            if let Some(tag) = dist.get(0).and_then(|v| v.as_str()) {
                if tag == "Library" || tag == "library" {
                    let pkg_val = dist.get(1);
                    if let Some(s) = pkg_val.and_then(|v| v.as_str()) {
                        Some(s.to_string())
                    } else if let Some(arr) = pkg_val.and_then(|v| v.as_array()) {
                        let parts: Vec<String> = arr
                            .iter()
                            .filter_map(|segment| {
                                if let Some(s) = segment.as_str() {
                                    Some(s.to_string())
                                } else if let Some(inner_arr) = segment.as_array() {
                                    // Join all words in the Name segment with "-"
                                    let words: Vec<String> = inner_arr
                                        .iter()
                                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                                        .collect();
                                    if words.is_empty() {
                                        None
                                    } else {
                                        Some(words.join("-"))
                                    }
                                } else {
                                    None
                                }
                            })
                            .collect();
                        if parts.is_empty() {
                            None
                        } else {
                            Some(parts.join("-"))
                        }
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            }
        } else if dist.is_object() {
            if let Some(lib) = dist.get("Library") {
                // New V4 format: { "Library": { "packageName": "name", ... } }
                lib.get("packageName")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };
    assert_eq!(
        pkg_name,
        Some(name),
        "Package name mismatch. Found {:?}",
        pkg_name
    );
}

// VFS Steps

#[given(expr = "I have a Memory VFS")]
async fn i_have_a_memory_vfs(w: &mut TestWorld) {
    w.memory_vfs = Some(MemoryVfs::new());
}

#[given(expr = "I create a file {string}")]
async fn i_create_a_file(w: &mut TestWorld, name: String) {
    let vfs = w.memory_vfs.as_ref().expect("MemoryVfs not initialized");
    vfs.write_from_string(Path::new(&name), "content")
        .expect("Failed to write to MemoryVfs");
}

#[given(expr = "I have a project structure with the following files:")]
async fn i_have_project_structure(w: &mut TestWorld, step: &cucumber::gherkin::Step) {
    let vfs = w.memory_vfs.as_ref().expect("MemoryVfs not initialized");
    if let Some(table) = &step.table {
        for row in &table.rows {
            let filename = &row[0];
            let content = if row.len() > 1 { &row[1] } else { "content" };
            vfs.write_from_string(Path::new(filename), content)
                .expect("Failed to write to MemoryVfs");
        }
    }
}

#[when(expr = "I glob for {string}")]
async fn i_glob_for(w: &mut TestWorld, pattern: String) {
    let vfs = w.memory_vfs.as_ref().expect("MemoryVfs not initialized");
    w.glob_results = vfs.glob(&pattern).expect("Glob failed");
}

#[then(expr = "I should find {string}")]
async fn i_should_find(w: &mut TestWorld, name: String) {
    let expected = PathBuf::from(name);
    assert!(
        w.glob_results.contains(&expected),
        "Expected to find {:?}, but got {:?}",
        expected,
        w.glob_results
    );
}

#[then(expr = "I should not find {string}")]
async fn i_should_not_find(w: &mut TestWorld, name: String) {
    let expected = PathBuf::from(name);
    assert!(
        !w.glob_results.contains(&expected),
        "Expected NOT to find {:?}, but got it",
        expected
    );
}

// Visitor Steps

struct ModuleCountingVisitor {
    count: usize,
}

impl morphir_ir::visitor::Visitor for ModuleCountingVisitor {
    fn visit_module(
        &mut self,
        cursor: &mut morphir_ir::visitor::Cursor,
        _module: &morphir_ir::ir::classic::Module,
    ) {
        self.count += 1;
        morphir_ir::visitor::walk_module(self, cursor, _module);
    }
}

struct VariableCountingVisitor {
    count: usize,
}

impl morphir_ir::visitor::Visitor for VariableCountingVisitor {
    fn visit_expression(
        &mut self,
        cursor: &mut morphir_ir::visitor::Cursor,
        expr: &morphir_ir::ir::classic::Expression,
    ) {
        if let morphir_ir::ir::classic::Expression::Variable { .. } = expr {
            self.count += 1;
        }
        morphir_ir::visitor::walk_expression(self, cursor, expr);
    }
}

#[when(expr = "I visit the distribution using a Module Counting Visitor")]
async fn i_visit_distribution(w: &mut TestWorld) {
    let vfs = OsVfs;
    let load_res =
        loader::load_distribution(&vfs, &w.input_path).expect("Failed to load distribution");
    match load_res {
        LoadedDistribution::Classic(dist) => {
            let mut visitor = ModuleCountingVisitor { count: 0 };
            use morphir_ir::visitor::Visitor;
            visitor.traverse(&dist);
            w.visitor_count = visitor.count;
        }
        LoadedDistribution::V4(_) => panic!("Expected Classic distribution for this test"),
    }
}

#[given(expr = "I have a simple expression with 3 variables")]
async fn i_have_simple_expression(w: &mut TestWorld) {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    w.input_path = PathBuf::from(manifest_dir).join("tests/features/simple_classic.json");
}

#[when(expr = "I visit the expression using a Variable Counting Visitor")]
async fn i_visit_expression(w: &mut TestWorld) {
    let vfs = OsVfs;
    let load_res =
        loader::load_distribution(&vfs, &w.input_path).expect("Failed to load distribution");
    match load_res {
        LoadedDistribution::Classic(dist) => {
            let mut visitor = VariableCountingVisitor { count: 0 };
            use morphir_ir::visitor::Visitor;
            visitor.traverse(&dist);
            w.visitor_count = visitor.count;
        }
        _ => panic!("Expected classic"),
    }
}

#[then(expr = "the module count should be {int}")]
async fn module_count_should_be(w: &mut TestWorld, count: usize) {
    assert_eq!(w.visitor_count, count);
}

#[then(expr = "the variable count should be {int}")]
async fn variable_count_should_be(w: &mut TestWorld, count: usize) {
    assert_eq!(w.visitor_count, count);
}

#[tokio::main]
async fn main() {
    TestWorld::run("tests/features").await;
}
