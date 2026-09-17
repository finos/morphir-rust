use morphir_extension_sdk::prelude::*;
use morphir_python_binding::PythonExtension;
use serde_json::{Value, json};

fn request(sources: &[(&str, &str)]) -> CompileRequest {
    CompileRequest {
        language_id: "python".into(),
        documents: sources
            .iter()
            .map(|(uri, text)| SourceDocument {
                uri: (*uri).into(),
                language_id: "python".into(),
                version: 1,
                text: (*text).into(),
            })
            .collect(),
        package: CompilePackage {
            name: "acme/example".into(),
            exposed_modules: None,
        },
        dependencies: vec![],
        options: CompileOptions {
            ir_version: "4".into(),
            types_only: false,
            ..Default::default()
        },
    }
}

fn compile(request: CompileRequest) -> Value {
    let result = PythonExtension.compile(request).unwrap();
    assert!(result.success, "{:?}", result.diagnostics);
    result.ir.unwrap()
}

fn roundtrip(request: CompileRequest) -> (Value, Vec<Artifact>) {
    let ir = compile(request.clone());
    let mut reversed = request;
    reversed.documents.reverse();
    assert_eq!(
        ir,
        compile(reversed),
        "document order must not affect resolution"
    );
    let generated = PythonExtension
        .generate(GenerateRequest {
            ir: ir.clone(),
            target: "python".into(),
            options: Default::default(),
        })
        .unwrap();
    assert!(generated.success, "{:?}", generated.diagnostics);
    let sources: Vec<_> = generated
        .artifacts
        .iter()
        .map(|a| (a.path.as_str(), a.content.as_str()))
        .collect();
    assert_eq!(ir, compile(self::request(&sources)));
    (ir, generated.artifacts)
}

const TYPES: &str = "from dataclasses import dataclass\n@dataclass(frozen=True)\nclass Address:\n    zip_code: int\ntype Pair = tuple[int, str]\n";

#[test]
fn private_modules_are_available_inside_the_package_and_survive_generation() {
    let mut input = request(&[
        ("internal/models.py", TYPES),
        (
            "api.py",
            "from internal.models import Pair\ndef origin() -> Pair:\n    return (1, 'one')\n",
        ),
    ]);
    input.package.exposed_modules = Some(vec!["api".into()]);
    let ir = compile(input.clone());
    assert!(
        ir["distribution"]["Library"]["def"]["modules"]["internal/models"]["Private"].is_object()
    );
    assert!(ir["distribution"]["Library"]["def"]["modules"]["api"]["Public"].is_object());
    let generated = PythonExtension
        .generate(GenerateRequest {
            ir: ir.clone(),
            target: "python".into(),
            options: Default::default(),
        })
        .unwrap();
    assert!(generated.success, "{:?}", generated.diagnostics);
    input.documents = generated
        .artifacts
        .iter()
        .map(|artifact| SourceDocument {
            uri: artifact.path.clone(),
            text: artifact.content.clone(),
            language_id: "python".into(),
            version: 1,
        })
        .collect();
    assert_eq!(ir, compile(input));
}

#[test]
fn explicitly_empty_exposure_makes_every_module_private() {
    let mut input = request(&[("models.py", TYPES)]);
    input.package.exposed_modules = Some(vec![]);
    let ir = compile(input);
    assert!(ir["distribution"]["Library"]["def"]["modules"]["models"]["Private"].is_object());
}

#[test]
fn imported_types_and_tuple_aliases_roundtrip_across_modules() {
    let (ir, artifacts) = roundtrip(request(&[
        ("models.py", TYPES),
        (
            "rules.py",
            "from models import Address as Home, Pair\ndef choose(flag: bool, first: Home, second: Home) -> Home:\n    return first if flag else second\ndef make_pair() -> Pair:\n    return (1, 'one')\n",
        ),
    ]));
    assert_eq!(
        artifacts
            .iter()
            .map(|a| a.path.as_str())
            .collect::<Vec<_>>(),
        ["models.py", "rules.py"]
    );
    assert_eq!(
        ir["distribution"]["Library"]["def"]["modules"]["rules"]["Public"]["values"]["choose"]["Public"]
            ["ExpressionBody"]["outputType"],
        json!("acme/example:models#address")
    );
}

#[test]
fn nested_modules_resolve_absolute_relative_and_qualified_imports() {
    let mut input = request(&[
        ("file:///project/src/domain/models.py", TYPES),
        (
            "file:///project/src/domain/rules.py",
            "from .models import Pair\nimport domain.models as model\nfrom dataclasses import dataclass\n@dataclass(frozen=True)\nclass Holder:\n    home: model.Address\ndef make_pair() -> Pair:\n    return (1, 'one')\n",
        ),
        (
            "file:///project/src/app.py",
            "import domain.models\nfrom domain import rules as rule\nfrom dataclasses import dataclass\n@dataclass(frozen=True)\nclass App:\n    home: domain.models.Address\n    holder: rule.Holder\n",
        ),
    ]);
    input
        .options
        .extra
        .insert("sourceRootUri".into(), json!("file:///project/src"));
    input.package.exposed_modules = Some(vec![
        "App".into(),
        "Domain.Models".into(),
        "Domain.Rules".into(),
    ]);
    let (_, artifacts) = roundtrip(input);
    assert_eq!(
        artifacts
            .iter()
            .map(|a| a.path.as_str())
            .collect::<Vec<_>>(),
        ["app.py", "domain/models.py", "domain/rules.py"]
    );
}

#[test]
fn mutually_referencing_records_and_same_named_types_roundtrip() {
    roundtrip(request(&[
        (
            "first.py",
            "from dataclasses import dataclass\nimport second\n@dataclass(frozen=True)\nclass Item:\n    other: second.Item\n",
        ),
        (
            "second.py",
            "from dataclasses import dataclass\nimport first\n@dataclass(frozen=True)\nclass Item:\n    other: first.Item\n",
        ),
    ]));
}

#[test]
fn rejects_invalid_module_graphs_without_partial_ir() {
    for sources in [
        vec![],
        vec![("models.py", TYPES), ("models.py", TYPES)],
        vec![("foo_bar.py", ""), ("fooBar.py", "")],
        vec![
            ("models.py", TYPES),
            ("rules.py", "from missing import Address\n"),
        ],
        vec![
            ("models.py", TYPES),
            ("rules.py", "from models import Missing\n"),
        ],
        vec![("models.py", TYPES), ("rules.py", "from models import *\n")],
        vec![
            ("models.py", TYPES),
            ("rules.py", "from models import Address as int\n"),
        ],
        vec![
            ("models.py", TYPES),
            (
                "rules.py",
                "from models import Address\nfrom dataclasses import dataclass\n@dataclass(frozen=True)\nclass Address:\n    x: int\n",
            ),
        ],
        vec![
            ("models.py", TYPES),
            ("rules.py", "from .models import Address\n"),
        ],
        vec![
            (
                "a.py",
                "from b import Pair as Other\ntype Pair = tuple[Other, int]\n",
            ),
            (
                "b.py",
                "from a import Pair as Other\ntype Pair = tuple[Other, int]\n",
            ),
        ],
        vec![("../models.py", TYPES)],
        vec![("models/__init__.py", TYPES)],
        vec![("models.py", TYPES), ("models/child.py", TYPES)],
    ] {
        let result = PythonExtension.compile(request(&sources)).unwrap();
        assert!(!result.success, "accepted {sources:?}");
        assert!(result.ir.is_none());
        assert!(result.modules.is_empty());
    }
}

#[test]
fn rejects_sources_outside_root_and_unknown_exposed_modules() {
    let mut input = request(&[("file:///other/models.py", TYPES)]);
    input
        .options
        .extra
        .insert("sourceRootUri".into(), json!("file:///project"));
    assert!(!PythonExtension.compile(input).unwrap().success);
    let mut input = request(&[("models.py", TYPES), ("rules.py", "")]);
    input.package.exposed_modules = Some(vec!["Missing".into()]);
    assert!(!PythonExtension.compile(input).unwrap().success);
}

#[test]
fn imports_sharing_a_package_prefix_and_parent_relative_imports_roundtrip() {
    roundtrip(request(&[
        ("domain/models.py", TYPES),
        ("domain/other.py", TYPES),
        (
            "domain/rules/select.py",
            "from ..models import Address\nfrom .. import other\nimport domain.models\nimport domain.other\nfrom dataclasses import dataclass\n@dataclass(frozen=True)\nclass Choice:\n    first: domain.models.Address\n    second: domain.other.Address\n    third: other.Address\n    fourth: Address\n",
        ),
    ]));
}

#[test]
fn generated_module_imports_avoid_declarations_and_constructor_names() {
    let (_, artifacts) = roundtrip(request(&[
        ("models.py", TYPES),
        (
            "rules.py",
            "from models import Address\nfrom dataclasses import dataclass\n@dataclass(frozen=True)\nclass MorphirModule1:\n    home: Address\n@dataclass(frozen=True)\nclass MorphirModule2:\n    pass\ntype Choice = MorphirModule2\ndef morphir_module_1(value: Address) -> Address:\n    return value\ndef morphir_module_2(value: Address) -> Address:\n    return value\n",
        ),
    ]));
    assert!(
        artifacts[1]
            .content
            .contains("import models as morphir_module_3")
    );
}

#[test]
fn reports_every_module_and_locates_bad_imports() {
    let result = PythonExtension
        .compile(request(&[("b.py", ""), ("a.py", "")]))
        .unwrap();
    assert_eq!(result.modules, ["a", "b"]);
    let result = PythonExtension
        .compile(request(&[
            ("models.py", TYPES),
            ("bad.py", "from models import Missing\n"),
        ]))
        .unwrap();
    assert!(!result.success);
    assert_eq!(
        result.diagnostics[0].location.as_ref().unwrap().uri,
        "bad.py"
    );
}

#[test]
fn backend_rejects_unresolved_cross_module_references_atomically() {
    let mut ir = compile(request(&[
        ("models.py", TYPES),
        (
            "rules.py",
            "from models import Address\nfrom dataclasses import dataclass\n@dataclass(frozen=True)\nclass Holder:\n    home: Address\n",
        ),
    ]));
    ir["distribution"]["Library"]["def"]["modules"]["rules"]["Public"]["types"]["holder"]["Public"]
        ["TypeAliasDefinition"]["typeExp"]["Record"]["fields"]["home"] =
        json!("acme/example:models#missing");
    let result = PythonExtension
        .generate(GenerateRequest {
            ir,
            target: "python".into(),
            options: Default::default(),
        })
        .unwrap();
    assert!(!result.success);
    assert!(result.artifacts.is_empty());
}

#[test]
fn types_only_applies_to_every_module() {
    let mut input = request(&[
        ("models.py", TYPES),
        ("rules.py", "def count() -> int:\n    return 1\n"),
    ]);
    input.options.types_only = true;
    assert!(!PythonExtension.compile(input).unwrap().success);
}

#[test]
fn rejects_module_paths_that_collide_on_case_insensitive_filesystems() {
    for sources in [
        vec![("API.py", ""), ("api.py", "")],
        vec![("API.py", ""), ("api/models.py", "")],
        vec![("domain/API.py", ""), ("domain/api.py", "")],
    ] {
        assert!(
            !PythonExtension.compile(request(&sources)).unwrap().success,
            "accepted {sources:?}"
        );
    }
    let ir = json!({"formatVersion": 4, "distribution": {"Library": {
        "packageName": "acme/example", "dependencies": {}, "def": {"modules": {
            "API": {"Public": {"types": {}, "values": {}}},
            "api": {"Public": {"types": {}, "values": {}}}
        }}
    }}});
    let result = PythonExtension
        .generate(GenerateRequest {
            ir,
            target: "python".into(),
            options: Default::default(),
        })
        .unwrap();
    assert!(!result.success);
    assert!(result.artifacts.is_empty());
}

#[test]
fn document_uri_metadata_and_percent_encoding_do_not_change_module_identity() {
    let mut input = request(&[
        (
            "file:///my%20project/src/domain/%6dodels.py?rev=1#selection",
            TYPES,
        ),
        (
            "file:///my%20project/src/domain/rules.py?rev=2",
            "from .models import Pair\ndef origin() -> Pair:\n    return (0, '')\n",
        ),
    ]);
    input
        .options
        .extra
        .insert("sourceRootUri".into(), json!("file:///my%20project/src/"));
    roundtrip(input);
    for uri in [
        "file://other/project/src/models.py",
        "file:///project/src/../models.py",
        "file:///project/src/%2e%2e/models.py",
        "file:///project/src/domain%2Fmodels.py",
        "file:///project/src/domain%5Cmodels.py",
    ] {
        let mut input = request(&[(uri, TYPES)]);
        input
            .options
            .extra
            .insert("sourceRootUri".into(), json!("file:///project/src"));
        assert!(
            !PythonExtension.compile(input).unwrap().success,
            "accepted {uri}"
        );
    }
}
