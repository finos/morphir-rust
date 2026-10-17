//! Bounded JSON inspection retaining arbitrary-precision scalar lexemes.
//!
//! Borrowed RawValue validation scans nested spans iteratively in serde_json.
//! Our container traversal is also iterative; only one container is deserialized
//! at a time, so neither numeric shims nor the default recursion128 affect node
//! accounting. Rescanning borrowed child spans costs O(bytes * depth) worst
//! case, bounded by the negotiated byte/depth budgets, without a recursive tree.

use super::{ModelAdmissionError, ModelTransportLimits};
use semver::Version;
use serde::de::{DeserializeSeed, Error as _, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::value::RawValue;
use std::collections::{HashMap, HashSet};
use std::fmt;

fn invalid() -> ModelAdmissionError {
    ModelAdmissionError::new("invalid_json")
}
fn invalid_distribution() -> ModelAdmissionError {
    ModelAdmissionError::new("invalid_distribution")
}

fn charge(
    depth: u32,
    limits: &ModelTransportLimits,
    nodes: &mut u64,
) -> Result<(), ModelAdmissionError> {
    if depth > limits.max_depth {
        return Err(ModelAdmissionError::new("depth_limit"));
    }
    *nodes = nodes
        .checked_add(1)
        .ok_or_else(|| ModelAdmissionError::new("node_limit"))?;
    if *nodes > u64::from(limits.max_nodes) {
        return Err(ModelAdmissionError::new("node_limit"));
    }
    Ok(())
}

struct Child<'a> {
    depth: u32,
    limits: &'a ModelTransportLimits,
    nodes: &'a mut u64,
}
impl<'de> DeserializeSeed<'de> for Child<'_> {
    type Value = &'de RawValue;
    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        charge(self.depth, self.limits, self.nodes).map_err(D::Error::custom)?;
        <&RawValue>::deserialize(deserializer)
    }
}

struct Children<'a> {
    depth: u32,
    limits: &'a ModelTransportLimits,
    nodes: &'a mut u64,
}
impl<'de> Visitor<'de> for Children<'_> {
    type Value = Vec<&'de RawValue>;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a shallow JSON container")
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let mut values = Vec::new();
        while let Some(value) = seq.next_element_seed(Child {
            depth: self.depth,
            limits: self.limits,
            nodes: self.nodes,
        })? {
            values.push(value);
        }
        Ok(values)
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut values = Vec::new();
        let mut keys = HashSet::new();
        while let Some(key) = map.next_key::<String>()? {
            if !keys.insert(key) {
                return Err(A::Error::custom("duplicate_member"));
            }
            values.push(map.next_value_seed(Child {
                depth: self.depth,
                limits: self.limits,
                nodes: self.nodes,
            })?);
        }
        Ok(values)
    }
}

fn boundary_error(error: serde_json::Error) -> ModelAdmissionError {
    // Our seed's stable reason is wrapped with serde's source location. Payload
    // text and the underlying parser message never escape this boundary.
    let message = error.to_string();
    if message.contains("depth_limit") {
        ModelAdmissionError::new("depth_limit")
    } else if message.contains("node_limit") {
        ModelAdmissionError::new("node_limit")
    } else {
        invalid()
    }
}

pub(super) fn inspect_json(
    bytes: &[u8],
    profile: Option<(&str, &str)>,
    limits: &ModelTransportLimits,
    nodes: &mut u64,
) -> Result<(), ModelAdmissionError> {
    charge(1, limits, nodes)?;
    let root: &RawValue = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    let mut pending = vec![(root, 1_u32)];
    while let Some((value, depth)) = pending.pop() {
        let mut reader = serde_json::Deserializer::from_str(value.get());
        let children = Children {
            depth: depth + 1,
            limits,
            nodes,
        };
        let values = match value.get().as_bytes().first() {
            Some(b'{') => reader.deserialize_map(children).map_err(boundary_error)?,
            Some(b'[') => reader.deserialize_seq(children).map_err(boundary_error)?,
            Some(b'"') => {
                // RawValue validates escape spelling but intentionally skips
                // surrogate pairing. Match the normal UTF-8 JSON string reader.
                serde_json::from_str::<String>(value.get()).map_err(|_| invalid())?;
                continue;
            }
            // RawValue already validated each exact numeric scalar without
            // converting its lexeme to f64 or a private arbitrary-precision map.
            _ => continue,
        };
        reader.end().map_err(|_| invalid())?;
        pending.extend(values.into_iter().map(|child| (child, depth + 1)));
    }
    if let Some((profile, version)) = profile {
        inspect_profile(root, profile, version)?;
    }
    Ok(())
}

type Members<'a> = HashMap<String, &'a RawValue>;
fn object(value: &RawValue) -> Result<Members<'_>, ModelAdmissionError> {
    serde_json::from_str(value.get()).map_err(|_| invalid_distribution())
}
fn required<'a>(members: &Members<'a>, name: &str) -> Result<&'a RawValue, ModelAdmissionError> {
    members.get(name).copied().ok_or_else(invalid_distribution)
}
fn is_object(value: &RawValue) -> bool {
    value.get().starts_with('{')
}
fn is_array(value: &RawValue) -> bool {
    value.get().starts_with('[')
}

fn inspect_profile(
    root: &RawValue,
    profile: &str,
    version: &str,
) -> Result<(), ModelAdmissionError> {
    if profile != "morphir-ir" {
        return Err(ModelAdmissionError::new("invalid_profile"));
    }
    let expected =
        Version::parse(version).map_err(|_| ModelAdmissionError::new("unsupported_ir_version"))?;
    if !expected.pre.is_empty() || !expected.build.is_empty() || !matches!(expected.major, 3 | 4) {
        return Err(ModelAdmissionError::new("unsupported_ir_version"));
    }
    let root = object(root)?;
    let declared = required(&root, "formatVersion")?;
    let declared = if declared.get().starts_with('"') {
        serde_json::from_str::<String>(declared.get()).ok()
    } else {
        serde_json::from_str::<u32>(declared.get())
            .ok()
            .map(|major| format!("{major}.0.0"))
    };
    if declared.as_deref() != Some(version) {
        return Err(ModelAdmissionError::new("unsupported_ir_version"));
    }
    let distribution = required(&root, "distribution")?;
    if expected.major == 3 {
        return inspect_classic(distribution, &expected);
    }
    let variants = object(distribution)?;
    if variants.len() != 1 {
        return Err(invalid_distribution());
    }
    let (kind, body) = variants.iter().next().unwrap();
    let body = object(body)?;
    let name = serde_json::from_str::<String>(required(&body, "packageName")?.get())
        .map_err(|_| invalid_distribution())?;
    if name.trim().is_empty() {
        return Err(invalid_distribution());
    }
    if body
        .get("dependencies")
        .is_some_and(|value| !is_object(value))
    {
        return Err(invalid_distribution());
    }
    let package = match kind.as_str() {
        "Library" => body.get("def"),
        "Specs" => body.get("spec"),
        "Application" => {
            if !is_object(required(&body, "entryPoints")?) {
                return Err(invalid_distribution());
            }
            body.get("def")
        }
        _ => return Err(invalid_distribution()),
    };
    if let Some(package) = package {
        let package = object(package)?;
        if package
            .get("modules")
            .is_some_and(|value| !is_object(value))
        {
            return Err(invalid_distribution());
        }
    }
    Ok(())
}

fn inspect_classic(value: &RawValue, version: &Version) -> Result<(), ModelAdmissionError> {
    let values: Vec<&RawValue> =
        serde_json::from_str(value.get()).map_err(|_| invalid_distribution())?;
    if values.len() != 4 {
        return Err(invalid_distribution());
    }
    let kind: String = serde_json::from_str(values[0].get()).map_err(|_| invalid_distribution())?;
    if kind != "Library" && !(kind == "Specs" && version.minor >= 1) {
        return Err(invalid_distribution());
    }
    if !is_array(values[1]) || !is_array(values[2]) {
        return Err(invalid_distribution());
    }
    let package = object(values[3])?;
    if !is_array(required(&package, "modules")?) {
        return Err(invalid_distribution());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits(depth: u32, nodes: u32) -> ModelTransportLimits {
        ModelTransportLimits {
            max_encoded_bytes: 65536,
            max_decoded_bytes: 49152,
            max_depth: depth,
            max_nodes: nodes,
        }
    }

    #[test]
    fn exact_numeric_scalar_lexemes_count_once_at_depth_one() {
        for scalar in [
            "12345678901234567890123456789012345678901234567890",
            "0.1234567890123456789012345678901234567890123456789000",
            "1.234567890123456789e999999",
        ] {
            let mut nodes = 0;
            inspect_json(scalar.as_bytes(), None, &limits(1, 1), &mut nodes).unwrap();
            assert_eq!(nodes, 1);
        }
    }

    #[test]
    fn negotiated_depth_above_128_is_supported_without_recursive_traversal() {
        let document = format!("{}0{}", "[".repeat(150), "]".repeat(150));
        let mut nodes = 0;
        inspect_json(document.as_bytes(), None, &limits(151, 151), &mut nodes).unwrap();
        assert_eq!(nodes, 151);
    }

    #[test]
    fn exact_depth_and_aggregate_node_budgets_remain_enforced() {
        assert_eq!(
            inspect_json(b"[[0]]", None, &limits(2, 99), &mut 0)
                .unwrap_err()
                .code(),
            "depth_limit"
        );
        assert_eq!(
            inspect_json(b"[0,1]", None, &limits(99, 2), &mut 0)
                .unwrap_err()
                .code(),
            "node_limit"
        );
        let mut nodes = 2;
        inspect_json(b"0", None, &limits(1, 3), &mut nodes).unwrap();
        assert_eq!(nodes, 3);
        assert_eq!(
            inspect_json(b"0", None, &limits(1, 3), &mut nodes)
                .unwrap_err()
                .code(),
            "node_limit"
        );
    }

    #[test]
    fn duplicate_members_and_malformed_documents_are_refused() {
        for document in [
            "{\"same\":0,\"same\":1}",
            "{\"same\":0,\"s\\u0061me\":1}",
            "[1,]",
            "true false",
            "{\"x\":{\"a\":0,\"a\":1}}",
        ] {
            assert!(
                inspect_json(document.as_bytes(), None, &limits(99, 99), &mut 0).is_err(),
                "{document}"
            );
        }
    }

    #[test]
    fn borrowed_string_lexemes_still_require_valid_unicode_escapes() {
        for document in [r#""\uD800""#, r#"{"future":"\uD800"}"#] {
            assert!(inspect_json(document.as_bytes(), None, &limits(99, 99), &mut 0).is_err());
        }
        inspect_json(br#""\uD83D\uDE00""#, None, &limits(1, 1), &mut 0).unwrap();
    }
}
