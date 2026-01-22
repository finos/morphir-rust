use cucumber::{given, when, then, World};
use std::path::{Path, PathBuf};
use tokio;
use morphir_common::vfs::{Vfs, OsVfs, MemoryVfs};
use morphir_common::loader::{self, LoadedDistribution};
use morphir_ir::converter;
use anyhow::Result;

#[derive(Debug, World)]
pub struct TestWorld {
    input_path: PathBuf,
    // Eliminated output_path dependency for validation
    loaded_content: Option<String>, 
    last_result: Option<Result<()>>,
    memory_vfs: Option<MemoryVfs>,
    glob_results: Vec<PathBuf>,
    visitor_count: usize,
}

impl Default for TestWorld {
    fn default() -> Self {
        Self {
            input_path: PathBuf::new(),
            loaded_content: None,
            last_result: None,
            memory_vfs: None,
            glob_results: Vec::new(),
            visitor_count: 0,
        }
    }
}

#[given(expr = "I have a {string} IR file named {string}")]
async fn i_have_an_ir_file(w: &mut TestWorld, _version: String, filename: String) {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    w.input_path = PathBuf::from(manifest_dir).join("tests/features").join(filename);
    
    if !w.input_path.exists() {
        panic!("Fixture file not found: {:?}", w.input_path);
    }
}

#[when(expr = "I load the distribution from the directory")]
async fn i_load_distribution_from_dir(w: &mut TestWorld) {
    let vfs = w.memory_vfs.as_ref().expect("MemoryVfs not initialized");
    let path = Path::new(".");
    let load_res = loader::load_distribution(vfs, path);
    
    match load_res {
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
    
    // Read the original file content first
    let content = match vfs.read_to_string(&w.input_path) {
        Ok(c) => c,
        Err(e) => {
            w.last_result = Some(Err(e.into()));
            return;
        }
    };

    let load_res = loader::load_distribution(&vfs, &w.input_path);
    
    match load_res {
         Ok(_dist) => {
             // Store the ORIGINAL file content, not re-serialized
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
    let load_res = loader::load_distribution(&vfs, &w.input_path);
    
    match load_res {
        Ok(dist) => {
            let target_v4 = target_version == "v4";
            let result_content = match dist {
                LoadedDistribution::Classic(classic_dist) => {
                    if target_v4 {
                        let morphir_ir::ir::classic::DistributionBody::Library(_, pkg_name, _, pkg) = classic_dist.distribution;
                        let v4_pkg = converter::classic_to_v4(pkg);
                        
                        let v4_dist = morphir_ir::ir::v4::Distribution {
                            format_version: 4,
                            distribution: morphir_ir::ir::v4::DistributionBody::Library(
                                morphir_ir::ir::v4::LibraryDistribution(
                                    morphir_ir::ir::v4::LibraryTag::Library,
                                    pkg_name,
                                    vec![],
                                    v4_pkg
                                )
                            )
                        };
                        serde_json::to_string(&v4_dist)
                    } else {
                         serde_json::to_string(&classic_dist)
                    }
                }
                LoadedDistribution::V4(v4_dist) => {
                     if !target_v4 {
                        let morphir_ir::ir::v4::DistributionBody::Library(lib) = v4_dist.distribution;
                        let pkg_name = lib.1;
                        let pkg_def = lib.3;
                        
                        let classic_pkg = converter::v4_to_classic(pkg_def);
                        
                        let classic_dist = morphir_ir::ir::classic::Distribution {
                            format_version: 2024,
                            distribution: morphir_ir::ir::classic::DistributionBody::Library(
                                morphir_ir::ir::classic::LibraryTag::Library,
                                pkg_name,
                                vec![],
                                classic_pkg
                            )
                        };
                        serde_json::to_string(&classic_dist)
                    } else {
                         serde_json::to_string(&v4_dist)
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
        let _dist: morphir_ir::ir::v4::Distribution = serde_json::from_str(content).expect("Failed to parse as V4 Distribution");
    } else {
         let _dist: morphir_ir::ir::classic::Distribution = serde_json::from_str(content).expect("Failed to parse as Classic Distribution");
    }
}

#[then(expr = "the output file should be a valid {string} IR distribution")]
async fn output_should_be_valid(w: &mut TestWorld, version: String) {
    i_should_get_valid_ir(w, version).await;
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
                          // V4: Simple string
                          Some(s.to_string())
                      } else if let Some(arr) = pkg_val.and_then(|v| v.as_array()) {
                          // Legacy: Array of arrays [["morphir"], ["example"], ["app"]]
                          // or simple names [["rentals"]]
                          let parts: Vec<String> = arr.iter().filter_map(|segment| {
                              if let Some(s) = segment.as_str() {
                                  // Invalid per spec but just in case
                                  Some(s.to_string())
                              } else if let Some(inner_arr) = segment.as_array() {
                                  inner_arr.get(0).and_then(|v| v.as_str()).map(|s| s.to_string())
                              } else {
                                  None
                              }
                          }).collect();
                          if parts.is_empty() { None } else { Some(parts.join("-")) }
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
                 lib.get(1).and_then(|v| v.as_str()).map(|s| s.to_string())
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };
    
    assert_eq!(pkg_name, Some(name), "Package name mismatch. Found {:?}", pkg_name);
}

// VFS Steps

#[given(expr = "I have a Memory VFS")]
async fn i_have_a_memory_vfs(w: &mut TestWorld) {
    w.memory_vfs = Some(MemoryVfs::new());
}

#[given(expr = "I create a file {string}")]
async fn i_create_a_file(w: &mut TestWorld, name: String) {
    let vfs = w.memory_vfs.as_ref().expect("MemoryVfs not initialized");
    vfs.write_from_string(Path::new(&name), "content").expect("Failed to write to MemoryVfs");
}

#[given(expr = "I have a project structure with the following files:")]
async fn i_have_project_structure(w: &mut TestWorld, step: &cucumber::gherkin::Step) {
    let vfs = w.memory_vfs.as_ref().expect("MemoryVfs not initialized");
    if let Some(table) = &step.table {
        for row in &table.rows {
             let filename = &row[0];
             vfs.write_from_string(Path::new(filename), "content").expect("Failed to write to MemoryVfs");
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
    assert!(w.glob_results.contains(&expected), "Expected to find {:?}, but got {:?}", expected, w.glob_results);
}

#[then(expr = "I should not find {string}")]
async fn i_should_not_find(w: &mut TestWorld, name: String) {
    let expected = PathBuf::from(name);
    assert!(!w.glob_results.contains(&expected), "Expected NOT to find {:?}, but got it", expected);
}

// Visitor Steps

struct ModuleCountingVisitor {
    count: usize,
}

impl morphir_ir::visitor::Visitor for ModuleCountingVisitor {
    fn visit_module(&mut self, _module: &morphir_ir::ir::classic::Module) {
        self.count += 1;
        morphir_ir::visitor::walk_module(self, _module);
    }
}

struct VariableCountingVisitor {
    count: usize,
}

impl morphir_ir::visitor::Visitor for VariableCountingVisitor {
    fn visit_expression(&mut self, expr: &morphir_ir::ir::classic::Expression) {
        if let morphir_ir::ir::classic::Expression::Variable { .. } = expr {
            self.count += 1;
        }
        morphir_ir::visitor::walk_expression(self, expr);
    }
}

#[when(expr = "I visit the distribution using a Module Counting Visitor")]
async fn i_visit_distribution(w: &mut TestWorld) {
    let vfs = OsVfs;
    let load_res = loader::load_distribution(&vfs, &w.input_path).expect("Failed to load distribution");
    
    match load_res {
        LoadedDistribution::Classic(dist) => {
            let mut visitor = ModuleCountingVisitor { count: 0 };
            use morphir_ir::visitor::Visitor;
            visitor.visit_distribution(&dist);
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
    let load_res = loader::load_distribution(&vfs, &w.input_path).expect("Failed to load distribution");
    
    match load_res {
        LoadedDistribution::Classic(dist) => {
            let mut visitor = VariableCountingVisitor { count: 0 };
            use morphir_ir::visitor::Visitor;
            visitor.visit_distribution(&dist);
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
