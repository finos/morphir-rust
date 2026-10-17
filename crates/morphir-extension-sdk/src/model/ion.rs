//! Bounded lazy inspection of retained Ion binary documents.
//!
//! Inspection never rewrites the application bytes or establishes a semantic
//! guarantee. Containers are traversed iteratively before structural checks.

use super::{ModelAdmissionError, ModelTransportLimits};
use ion_rs::{
    IonResult, LazyStruct, LazyValue, RawVersionMarker, SystemReader, SystemStreamItem, ValueRef,
    v1_0::Binary,
};
use semver::Version;

fn invalid() -> ModelAdmissionError {
    ModelAdmissionError::new("invalid_ion")
}
fn profile_error() -> ModelAdmissionError {
    ModelAdmissionError::new("invalid_profile")
}

pub(super) fn inspect_ion(
    bytes: &[u8],
    profile: Option<(&str, &str)>,
    limits: &ModelTransportLimits,
    nodes: &mut u64,
) -> Result<(), ModelAdmissionError> {
    if !bytes.starts_with(&[0xe0, 1, 0, 0xea]) {
        return Err(invalid());
    }
    // Binary 1.0 disables Ion 1.1 macro expansion. The default catalog is empty:
    // ion-rs 1.0.1 rejects named shared imports before consulting max_id or
    // allocating placeholder symbols. Local symbols are bounded by input bytes.
    let mut reader = SystemReader::new(Binary, bytes);
    let mut application_values = 0;
    loop {
        match reader.next_item().map_err(|_| invalid())? {
            SystemStreamItem::VersionMarker(marker) => {
                if marker.major() != 1 || marker.minor() != 0 {
                    return Err(invalid());
                }
            }
            SystemStreamItem::SymbolTable(_) => {}
            SystemStreamItem::Value(value) => {
                application_values += 1;
                if application_values != 1 {
                    return Err(invalid());
                }
                walk(value, limits, nodes)?;
                if let Some((name, version)) = profile {
                    inspect_profile(value, name, version)?;
                }
            }
            SystemStreamItem::EndOfStream(_) => break,
            _ => return Err(invalid()),
        }
    }
    if application_values == 1 {
        Ok(())
    } else {
        Err(invalid())
    }
}

type Values<'a> = Box<dyn Iterator<Item = IonResult<LazyValue<'a, Binary>>> + 'a>;

fn walk<'a>(
    root: LazyValue<'a, Binary>,
    limits: &ModelTransportLimits,
    nodes: &mut u64,
) -> Result<(), ModelAdmissionError> {
    let mut stack: Vec<(Values<'a>, u32)> = vec![(Box::new(std::iter::once(Ok(root))), 1)];
    while let Some((values, depth)) = stack.last_mut() {
        let Some(value) = values.next() else {
            stack.pop();
            continue;
        };
        let depth = *depth;
        if depth > limits.max_depth {
            return Err(ModelAdmissionError::new("depth_limit"));
        }
        *nodes = nodes
            .checked_add(1)
            .ok_or_else(|| ModelAdmissionError::new("node_limit"))?;
        if *nodes > u64::from(limits.max_nodes) {
            return Err(ModelAdmissionError::new("node_limit"));
        }
        let value = value.map_err(|_| invalid())?;
        for annotation in value.annotations() {
            annotation.map_err(|_| invalid())?;
        }
        let children: Option<Values<'a>> = match value.read().map_err(|_| invalid())? {
            ValueRef::List(list) => Some(Box::new(list.iter())),
            ValueRef::SExp(sexp) => Some(Box::new(sexp.iter())),
            ValueRef::Struct(structure) => Some(Box::new(structure.iter().map(|field| {
                let field = field?;
                field.name()?;
                Ok(field.value())
            }))),
            _ => None,
        };
        if let Some(children) = children {
            stack.push((children, depth + 1));
        }
    }
    Ok(())
}

fn required<'a>(
    structure: LazyStruct<'a, Binary>,
    name: &str,
) -> Result<LazyValue<'a, Binary>, ModelAdmissionError> {
    optional(structure, name)?.ok_or_else(profile_error)
}

fn optional<'a>(
    structure: LazyStruct<'a, Binary>,
    name: &str,
) -> Result<Option<LazyValue<'a, Binary>>, ModelAdmissionError> {
    let mut found = None;
    for field in structure.iter() {
        let field = field.map_err(|_| invalid())?;
        if field.name().map_err(|_| invalid())? == name {
            if found.is_some() {
                return Err(profile_error());
            }
            found = Some(field.value());
        }
    }
    Ok(found)
}

fn as_struct(value: LazyValue<'_, Binary>) -> Result<LazyStruct<'_, Binary>, ModelAdmissionError> {
    value
        .read()
        .map_err(|_| invalid())?
        .expect_struct()
        .map_err(|_| profile_error())
}

fn nonempty_text(value: LazyValue<'_, Binary>) -> Result<(), ModelAdmissionError> {
    match value.read().map_err(|_| invalid())? {
        ValueRef::String(text) if !text.trim().is_empty() => Ok(()),
        _ => Err(profile_error()),
    }
}

fn inspect_profile(
    root: LazyValue<'_, Binary>,
    profile: &str,
    expected_version: &str,
) -> Result<(), ModelAdmissionError> {
    if profile != "morphir-model" {
        return Err(profile_error());
    }
    let mut annotated = false;
    for annotation in root.annotations() {
        if annotation.map_err(|_| invalid())? == "morphir_model" {
            annotated = true;
        }
    }
    if !annotated {
        return Err(profile_error());
    }
    let body = as_struct(root)?;
    inspect_ir(as_struct(required(body, "ir")?)?, expected_version)?;
    let enrichments = required(body, "enrichments")?
        .read()
        .map_err(|_| invalid())?
        .expect_list()
        .map_err(|_| profile_error())?;
    for enrichment in enrichments.iter() {
        let enrichment = as_struct(enrichment.map_err(|_| invalid())?)?;
        nonempty_text(required(enrichment, "owner")?)?;
        required(enrichment, "data")?;
    }
    required(body, "provenance")?
        .read()
        .map_err(|_| invalid())?
        .expect_list()
        .map_err(|_| profile_error())?;
    Ok(())
}

fn inspect_ir(ir: LazyStruct<'_, Binary>, expected: &str) -> Result<(), ModelAdmissionError> {
    let unsupported = || ModelAdmissionError::new("unsupported_ir_version");
    let expected = Version::parse(expected).map_err(|_| unsupported())?;
    if !expected.pre.is_empty() || !expected.build.is_empty() || !matches!(expected.major, 3 | 4) {
        return Err(unsupported());
    }
    let declared = required(ir, "formatVersion")?
        .read()
        .map_err(|_| invalid())?;
    let same = match declared {
        ValueRef::String(version) => *version == expected.to_string(),
        ValueRef::Int(major) if expected.minor == 0 && expected.patch == 0 => {
            major.as_i64() == Some(expected.major as i64)
        }
        _ => false,
    };
    if !same {
        return Err(unsupported());
    }
    let distribution = required(ir, "distribution")?;
    if expected.major == 3 {
        return inspect_classic_distribution(distribution, &expected);
    }
    let distribution = as_struct(distribution)?;
    let mut fields = distribution.iter();
    let field = fields
        .next()
        .ok_or_else(profile_error)?
        .map_err(|_| invalid())?;
    if fields.next().is_some() {
        return Err(profile_error());
    }
    let kind = field.name().map_err(|_| invalid())?;
    let content = as_struct(field.value())?;
    nonempty_text(required(content, "packageName")?)?;
    if let Some(dependencies) = optional(content, "dependencies")? {
        as_struct(dependencies)?;
    }
    // These are accepted reader defaults, not a requirement to reproduce every
    // field emitted by a canonical writer. A present member must keep its type.
    let package = if kind == "Library" {
        optional(content, "def")?
    } else if kind == "Specs" {
        optional(content, "spec")?
    } else if kind == "Application" {
        as_struct(required(content, "entryPoints")?)?;
        optional(content, "def")?
    } else {
        return Err(profile_error());
    };
    if let Some(package) = package
        && let Some(modules) = optional(as_struct(package)?, "modules")?
    {
        as_struct(modules)?;
    }
    Ok(())
}

fn inspect_classic_distribution(
    value: LazyValue<'_, Binary>,
    version: &Version,
) -> Result<(), ModelAdmissionError> {
    let list = value
        .read()
        .map_err(|_| invalid())?
        .expect_list()
        .map_err(|_| profile_error())?;
    let values = list
        .iter()
        .take(5)
        .collect::<IonResult<Vec<_>>>()
        .map_err(|_| invalid())?;
    if values.len() != 4 {
        return Err(profile_error());
    }
    let kind = values[0].read().map_err(|_| invalid())?;
    let kind = kind.expect_string().map_err(|_| profile_error())?;
    if kind != "Library" && !(kind == "Specs" && version.minor >= 1) {
        return Err(profile_error());
    }
    values[1]
        .read()
        .map_err(|_| invalid())?
        .expect_list()
        .map_err(|_| profile_error())?;
    values[2]
        .read()
        .map_err(|_| invalid())?
        .expect_list()
        .map_err(|_| profile_error())?;
    let package = as_struct(values[3])?;
    required(package, "modules")?
        .read()
        .map_err(|_| invalid())?
        .expect_list()
        .map_err(|_| profile_error())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ion_rs::{Element, v1_0::Binary};

    fn limits() -> ModelTransportLimits {
        ModelTransportLimits {
            max_encoded_bytes: 65536,
            max_decoded_bytes: 49152,
            max_depth: 64,
            max_nodes: 4096,
        }
    }

    fn binary(text: &str) -> Vec<u8> {
        Element::read_one(text).unwrap().encode_as(Binary).unwrap()
    }

    #[test]
    fn genuine_rich_ion_is_inspected_without_rewriting() {
        let bytes = include_bytes!("../../tests/fixtures/model_transport/fixtures/rich-model.ionb");
        let mut nodes = 0;
        inspect_ion(
            bytes,
            Some(("morphir-model", "4.0.0")),
            &limits(),
            &mut nodes,
        )
        .unwrap();
        assert!(nodes > 20);
    }

    #[test]
    fn scalar_evidence_counts_only_the_application_root() {
        let mut nodes = 0;
        inspect_ion(&binary("unknown::12.3400"), None, &limits(), &mut nodes).unwrap();
        assert_eq!(nodes, 1);
    }

    #[test]
    fn ion_tree_bounds_are_aggregate_and_root_is_depth_one() {
        let bytes = binary("[[1], 2]");
        let mut low = limits();
        low.max_depth = 2;
        assert!(
            inspect_ion(&bytes, None, &low, &mut 0)
                .unwrap_err()
                .to_string()
                .contains("depth_limit")
        );
        low.max_depth = 3;
        low.max_nodes = 3;
        assert!(
            inspect_ion(&bytes, None, &low, &mut 0)
                .unwrap_err()
                .to_string()
                .contains("node_limit")
        );
        low.max_nodes = 8;
        let mut nodes = 4;
        inspect_ion(&bytes, None, &low, &mut nodes).unwrap();
        assert_eq!(nodes, 8);
        assert!(inspect_ion(&binary("0"), None, &low, &mut nodes).is_err());
    }

    #[test]
    fn malformed_binary_and_multiple_application_documents_are_refused() {
        let bytes = binary("0");
        for invalid in [
            b"0".to_vec(),
            vec![0xe0, 1, 0, 0xea],
            vec![0xe0, 1, 0, 0xea, 0xbe],
            [bytes.clone(), bytes].concat(),
        ] {
            assert!(inspect_ion(&invalid, None, &limits(), &mut 0).is_err());
        }
    }

    #[test]
    fn profile_reserved_members_and_complete_ir_are_checked() {
        for text in [
            "{ir:{},enrichments:[],provenance:[]}",
            "morphir_model::{ir:{},ir:{},enrichments:[],provenance:[]}",
            "morphir_model::{ir:{formatVersion:4,distribution:{Library:{}}},enrichments:[],provenance:[]}",
            "morphir_model::{ir:{formatVersion:4,distribution:{Library:{packageName:\"x\",dependencies:{},def:{modules:{}}}}},enrichments:[{owner:\"\",data:0}],provenance:[]}",
        ] {
            assert!(
                inspect_ion(
                    &binary(text),
                    Some(("morphir-model", "4.0.0")),
                    &limits(),
                    &mut 0
                )
                .is_err(),
                "{text}"
            );
        }
    }

    #[test]
    fn shared_symbol_import_with_large_max_id_is_refused_by_empty_catalog() {
        let table = Element::read_one(
            "{imports:[{name:\"unavailable\",version:1,max_id:9223372036854775807}]}",
        )
        .unwrap()
        .with_annotations(["$ion_symbol_table"])
        .encode_as(Binary)
        .unwrap();
        let bytes = [table, binary("0")].concat();
        assert!(inspect_ion(&bytes, None, &limits(), &mut 0).is_err());
    }

    #[test]
    fn reserved_fields_are_unique_but_unknown_ion_struct_members_may_repeat() {
        let text = "morphir_model::{ir:{formatVersion:4,distribution:{Library:{packageName:\"x\",dependencies:{},def:{modules:{}}}}},enrichments:[{owner:\"urn:fixture:unknown\",data:one::two::{same:1,same:2,exact:123.4500}}],provenance:[],future:{same:1,same:2}}";
        let bytes = binary(text);
        inspect_ion(&bytes, Some(("morphir-model", "4.0.0")), &limits(), &mut 0).unwrap();
        assert!(
            inspect_ion(&bytes, Some(("morphir-model", "4.1.0")), &limits(), &mut 0)
                .unwrap_err()
                .to_string()
                .contains("unsupported_ir_version")
        );
        assert!(
            inspect_ion(
                &bytes,
                Some(("unknown-profile", "4.0.0")),
                &limits(),
                &mut 0
            )
            .is_err()
        );
    }

    #[test]
    fn classic_and_v4_distribution_carriers_remain_distinct() {
        for (ir, version) in [
            (
                "{formatVersion:3,distribution:[\"Library\",[[\"example\"]],[],{modules:[]}]}",
                "3.0.0",
            ),
            (
                "{formatVersion:\"3.1.0\",distribution:[\"Specs\",[[\"example\"]],[],{modules:[]}]}",
                "3.1.0",
            ),
            (
                "{formatVersion:4,distribution:{Specs:{packageName:\"x\",dependencies:{},spec:{modules:{}}}}}",
                "4.0.0",
            ),
            (
                "{formatVersion:4,distribution:{Application:{packageName:\"x\",dependencies:{},def:{modules:{}},entryPoints:{}}}}",
                "4.0.0",
            ),
        ] {
            let bytes = binary(&format!(
                "morphir_model::{{ir:{ir},enrichments:[],provenance:[]}}"
            ));
            inspect_ion(&bytes, Some(("morphir-model", version)), &limits(), &mut 0).unwrap();
        }
    }
}
