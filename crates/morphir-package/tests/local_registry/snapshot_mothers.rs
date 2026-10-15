//! Signed provider fixture with independent TUF signing and literal native facts.
use super::tuf_mothers as tuf;
use morphir_package::{
    authoring::{AuthoredLibrary, LocalSigningKey},
    digest::Digest,
    local_registry::mvp::{self, InitializeRequest, ResolveRequest},
    resolution::{PackagePath, ReleaseId, StableVersion},
};
use serde_json::json;
use std::{fs, path::Path};

pub const CONTEXT: &[u8] = br#"{
  "@context": {"@vocab": "morphir://ir/pkg/morphir/metadata?format=4.1.0#/module/schema/value/"}
}
"#;
pub const PREDICATE: &str =
    "morphir://ir/pkg/acme/metadata?format=4.1.0#/module/lifecycle/value/deprecated";

pub struct SignedProvider {
    pub directory: tempfile::TempDir,
    pub policy: Vec<u8>,
    pub lock: Vec<u8>,
    pub native: Vec<u8>,
    pub manifest: Vec<u8>,
    pub bundle: String,
}

fn write(root: &Path, path: &str, bytes: &[u8]) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

fn target(root: &Path, path: &str, bytes: &[u8]) -> serde_json::Value {
    let (parent, name) = path.rsplit_once('/').unwrap();
    let digest = Digest::of_bytes(bytes).to_string();
    let hash = digest.strip_prefix("sha256:").unwrap();
    write(root, &format!("targets/{parent}/{hash}.{name}"), bytes);
    json!({"length": bytes.len(), "hashes": {"sha256": hash}})
}

pub async fn a_signed_provider() -> SignedProvider {
    a_signed_provider_with(|_| {}).await
}

pub async fn a_signed_provider_with(mutate: impl FnOnce(&mut serde_json::Value)) -> SignedProvider {
    a_signed_provider_at("example.com/metadata/provider", mutate).await
}

/// Signed test cases separate authentication from consumer admission.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NamingVariant {
    /// Accepted naming declaration and one context.
    #[default]
    Compatible,
    /// A well-formed Boolean value instead of the naming record.
    WrongType,
    /// A schema role unsupported by the naming interpreter.
    WrongSubjectRole,
    /// An IRI object declaration instead of literal data.
    WrongObjectForm,
    /// A descriptive interpreter instead of target naming.
    WrongInterpreter,
    /// Equal context bytes retained at two distinct inventory paths.
    AmbiguousContexts,
}
impl NamingVariant {
    /// Parse only documented test fixture variants.
    pub fn parse(value: &str) -> Result<Self, &'static str> {
        match value {
            "compatible" => Ok(Self::Compatible),
            "wrong-type" => Ok(Self::WrongType),
            "wrong-subject-role" => Ok(Self::WrongSubjectRole),
            "wrong-object-form" => Ok(Self::WrongObjectForm),
            "wrong-interpreter" => Ok(Self::WrongInterpreter),
            "ambiguous-contexts" => Ok(Self::AmbiguousContexts),
            _ => Err("unknown signed fixture variant"),
        }
    }
}

pub async fn a_signed_naming_provider() -> SignedProvider {
    a_signed_naming_variant(NamingVariant::Compatible).await
}

/// Sign a selected naming declaration and its complete context inventory.
pub async fn a_signed_naming_variant(variant: NamingVariant) -> SignedProvider {
    let mut contexts = vec![("contexts/schema.jsonld".into(), CONTEXT.to_vec())];
    if variant == NamingVariant::AmbiguousContexts {
        contexts.push(("contexts/alias.jsonld".into(), CONTEXT.to_vec()));
    }
    a_signed_provider_at_with_contexts("example.com/morphir/naming", |native| {
        let string_dict = json!({"Reference": {
            "fqname": "morphir/SDK:dict#dict",
            "args": ["morphir/SDK:string#string", "morphir/SDK:string#string"]
        }});
        let empty = json!({"Reference": {"fqname": "morphir/SDK:dict#empty"}});
        native["distribution"]["Library"]["packageName"] = json!("morphir/naming");
        native["distribution"]["Library"]["def"]["modules"] = json!({"naming": {"Public": {
            "types": {},
            "values": {"target-names": {"Public": {"ExpressionBody": {
                "inputTypes": {},
                "outputType": {"Record": {"fields": {"frontend": string_dict, "backend": string_dict}}},
                "body": {"Record": {"fields": {"frontend": empty, "backend": empty}}}
            }}}}
        }}});
        native["$meta"]["@graph"] = json!([{
            "@id": "morphir://ir/pkg/morphir/naming?format=4.1.0#/module/naming/value/target-names",
            "subject-role": "ValueDefinition", "object-form": "data",
            "interpreter": "target-name-language-ids"
        }]);
        match variant {
            NamingVariant::Compatible | NamingVariant::AmbiguousContexts => {},
            NamingVariant::WrongType => {
                let value = &mut native["distribution"]["Library"]["def"]["modules"]["naming"]["Public"]["values"]["target-names"]["Public"]["ExpressionBody"];
                value["outputType"] = json!("morphir/SDK:basics#bool");
                value["body"] = json!({"Literal": {"BoolLiteral": true}});
            },
            NamingVariant::WrongSubjectRole => native["$meta"]["@graph"][0]["subject-role"] = json!("ValueSpecification"),
            NamingVariant::WrongObjectForm => native["$meta"]["@graph"][0]["object-form"] = json!("iri"),
            NamingVariant::WrongInterpreter => native["$meta"]["@graph"][0]["interpreter"] = json!("descriptive"),
        }
    }, contexts).await
}

async fn a_signed_provider_at(
    package_path: &str,
    mutate: impl FnOnce(&mut serde_json::Value),
) -> SignedProvider {
    a_signed_provider_at_with_contexts(
        package_path,
        mutate,
        vec![("contexts/schema.jsonld".into(), CONTEXT.to_vec())],
    )
    .await
}

async fn a_signed_provider_at_with_contexts(
    package_path: &str,
    mutate: impl FnOnce(&mut serde_json::Value),
    contexts: Vec<(String, Vec<u8>)>,
) -> SignedProvider {
    let directory = tempfile::tempdir().unwrap();
    let registry = directory.path().join("registry");
    let mut document = json!({
        "formatVersion": "4.1.0",
        "distribution": {"Library": {
            "packageName": "acme/metadata", "dependencies": {},
            "def": {"modules": {"lifecycle": {"Public": {
                "types": {}, "values": {"deprecated": {"Public": {"ExpressionBody": {
                    "inputTypes": {}, "outputType": "morphir/SDK:basics#bool",
                    "body": {"Literal": {"BoolLiteral": true}}
                }}}}
            }}}}
        }},
        "$meta": {"@context": "./contexts/schema.jsonld", "@graph": [{
            "@id": PREDICATE,
            "subject-role": ["ValueSpecification", "ValueDefinition"],
            "object-form": "data", "interpreter": "descriptive"
        }]}
    });
    mutate(&mut document);
    let exports: serde_json::Map<String, serde_json::Value> =
        document["distribution"]["Library"]["def"]["modules"]
            .as_object()
            .unwrap()
            .keys()
            .map(|name| (name.clone(), json!(name)))
            .collect();
    let native = serde_json::to_vec_pretty(&document).unwrap();
    let library = AuthoredLibrary::create_with_contexts(
        &serde_json::to_vec(&json!({
            "packagePath": package_path, "version": "1.0.0",
            "dependencies": {}, "exports": exports
        }))
        .unwrap(),
        &native,
        contexts.clone(),
    )
    .unwrap();
    let publisher = LocalSigningKey::from_seed([21; 32]);
    let signed = library.sign(&publisher).unwrap();
    let record: serde_json::Value = serde_json::from_slice(signed.record_bytes()).unwrap();
    let statement_path = record["statement"]["path"].as_str().unwrap();
    let release = record["release"].clone();
    let bundle = record["source"]["path"].as_str().unwrap().to_owned();
    write(
        &registry,
        &format!("{bundle}/manifest.json"),
        library.manifest_bytes(),
    );
    write(&registry, &format!("{bundle}/ir.json"), library.ir_bytes());
    for (path, bytes) in contexts {
        write(&registry, &format!("{bundle}/{path}"), &bytes);
    }

    let mut targets = tuf::targets_body(1);
    let record_path = "records/provider-1.0.0.json";
    let mut record_target = target(&registry, record_path, signed.record_bytes());
    record_target["custom"] = json!({"morphir": {
        "formatVersion": "0.1.0-draft.3", "kind": "LibraryRelease",
        "release": release, "status": "active"
    }});
    targets["targets"][record_path] = record_target;
    let mut statement_target = target(&registry, statement_path, signed.envelope_bytes());
    statement_target["custom"] = json!({"morphir": {
        "formatVersion": "0.1.0-draft.3", "kind": "LibraryReleaseStatement",
        "release": release
    }});
    targets["targets"][statement_path] = statement_target;
    let targets = tuf::sign(targets, &[(42, tuf::key(42))]);
    let root = tuf::root(1, 42);
    let snapshot = tuf::snapshot(&targets, 42);
    let timestamp = tuf::timestamp(&snapshot, 42);
    for (path, bytes) in [
        ("metadata/1.root.json", &root),
        ("metadata/1.targets.json", &targets),
        ("metadata/1.snapshot.json", &snapshot),
        ("metadata/1.timestamp.json", &timestamp),
        ("metadata/timestamp.json", &timestamp),
    ] {
        write(&registry, path, bytes);
    }
    let root_digest = Digest::of_bytes(&root).to_string();
    let policy = serde_json::to_vec(&json!({
        "formatVersion": "0.1.0-draft.3", "kind": "LibraryTrustPolicy",
        "repositories": [{"identity": root_digest,
            "bootstrapRoot": {"version": 1, "digest": root_digest},
            "namespaces": ["example.com"]}],
        "publisherRules": [{"namespace": "example.com",
            "publicKeys": [publisher.public_key_hex()], "threshold": 1}],
        "continuedUse": "fresh-metadata"
    }))
    .unwrap();
    mvp::initialize(InitializeRequest {
        policy: &policy,
        root: &root,
        state: &directory.path().join("trust"),
    })
    .unwrap();
    mvp::resolve(ResolveRequest {
        policy: &policy,
        root: ReleaseId::new(
            PackagePath::parse(package_path).unwrap(),
            StableVersion::parse("1.0.0").unwrap(),
        ),
        registry: &registry,
        state: &directory.path().join("trust"),
        output: &directory.path().join("morphir.lock"),
    })
    .await
    .unwrap();
    let lock = fs::read(directory.path().join("morphir.lock")).unwrap();
    SignedProvider {
        directory,
        policy,
        lock,
        native,
        manifest: library.manifest_bytes().to_vec(),
        bundle,
    }
}
