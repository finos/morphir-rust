//! Converter between Classic and V4 Morphir IR formats.
//!
//! This module provides bidirectional conversion between the Classic (V1/V2/V3)
//! and V4 Morphir IR formats.

use crate::ir::{classic, v4};
use crate::naming::Name;

/// Convert a Classic package to V4 format.
///
/// # Arguments
/// * `pkg` - The Classic package to convert
///
/// # Returns
/// A V4 PackageDefinition with converted modules
pub fn classic_to_v4(pkg: classic::Package) -> v4::Package {
    let modules = pkg
        .modules
        .into_iter()
        .map(convert_module_to_v4)
        .collect();

    v4::PackageDefinition { modules }
}

/// Convert a V4 package to Classic format.
///
/// # Arguments
/// * `pkg` - The V4 package to convert
///
/// # Returns
/// A Classic Package with converted modules
pub fn v4_to_classic(pkg: v4::Package) -> classic::Package {
    let modules = pkg
        .modules
        .into_iter()
        .map(convert_module_to_classic)
        .collect();

    classic::Package {
        name: String::new(), // Name is stored at distribution level
        modules,
    }
}

/// Convert a Classic module to V4 ModuleDefinitionEntry
fn convert_module_to_v4(module: classic::Module) -> v4::ModuleDefinitionEntry {
    let access = convert_access_to_v4(&module.detail.access);
    let types = convert_types_to_v4(&module.detail.value.types);
    let values = convert_values_to_v4(&module.detail.value.values);

    v4::ModuleDefinitionEntry(
        module.name,
        v4::AccessControlledModuleDefinition {
            access,
            value: v4::ModuleDefinition {
                types,
                values,
                doc: module.detail.value.doc,
            },
        },
    )
}

/// Convert a V4 ModuleDefinitionEntry to Classic Module
fn convert_module_to_classic(entry: v4::ModuleDefinitionEntry) -> classic::Module {
    let v4::ModuleDefinitionEntry(name, access_controlled) = entry;

    classic::Module {
        name,
        detail: classic::ModuleDetail {
            access: convert_access_to_classic(&access_controlled.access),
            value: classic::ModuleValue {
                types: convert_types_to_classic(&access_controlled.value.types),
                values: convert_values_to_classic(&access_controlled.value.values),
                doc: access_controlled.value.doc,
            },
        },
    }
}

/// Convert access string to V4 Access enum
fn convert_access_to_v4(access: &str) -> v4::Access {
    match access.to_lowercase().as_str() {
        "private" => v4::Access::Private,
        _ => v4::Access::Public,
    }
}

/// Convert V4 Access enum to Classic access string
fn convert_access_to_classic(access: &v4::Access) -> String {
    match access {
        v4::Access::Public => "Public".to_string(),
        v4::Access::Private => "Private".to_string(),
    }
}

/// Convert Classic types array to V4 TypeDefinitionEntry list.
///
/// Classic format: `[[[name_parts], {access, value}], ...]`
/// V4 format: `[TypeDefinitionEntry(Name, AccessControlledTypeDefinition), ...]`
fn convert_types_to_v4(types: &[serde_json::Value]) -> Vec<v4::TypeDefinitionEntry> {
    types
        .iter()
        .filter_map(|type_val| {
            // Classic format: [[name_parts], {access, value}]
            let arr = type_val.as_array()?;
            if arr.len() < 2 {
                return None;
            }

            // Extract name from first element
            let name = extract_name_from_json(&arr[0])?;

            // Extract access-controlled definition from second element
            let def_obj = arr[1].as_object()?;
            let access_str = def_obj.get("access")?.as_str()?;
            let value = def_obj.get("value")?.clone();

            Some(v4::TypeDefinitionEntry(
                name,
                v4::AccessControlledTypeDefinition {
                    access: convert_access_to_v4(access_str),
                    value: convert_type_definition_to_v4(&value),
                },
            ))
        })
        .collect()
}

/// Convert V4 TypeDefinitionEntry list to Classic types array
fn convert_types_to_classic(types: &[v4::TypeDefinitionEntry]) -> Vec<serde_json::Value> {
    types
        .iter()
        .map(|entry| {
            let v4::TypeDefinitionEntry(name, access_controlled) = entry;

            let name_json = serde_json::to_value(name).unwrap_or(serde_json::Value::Null);
            let def_json = serde_json::json!({
                "access": convert_access_to_classic(&access_controlled.access),
                "value": convert_type_definition_to_classic(&access_controlled.value)
            });

            serde_json::json!([name_json, def_json])
        })
        .collect()
}

/// Convert Classic values array to V4 ValueDefinitionEntry list.
///
/// Classic format: `[[[name_parts], {access, value}], ...]`
/// V4 format: `[ValueDefinitionEntry(Name, AccessControlledValueDefinition), ...]`
fn convert_values_to_v4(values: &[serde_json::Value]) -> Vec<v4::ValueDefinitionEntry> {
    values
        .iter()
        .filter_map(|value_val| {
            // Classic format: [[name_parts], {access, value}]
            let arr = value_val.as_array()?;
            if arr.len() < 2 {
                return None;
            }

            // Extract name from first element
            let name = extract_name_from_json(&arr[0])?;

            // Extract access-controlled definition from second element
            let def_obj = arr[1].as_object()?;
            let access_str = def_obj.get("access")?.as_str()?;
            let value = def_obj.get("value")?.clone();

            Some(v4::ValueDefinitionEntry(
                name,
                v4::AccessControlledValueDefinition {
                    access: convert_access_to_v4(access_str),
                    value: convert_value_definition_to_v4(&value),
                },
            ))
        })
        .collect()
}

/// Convert V4 ValueDefinitionEntry list to Classic values array
fn convert_values_to_classic(values: &[v4::ValueDefinitionEntry]) -> Vec<serde_json::Value> {
    values
        .iter()
        .map(|entry| {
            let v4::ValueDefinitionEntry(name, access_controlled) = entry;

            let name_json = serde_json::to_value(name).unwrap_or(serde_json::Value::Null);
            let def_json = serde_json::json!({
                "access": convert_access_to_classic(&access_controlled.access),
                "value": convert_value_definition_to_classic(&access_controlled.value)
            });

            serde_json::json!([name_json, def_json])
        })
        .collect()
}

/// Extract a Name from JSON value (array of strings)
fn extract_name_from_json(json: &serde_json::Value) -> Option<Name> {
    let arr = json.as_array()?;
    let words: Vec<String> = arr
        .iter()
        .filter_map(|v| v.as_str().map(String::from))
        .collect();

    if words.is_empty() {
        None
    } else {
        Some(Name { words })
    }
}

/// Convert a Classic type definition value to V4 TypeDefinition.
///
/// The type definition structure varies based on the kind (TypeAlias, CustomType, etc.)
/// For now, we preserve the JSON structure as-is since both formats use compatible representations.
fn convert_type_definition_to_v4(value: &serde_json::Value) -> v4::TypeDefinition {
    // The value contains doc and a nested value structure
    // Try to extract the actual type definition
    if let Some(obj) = value.as_object() {
        if let Some(inner_value) = obj.get("value") {
            // Check if it's a CustomTypeDefinition or TypeAliasDefinition
            if let Some(arr) = inner_value.as_array() {
                if !arr.is_empty() {
                    let tag = arr[0].as_str().unwrap_or("");
                    match tag {
                        "CustomTypeDefinition" => {
                            return convert_custom_type_to_v4(arr);
                        }
                        "TypeAliasDefinition" => {
                            return convert_type_alias_to_v4(arr);
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    // Fallback: create empty type alias
    v4::TypeDefinition::TypeAlias(v4::TypeAliasDefinition {
        type_params: vec![],
        type_exp: serde_json::Value::Null,
    })
}

/// Convert Classic TypeDefinition to Classic JSON value
fn convert_type_definition_to_classic(type_def: &v4::TypeDefinition) -> serde_json::Value {
    match type_def {
        v4::TypeDefinition::TypeAlias(alias) => {
            let type_params: Vec<serde_json::Value> = alias
                .type_params
                .iter()
                .map(|n| serde_json::to_value(n).unwrap_or(serde_json::Value::Null))
                .collect();

            serde_json::json!({
                "value": ["TypeAliasDefinition", type_params, alias.type_exp]
            })
        }
        v4::TypeDefinition::CustomType(custom) => {
            let type_params: Vec<serde_json::Value> = custom
                .type_params
                .iter()
                .map(|n| serde_json::to_value(n).unwrap_or(serde_json::Value::Null))
                .collect();

            let constructors = convert_constructors_to_classic(&custom.constructors);

            serde_json::json!({
                "value": ["CustomTypeDefinition", type_params, constructors]
            })
        }
    }
}

/// Convert Classic CustomTypeDefinition array to V4
fn convert_custom_type_to_v4(arr: &[serde_json::Value]) -> v4::TypeDefinition {
    // Format: ["CustomTypeDefinition", [type_params], {access, value: constructors}]
    let type_params = if arr.len() > 1 {
        extract_type_params(&arr[1])
    } else {
        vec![]
    };

    let constructors = if arr.len() > 2 {
        convert_constructors_to_v4(&arr[2])
    } else {
        v4::AccessControlledConstructors {
            access: v4::Access::Public,
            value: vec![],
        }
    };

    v4::TypeDefinition::CustomType(v4::CustomTypeDefinition {
        type_params,
        constructors,
    })
}

/// Convert Classic TypeAliasDefinition array to V4
fn convert_type_alias_to_v4(arr: &[serde_json::Value]) -> v4::TypeDefinition {
    // Format: ["TypeAliasDefinition", [type_params], type_exp]
    let type_params = if arr.len() > 1 {
        extract_type_params(&arr[1])
    } else {
        vec![]
    };

    let type_exp = if arr.len() > 2 {
        arr[2].clone()
    } else {
        serde_json::Value::Null
    };

    v4::TypeDefinition::TypeAlias(v4::TypeAliasDefinition {
        type_params,
        type_exp,
    })
}

/// Extract type parameters from JSON array
fn extract_type_params(json: &serde_json::Value) -> Vec<Name> {
    json.as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| extract_name_from_json(v))
                .collect()
        })
        .unwrap_or_default()
}

/// Convert Classic constructors to V4 AccessControlledConstructors
fn convert_constructors_to_v4(json: &serde_json::Value) -> v4::AccessControlledConstructors {
    if let Some(obj) = json.as_object() {
        let access = obj
            .get("access")
            .and_then(|a| a.as_str())
            .map(convert_access_to_v4)
            .unwrap_or(v4::Access::Public);

        let constructors = obj
            .get("value")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|ctor| convert_constructor_to_v4(ctor))
                    .collect()
            })
            .unwrap_or_default();

        v4::AccessControlledConstructors {
            access,
            value: constructors,
        }
    } else {
        v4::AccessControlledConstructors {
            access: v4::Access::Public,
            value: vec![],
        }
    }
}

/// Convert V4 AccessControlledConstructors to Classic JSON
fn convert_constructors_to_classic(constructors: &v4::AccessControlledConstructors) -> serde_json::Value {
    let ctors: Vec<serde_json::Value> = constructors
        .value
        .iter()
        .map(|ctor| {
            let name_json = serde_json::to_value(&ctor.0).unwrap_or(serde_json::Value::Null);
            let args: Vec<serde_json::Value> = ctor
                .1
                .iter()
                .map(|(arg_name, arg_type)| {
                    serde_json::json!([
                        serde_json::to_value(arg_name).unwrap_or(serde_json::Value::Null),
                        arg_type
                    ])
                })
                .collect();

            serde_json::json!([name_json, args])
        })
        .collect();

    serde_json::json!({
        "access": convert_access_to_classic(&constructors.access),
        "value": ctors
    })
}

/// Convert a single constructor from Classic to V4
fn convert_constructor_to_v4(json: &serde_json::Value) -> Option<v4::ConstructorDefinition> {
    // Format: [[name_parts], [[arg_name, arg_type], ...]]
    let arr = json.as_array()?;
    if arr.len() < 2 {
        return None;
    }

    let name = extract_name_from_json(&arr[0])?;

    let args: Vec<(Name, v4::Type)> = arr[1]
        .as_array()
        .map(|args_arr| {
            args_arr
                .iter()
                .filter_map(|arg| {
                    let arg_arr = arg.as_array()?;
                    if arg_arr.len() < 2 {
                        return None;
                    }
                    let arg_name = extract_name_from_json(&arg_arr[0])?;
                    let arg_type = arg_arr[1].clone();
                    Some((arg_name, arg_type))
                })
                .collect()
        })
        .unwrap_or_default();

    Some(v4::ConstructorDefinition(name, args))
}

/// Convert a Classic value definition to V4 ValueDefinition.
fn convert_value_definition_to_v4(value: &serde_json::Value) -> v4::ValueDefinition {
    // Classic format: {inputTypes: [...], outputType: ..., body: ...}
    if let Some(obj) = value.as_object() {
        let input_types = obj
            .get("inputTypes")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|input| {
                        let input_arr = input.as_array()?;
                        if input_arr.len() < 3 {
                            return None;
                        }
                        let name = extract_name_from_json(&input_arr[0])?;
                        let attrs = input_arr[1].clone();
                        let typ = input_arr[2].clone();
                        Some((name, attrs, typ))
                    })
                    .collect()
            })
            .unwrap_or_default();

        let output_type = obj
            .get("outputType")
            .cloned()
            .unwrap_or(serde_json::Value::Null);

        let body = obj
            .get("body")
            .cloned()
            .unwrap_or(serde_json::Value::Null);

        v4::ValueDefinition {
            input_types,
            output_type,
            body,
        }
    } else {
        v4::ValueDefinition {
            input_types: vec![],
            output_type: serde_json::Value::Null,
            body: serde_json::Value::Null,
        }
    }
}

/// Convert V4 ValueDefinition to Classic JSON value
fn convert_value_definition_to_classic(value_def: &v4::ValueDefinition) -> serde_json::Value {
    let input_types: Vec<serde_json::Value> = value_def
        .input_types
        .iter()
        .map(|(name, attrs, typ)| {
            serde_json::json!([
                serde_json::to_value(name).unwrap_or(serde_json::Value::Null),
                attrs,
                typ
            ])
        })
        .collect();

    serde_json::json!({
        "inputTypes": input_types,
        "outputType": value_def.output_type,
        "body": value_def.body
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::naming::Path;

    #[test]
    fn test_access_conversion() {
        assert_eq!(convert_access_to_v4("Public"), v4::Access::Public);
        assert_eq!(convert_access_to_v4("public"), v4::Access::Public);
        assert_eq!(convert_access_to_v4("Private"), v4::Access::Private);
        assert_eq!(convert_access_to_v4("private"), v4::Access::Private);
        assert_eq!(convert_access_to_v4("unknown"), v4::Access::Public);

        assert_eq!(convert_access_to_classic(&v4::Access::Public), "Public");
        assert_eq!(convert_access_to_classic(&v4::Access::Private), "Private");
    }

    #[test]
    fn test_name_extraction() {
        let json = serde_json::json!(["test", "name"]);
        let name = extract_name_from_json(&json).unwrap();
        assert_eq!(name.words, vec!["test", "name"]);

        let empty = serde_json::json!([]);
        assert!(extract_name_from_json(&empty).is_none());
    }

    #[test]
    fn test_empty_package_conversion() {
        let classic_pkg = classic::Package {
            name: "test".to_string(),
            modules: vec![],
        };

        let v4_pkg = classic_to_v4(classic_pkg);
        assert!(v4_pkg.modules.is_empty());
    }

    #[test]
    fn test_module_conversion_roundtrip() {
        let classic_module = classic::Module {
            name: Path::new("test/module"),
            detail: classic::ModuleDetail {
                access: "Public".to_string(),
                value: classic::ModuleValue {
                    types: vec![],
                    values: vec![],
                    doc: Some("Test module".to_string()),
                },
            },
        };

        let v4_entry = convert_module_to_v4(classic_module.clone());
        let roundtrip = convert_module_to_classic(v4_entry);

        assert_eq!(roundtrip.name.to_string(), classic_module.name.to_string());
        assert_eq!(roundtrip.detail.access, classic_module.detail.access);
        assert_eq!(roundtrip.detail.value.doc, classic_module.detail.value.doc);
    }
}
