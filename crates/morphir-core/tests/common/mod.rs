//! The kit's document-tree cases, embedded verbatim, and the helpers the layout tests share.
//!
//! `spec/ir/mck/document-tree.md` (`document-tree-0003` to `0009`) is the byte oracle for both
//! halves of the layout: the writer turns each case's canonical document into the case's `file`
//! fences, and the reader turns those fences back into the document. A test in this submodule
//! cannot read the parent checkout's kit at run time, so every fence is a constant here — written
//! once and shared, so the two halves cannot drift apart.
//!
//! `document-tree-0005` carries `mode=read`: the kit runs only the read half of it, because a
//! writer never emits the `$meta` member the case is about. [`Case::writable`] says which half a
//! set is good for.
#![allow(dead_code)]

use morphir_core::ir::layout::{Profile, Tree, TreePolicy};
use morphir_core::ir::{IRFile, json, yaml};

// =============================================================================
// Kit case document-tree-0003: the node filename is the escaped stem
// =============================================================================

pub const DOCUMENT_0003: &str = r#"formatVersion: 4
distribution:
  Library:
    packageName: my-org/my-project
    dependencies: {}
    def:
      modules:
        domain:
          Public:
            types:
              user-ID:
                Public:
                  doc: The user's identifier
                  TypeAliasDefinition:
                    typeParams: []
                    typeExp: morphir/SDK:string#string
            values: {}
"#;

pub const TREE_0003: &[(&str, &str)] = &[
    (
        "manifest",
        r#"formatVersion: 4
distribution: Library
package: my-org/my-project
pathBudget: 4000
"#,
    ),
    (
        "pkg/my-org/my-project/domain/module",
        r#"formatVersion: 4
path: domain
types: [user-ID]
values: []
"#,
    ),
    (
        "pkg/my-org/my-project/domain/user-_id.type",
        r#"formatVersion: 4
name: user-ID
def:
  Public:
    doc: The user's identifier
    TypeAliasDefinition:
      typeParams: []
      typeExp: morphir/SDK:string#string
"#,
    ),
];

// =============================================================================
// Kit case document-tree-0004: a truncated stem is recorded in fileNames
// =============================================================================
//
// The names are the kit's own: the budget arithmetic depends on their lengths.

pub const DOCUMENT_0004: &str = r#"formatVersion: 4
distribution:
  Library:
    packageName: my-org/my-project
    dependencies: {}
    def:
      modules:
        domain:
          Public:
            types:
              customer-relationship-management-record:
                Public:
                  TypeAliasDefinition:
                    typeParams: []
                    typeExp: morphir/SDK:string#string
            values: {}
"#;

pub const TREE_0004: &[(&str, &str)] = &[
    (
        "manifest",
        r#"formatVersion: 4
distribution: Library
package: my-org/my-project
pathBudget: 64
"#,
    ),
    (
        "pkg/my-org/my-project/domain/module",
        r#"formatVersion: 4
path: domain
types: [customer-relationship-management-record]
values: []
fileNames:
  customer-relationship-management-record: customer-relati__44a101f8
"#,
    ),
    (
        "pkg/my-org/my-project/domain/customer-relati__44a101f8.type",
        r#"formatVersion: 4
name: customer-relationship-management-record
def:
  Public:
    TypeAliasDefinition:
      typeParams: []
      typeExp: morphir/SDK:string#string
"#,
    ),
];

// =============================================================================
// Kit case document-tree-0005: a top-level $meta member is reserved and ignored
// =============================================================================
//
// `mode=read`: the member is stripped on the way in and never written back, so only the read half
// of this set has an answer.

pub const DOCUMENT_0005: &str = r#"formatVersion: 4
distribution:
  Library:
    packageName: my-org/my-project
    dependencies: {}
    def:
      modules:
        domain:
          Public:
            types: {}
            values: {}
"#;

pub const TREE_0005: &[(&str, &str)] = &[
    (
        "manifest",
        r#"formatVersion: 4
distribution: Library
package: my-org/my-project
pathBudget: 4000
$meta:
  generator: example
"#,
    ),
    (
        "pkg/my-org/my-project/domain/module",
        r#"formatVersion: 4
path: domain
types: []
values: []
$meta:
  generator: example
"#,
    ),
];

// =============================================================================
// Kit case document-tree-0006: the same tree in the JSON profile
// =============================================================================

pub const DOCUMENT_0006: &str = r#"{ "formatVersion": 4, "distribution": { "Library": { "packageName": "my-org/my-project", "dependencies": {}, "def": { "modules": { "domain": { "Public": { "types": { "user-ID": { "Public": { "doc": "The user's identifier", "TypeAliasDefinition": { "typeParams": [], "typeExp": "morphir/SDK:string#string" } } } }, "values": {} } } } } } } }"#;

pub const TREE_0006: &[(&str, &str)] = &[
    (
        "manifest",
        r#"{ "formatVersion": 4, "distribution": "Library", "package": "my-org/my-project", "pathBudget": 4000 }"#,
    ),
    (
        "pkg/my-org/my-project/domain/module",
        r#"{ "formatVersion": 4, "path": "domain", "types": ["user-ID"], "values": [] }"#,
    ),
    (
        "pkg/my-org/my-project/domain/user-_id.type",
        r#"{ "formatVersion": 4, "name": "user-ID", "def": { "Public": { "doc": "The user's identifier", "TypeAliasDefinition": { "typeParams": [], "typeExp": "morphir/SDK:string#string" } } } }"#,
    ),
];

// =============================================================================
// Kit case document-tree-0007: a Private module
// =============================================================================

pub const DOCUMENT_0007: &str = r#"formatVersion: 4
distribution:
  Library:
    packageName: my-org/my-project
    dependencies: {}
    def:
      modules:
        domain:
          Private:
            types: {}
            values: {}
"#;

pub const TREE_0007: &[(&str, &str)] = &[
    (
        "manifest",
        r#"formatVersion: 4
distribution: Library
package: my-org/my-project
pathBudget: 4000
"#,
    ),
    (
        "pkg/my-org/my-project/domain/module",
        r#"formatVersion: 4
path: domain
access: Private
types: []
values: []
"#,
    ),
];

// =============================================================================
// Kit case document-tree-0008: a dependency lives under deps
// =============================================================================

pub const DOCUMENT_0008: &str = r#"formatVersion: 4
distribution:
  Library:
    packageName: my-org/my-project
    dependencies:
      morphir/SDK:
        modules:
          basics:
            types:
              int:
                OpaqueTypeSpecification: {}
            values: {}
    def:
      modules:
        domain:
          Public:
            types: {}
            values: {}
"#;

pub const TREE_0008: &[(&str, &str)] = &[
    (
        "manifest",
        r#"formatVersion: 4
distribution: Library
package: my-org/my-project
pathBudget: 4000
dependencies: [morphir/SDK]
"#,
    ),
    (
        "pkg/my-org/my-project/domain/module",
        r#"formatVersion: 4
path: domain
types: []
values: []
"#,
    ),
    (
        "deps/morphir/_sdk/@/basics/module",
        r#"formatVersion: 4
path: basics
types: [int]
values: []
"#,
    ),
    (
        "deps/morphir/_sdk/@/basics/int.type",
        r#"formatVersion: 4
name: int
spec:
  OpaqueTypeSpecification: {}
"#,
    ),
];

// =============================================================================
// Kit case document-tree-0009: an application's dependencies are definitions
// =============================================================================

pub const DOCUMENT_0009: &str = r#"formatVersion: 4
distribution:
  Application:
    packageName: example
    dependencies:
      my-org/shared:
        modules:
          util:
            Public:
              types: {}
              values:
                identity:
                  Public:
                    ExpressionBody:
                      inputTypes:
                        x: morphir/SDK:basics#int
                      outputType: morphir/SDK:basics#int
                      body:
                        Variable: x
    def:
      modules:
        main:
          Public:
            types: {}
            values:
              run:
                Public:
                  ExpressionBody:
                    inputTypes: {}
                    outputType: morphir/SDK:basics#unit
                    body:
                      Unit: {}
    entryPoints:
      start:
        target: example:main#run
        kind: main
"#;

pub const TREE_0009: &[(&str, &str)] = &[
    (
        "manifest",
        r#"formatVersion: 4
distribution: Application
package: example
pathBudget: 4000
dependencies: [my-org/shared]
entryPoints:
  start:
    target: example:main#run
    kind: main
"#,
    ),
    (
        "pkg/example/main/module",
        r#"formatVersion: 4
path: main
types: []
values: [run]
"#,
    ),
    (
        "pkg/example/main/run.value",
        r#"formatVersion: 4
name: run
def:
  Public:
    ExpressionBody:
      inputTypes: {}
      outputType: morphir/SDK:basics#unit
      body:
        Unit: {}
"#,
    ),
    (
        "deps/my-org/shared/@/util/module",
        r#"formatVersion: 4
path: util
types: []
values: [identity]
"#,
    ),
    (
        "deps/my-org/shared/@/util/identity.value",
        r#"formatVersion: 4
name: identity
def:
  Public:
    ExpressionBody:
      inputTypes:
        x: morphir/SDK:basics#int
      outputType: morphir/SDK:basics#int
      body:
        Variable: x
"#,
    ),
];

// =============================================================================
// The cases as data
// =============================================================================

/// One kit `file` set: the tree, the whole document it means, and how it is spelled.
pub struct Case {
    pub id: &'static str,
    pub profile: Profile,
    pub path_budget: u32,
    /// The case's canonical document, in the set's own profile.
    pub document: &'static str,
    /// The set's `file` fences, in the writer's emission order.
    pub files: &'static [(&'static str, &'static str)],
    /// Whether the set is written as well as read: a `mode=read` set is read only.
    pub writable: bool,
}

impl Case {
    /// The case's canonical document, read as an [`IRFile`].
    pub fn ir_file(&self) -> IRFile {
        read_document(self.profile, self.document)
    }

    /// The case's tree, as a map of logical path to text.
    pub fn tree(&self) -> Tree {
        tree(self.files)
    }

    pub fn policy(&self) -> TreePolicy {
        TreePolicy {
            profile: self.profile,
            path_budget: self.path_budget,
        }
    }
}

pub const CASES: &[Case] = &[
    Case {
        id: "document-tree-0003",
        profile: Profile::Yaml,
        path_budget: 4000,
        document: DOCUMENT_0003,
        files: TREE_0003,
        writable: true,
    },
    Case {
        id: "document-tree-0004",
        profile: Profile::Yaml,
        path_budget: 64,
        document: DOCUMENT_0004,
        files: TREE_0004,
        writable: true,
    },
    Case {
        id: "document-tree-0005",
        profile: Profile::Yaml,
        path_budget: 4000,
        document: DOCUMENT_0005,
        files: TREE_0005,
        writable: false,
    },
    Case {
        id: "document-tree-0006",
        profile: Profile::Json,
        path_budget: 4000,
        document: DOCUMENT_0006,
        files: TREE_0006,
        writable: true,
    },
    Case {
        id: "document-tree-0007",
        profile: Profile::Yaml,
        path_budget: 4000,
        document: DOCUMENT_0007,
        files: TREE_0007,
        writable: true,
    },
    Case {
        id: "document-tree-0008",
        profile: Profile::Yaml,
        path_budget: 4000,
        document: DOCUMENT_0008,
        files: TREE_0008,
        writable: true,
    },
    Case {
        id: "document-tree-0009",
        profile: Profile::Yaml,
        path_budget: 4000,
        document: DOCUMENT_0009,
        files: TREE_0009,
        writable: true,
    },
];

// =============================================================================
// Helpers
// =============================================================================

/// A tree from a list of logical paths and their text.
pub fn tree(files: &[(&str, &str)]) -> Tree {
    files
        .iter()
        .map(|(path, text)| ((*path).to_owned(), (*text).to_owned()))
        .collect()
}

/// The whole document `text` spells, read under `profile`.
pub fn read_document(profile: Profile, text: &str) -> IRFile {
    let read = match profile {
        Profile::Json => json::read_ir_file(text),
        Profile::Yaml => yaml::read_ir_file(text),
    };
    read.unwrap_or_else(|error| panic!("the kit's canonical document reads: {:?}", error.0))
        .0
}

pub fn yaml_document(text: &str) -> IRFile {
    read_document(Profile::Yaml, text)
}

pub fn json_document(text: &str) -> IRFile {
    read_document(Profile::Json, text)
}
