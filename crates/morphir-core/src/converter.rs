//! Converter between Classic and V4 Morphir IR formats.
//!
//! This module provides bidirectional conversion between the Classic (V1/V2/V3)
//! and V4 Morphir IR formats.

use crate::ir::{classic, v4};
use crate::naming::Name;
use indexmap::IndexMap;
use std::cell::RefCell;

// Thread-local configuration for conversion options
thread_local! {
    static USE_COMPACT_FORMAT: RefCell<bool> = const { RefCell::new(true) };
}

/// Set whether to use compact format for V4 output.
/// When false, uses expanded/verbose format.
fn set_compact_format(compact: bool) {
    USE_COMPACT_FORMAT.with(|f| *f.borrow_mut() = compact);
}

/// Check if compact format should be used.
fn is_compact_format() -> bool {
    USE_COMPACT_FORMAT.with(|f| *f.borrow())
}

// =============================================================================
// Type Expression Conversion (Classic Array -> V4 Object Wrapper)
// =============================================================================

/// Convert a Classic type expression (array format) to V4 format (object wrapper).
///
/// Classic format: `["TypeName", {attrs}, ...args]`
/// V4 format: `{ "TypeName": { field: value, ... } }`
fn convert_type_expr_to_v4(json: &serde_json::Value) -> serde_json::Value {
    let arr = match json.as_array() {
        Some(a) if !a.is_empty() => a,
        _ => return json.clone(), // Not an array or empty, return as-is
    };

    let tag = match arr[0].as_str() {
        Some(t) => t,
        None => return json.clone(), // First element not a string, return as-is
    };

    match tag {
        "Variable" => {
            // Classic: ["Variable", {attrs}, ["name", "parts"]]
            let name = if arr.len() > 2 {
                extract_name_from_json(&arr[2])
                    .map(|n| n.to_string())
                    .unwrap_or_default()
            } else {
                String::new()
            };
            // Compact: bare name string; Expanded: {"Variable": {"name": name}}
            if is_compact_format() {
                serde_json::json!(name)
            } else {
                serde_json::json!({ "Variable": { "name": name } })
            }
        }
        "Reference" => {
            // Classic: ["Reference", {attrs}, [pkg_path, mod_path, local_name], [type_args]]
            let fqname = if arr.len() > 2 {
                extract_fqname_from_classic(&arr[2])
            } else {
                String::new()
            };
            let args: Vec<serde_json::Value> = if arr.len() > 3 {
                arr[3]
                    .as_array()
                    .map(|a| a.iter().map(convert_type_expr_to_v4).collect())
                    .unwrap_or_default()
            } else {
                vec![]
            };
            if is_compact_format() {
                // Compact: bare fqname when no args, array [fqname, arg1, ...] with args
                if args.is_empty() {
                    serde_json::json!(fqname)
                } else {
                    let mut ref_arr = vec![serde_json::json!(fqname)];
                    ref_arr.extend(args);
                    serde_json::json!({ "Reference": ref_arr })
                }
            } else {
                // Expanded: always object with fqname and args fields
                serde_json::json!({ "Reference": { "fqname": fqname, "args": args } })
            }
        }
        "Tuple" => {
            // Classic: ["Tuple", {attrs}, [elements...]]
            let elements: Vec<serde_json::Value> = if arr.len() > 2 {
                arr[2]
                    .as_array()
                    .map(|a| a.iter().map(convert_type_expr_to_v4).collect())
                    .unwrap_or_default()
            } else {
                vec![]
            };
            serde_json::json!({ "Tuple": { "elements": elements } })
        }
        "Record" => {
            // Classic format can be either:
            // 1. Array format: ["Record", {attrs}, [[["field", "name"], type_expr], ...]]
            // 2. Object format: ["Record", {attrs}, [{"name": ["field"], "tpe": type_expr}, ...]]
            let fields: IndexMap<String, serde_json::Value> = if arr.len() > 2 {
                arr[2]
                    .as_array()
                    .map(|fields_arr| {
                        fields_arr
                            .iter()
                            .filter_map(|field| {
                                // Try object format first: {"name": [...], "tpe": ...}
                                if let Some(obj) = field.as_object() {
                                    let name = obj.get("name").and_then(extract_name_from_json)?;
                                    let tpe = obj
                                        .get("tpe")
                                        .map(convert_type_expr_to_v4)
                                        .unwrap_or(serde_json::Value::Null);
                                    return Some((name.to_string(), tpe));
                                }
                                // Fall back to array format: [["name"], type_expr]
                                let field_arr = field.as_array()?;
                                if field_arr.len() < 2 {
                                    return None;
                                }
                                let name = extract_name_from_json(&field_arr[0])?;
                                let tpe = convert_type_expr_to_v4(&field_arr[1]);
                                Some((name.to_string(), tpe))
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            } else {
                IndexMap::new()
            };
            // Compact format: fields directly under Record
            serde_json::json!({ "Record": fields })
        }
        "ExtensibleRecord" => {
            // Classic format can be either:
            // 1. Array format: ["ExtensibleRecord", {attrs}, ["var", "name"], [[field_name, type], ...]]
            // 2. Object format: ["ExtensibleRecord", {attrs}, ["var"], [{"name": [...], "tpe": ...}, ...]]
            let variable = if arr.len() > 2 {
                extract_name_from_json(&arr[2])
                    .map(|n| n.to_string())
                    .unwrap_or_default()
            } else {
                String::new()
            };
            let fields: IndexMap<String, serde_json::Value> = if arr.len() > 3 {
                arr[3]
                    .as_array()
                    .map(|fields_arr| {
                        fields_arr
                            .iter()
                            .filter_map(|field| {
                                // Try object format first: {"name": [...], "tpe": ...}
                                if let Some(obj) = field.as_object() {
                                    let name = obj.get("name").and_then(extract_name_from_json)?;
                                    let tpe = obj
                                        .get("tpe")
                                        .map(convert_type_expr_to_v4)
                                        .unwrap_or(serde_json::Value::Null);
                                    return Some((name.to_string(), tpe));
                                }
                                // Fall back to array format: [["name"], type_expr]
                                let field_arr = field.as_array()?;
                                if field_arr.len() < 2 {
                                    return None;
                                }
                                let name = extract_name_from_json(&field_arr[0])?;
                                let tpe = convert_type_expr_to_v4(&field_arr[1]);
                                Some((name.to_string(), tpe))
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            } else {
                IndexMap::new()
            };
            serde_json::json!({ "ExtensibleRecord": { "variable": variable, "fields": fields } })
        }
        "Function" => {
            // Classic: ["Function", {attrs}, arg_type, return_type]
            let arg = if arr.len() > 2 {
                convert_type_expr_to_v4(&arr[2])
            } else {
                serde_json::Value::Null
            };
            let result = if arr.len() > 3 {
                convert_type_expr_to_v4(&arr[3])
            } else {
                serde_json::Value::Null
            };
            serde_json::json!({ "Function": { "arg": arg, "result": result } })
        }
        "Unit" => {
            // Classic: ["Unit", {attrs}]
            serde_json::json!({ "Unit": {} })
        }
        _ => {
            // Unknown type tag, return as-is
            json.clone()
        }
    }
}

/// Extract FQName from Classic format and convert to canonical string.
///
/// Classic format: `[package_path, module_path, local_name]`
/// where each path is `[["word1"], ["word2", "word3"]]`
fn extract_fqname_from_classic(json: &serde_json::Value) -> String {
    let arr = match json.as_array() {
        Some(a) if a.len() >= 3 => a,
        _ => return String::new(),
    };

    // Extract package path: [["word1"], ["word2"]] -> "word1/word2"
    let package_path = extract_path_from_classic(&arr[0]);
    // Extract module path: [["word1", "word2"]] -> "word1-word2"
    let module_path = extract_path_from_classic(&arr[1]);
    // Extract local name: ["word1", "word2"] -> "word1-word2"
    let local_name = extract_name_from_json(&arr[2])
        .map(|n| n.to_string())
        .unwrap_or_default();

    format!("{}:{}#{}", package_path, module_path, local_name)
}

/// Extract Path from Classic format and convert to canonical string.
///
/// Classic format: `[["word1"], ["word2", "word3"]]` -> "word1/word2-word3"
fn extract_path_from_classic(json: &serde_json::Value) -> String {
    json.as_array()
        .map(|segments| {
            segments
                .iter()
                .filter_map(|seg| extract_name_from_json(seg).map(|n| n.to_string()))
                .collect::<Vec<_>>()
                .join("/")
        })
        .unwrap_or_default()
}

/// Convert a Classic value expression (array format) to V4 format (object wrapper).
///
/// Classic format: `["ValueTag", {attrs}, ...args]`
/// V4 format: `{ "ValueTag": { field: value, ... } }`
fn convert_value_expr_to_v4(json: &serde_json::Value) -> serde_json::Value {
    let arr = match json.as_array() {
        Some(a) if !a.is_empty() => a,
        _ => return json.clone(),
    };

    let tag = match arr[0].as_str() {
        Some(t) => t,
        None => return json.clone(),
    };

    match tag {
        "Unit" => serde_json::json!({ "Unit": {} }),
        "Literal" => {
            // Classic: ["Literal", {attrs}, literal_value]
            if arr.len() > 2 {
                let literal = convert_literal_to_v4(&arr[2]);
                serde_json::json!({ "Literal": { "value": literal } })
            } else {
                serde_json::json!({ "Literal": { "value": null } })
            }
        }
        "Constructor" => {
            // Classic: ["Constructor", {attrs}, fqname]
            let fqname = if arr.len() > 2 {
                extract_fqname_from_classic(&arr[2])
            } else {
                String::new()
            };
            serde_json::json!({ "Constructor": { "fqname": fqname } })
        }
        "Variable" => {
            // Classic: ["Variable", {attrs}, ["name", "parts"]]
            let name = if arr.len() > 2 {
                extract_name_from_json(&arr[2])
                    .map(|n| n.to_string())
                    .unwrap_or_default()
            } else {
                String::new()
            };
            // Compact format: name directly under Variable
            serde_json::json!({ "Variable": name })
        }
        "Reference" => {
            // Classic: ["Reference", {attrs}, fqname]
            // Value references use compact format: just the fqname string
            let fqname = if arr.len() > 2 {
                extract_fqname_from_classic(&arr[2])
            } else {
                String::new()
            };
            serde_json::json!({ "Reference": fqname })
        }
        "Tuple" => {
            // Classic: ["Tuple", {attrs}, [elements...]]
            let elements: Vec<serde_json::Value> = if arr.len() > 2 {
                arr[2]
                    .as_array()
                    .map(|a| a.iter().map(convert_value_expr_to_v4).collect())
                    .unwrap_or_default()
            } else {
                vec![]
            };
            serde_json::json!({ "Tuple": { "elements": elements } })
        }
        "List" => {
            // Classic: ["List", {attrs}, [items...]]
            let items: Vec<serde_json::Value> = if arr.len() > 2 {
                arr[2]
                    .as_array()
                    .map(|a| a.iter().map(convert_value_expr_to_v4).collect())
                    .unwrap_or_default()
            } else {
                vec![]
            };
            serde_json::json!({ "List": { "items": items } })
        }
        "Record" => {
            // Classic format can be either:
            // 1. Array format: ["Record", {attrs}, [[["field", "name"], value], ...]]
            // 2. Object format: ["Record", {attrs}, [{"name": ["field"], "value": ...}, ...]]
            let fields: IndexMap<String, serde_json::Value> = if arr.len() > 2 {
                arr[2]
                    .as_array()
                    .map(|fields_arr| {
                        fields_arr
                            .iter()
                            .filter_map(|field| {
                                // Try object format first: {"name": [...], "value": ...}
                                if let Some(obj) = field.as_object() {
                                    let name = obj.get("name").and_then(extract_name_from_json)?;
                                    let val = obj
                                        .get("value")
                                        .map(convert_value_expr_to_v4)
                                        .unwrap_or(serde_json::Value::Null);
                                    return Some((name.to_string(), val));
                                }
                                // Fall back to array format: [["name"], value_expr]
                                let field_arr = field.as_array()?;
                                if field_arr.len() < 2 {
                                    return None;
                                }
                                let name = extract_name_from_json(&field_arr[0])?;
                                let val = convert_value_expr_to_v4(&field_arr[1]);
                                Some((name.to_string(), val))
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            } else {
                IndexMap::new()
            };
            // Compact format: fields directly under Record
            serde_json::json!({ "Record": fields })
        }
        "Field" => {
            // Classic: ["Field", {attrs}, target_expr, ["field", "name"]]
            let target = if arr.len() > 2 {
                convert_value_expr_to_v4(&arr[2])
            } else {
                serde_json::Value::Null
            };
            let name = if arr.len() > 3 {
                extract_name_from_json(&arr[3])
                    .map(|n| n.to_string())
                    .unwrap_or_default()
            } else {
                String::new()
            };
            serde_json::json!({ "Field": { "target": target, "name": name } })
        }
        "FieldFunction" => {
            // Classic: ["FieldFunction", {attrs}, ["field", "name"]]
            let name = if arr.len() > 2 {
                extract_name_from_json(&arr[2])
                    .map(|n| n.to_string())
                    .unwrap_or_default()
            } else {
                String::new()
            };
            serde_json::json!({ "FieldFunction": { "name": name } })
        }
        "Apply" => {
            // Classic: ["Apply", {attrs}, function, argument]
            let function = if arr.len() > 2 {
                convert_value_expr_to_v4(&arr[2])
            } else {
                serde_json::Value::Null
            };
            let argument = if arr.len() > 3 {
                convert_value_expr_to_v4(&arr[3])
            } else {
                serde_json::Value::Null
            };
            serde_json::json!({ "Apply": { "function": function, "argument": argument } })
        }
        "Lambda" => {
            // Classic: ["Lambda", {attrs}, pattern, body]
            let pattern = if arr.len() > 2 {
                convert_pattern_to_v4(&arr[2])
            } else {
                serde_json::Value::Null
            };
            let body = if arr.len() > 3 {
                convert_value_expr_to_v4(&arr[3])
            } else {
                serde_json::Value::Null
            };
            serde_json::json!({ "Lambda": { "pattern": pattern, "body": body } })
        }
        "LetDefinition" => {
            // Classic: ["LetDefinition", {attrs}, ["name"], def, in_expr]
            let name = if arr.len() > 2 {
                extract_name_from_json(&arr[2])
                    .map(|n| n.to_string())
                    .unwrap_or_default()
            } else {
                String::new()
            };
            let definition = if arr.len() > 3 {
                convert_value_def_inline_to_v4(&arr[3])
            } else {
                serde_json::Value::Null
            };
            let in_expr = if arr.len() > 4 {
                convert_value_expr_to_v4(&arr[4])
            } else {
                serde_json::Value::Null
            };
            serde_json::json!({ "LetDefinition": { "name": name, "definition": definition, "in": in_expr } })
        }
        "LetRecursion" => {
            // Classic format can be either:
            // 1. Array format: ["LetRecursion", {attrs}, [[name, def], ...], in_expr]
            // 2. Object format: ["LetRecursion", {attrs}, [{"name": [...], "def": ...}, ...], in_expr]
            let definitions: IndexMap<String, serde_json::Value> = if arr.len() > 2 {
                arr[2]
                    .as_array()
                    .map(|defs| {
                        defs.iter()
                            .filter_map(|def_entry| {
                                // Try object format first: {"name": [...], "def": ...}
                                if let Some(obj) = def_entry.as_object() {
                                    let name = obj.get("name").and_then(extract_name_from_json)?;
                                    let def = obj
                                        .get("def")
                                        .map(convert_value_def_inline_to_v4)
                                        .unwrap_or(serde_json::Value::Null);
                                    return Some((name.to_string(), def));
                                }
                                // Fall back to array format: [["name"], def]
                                let entry_arr = def_entry.as_array()?;
                                if entry_arr.len() < 2 {
                                    return None;
                                }
                                let name = extract_name_from_json(&entry_arr[0])?;
                                let def = convert_value_def_inline_to_v4(&entry_arr[1]);
                                Some((name.to_string(), def))
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            } else {
                IndexMap::new()
            };
            let in_expr = if arr.len() > 3 {
                convert_value_expr_to_v4(&arr[3])
            } else {
                serde_json::Value::Null
            };
            serde_json::json!({ "LetRecursion": { "definitions": definitions, "in": in_expr } })
        }
        "Destructure" => {
            // Classic: ["Destructure", {attrs}, pattern, value_expr, in_expr]
            let pattern = if arr.len() > 2 {
                convert_pattern_to_v4(&arr[2])
            } else {
                serde_json::Value::Null
            };
            let value = if arr.len() > 3 {
                convert_value_expr_to_v4(&arr[3])
            } else {
                serde_json::Value::Null
            };
            let in_expr = if arr.len() > 4 {
                convert_value_expr_to_v4(&arr[4])
            } else {
                serde_json::Value::Null
            };
            serde_json::json!({ "Destructure": { "pattern": pattern, "value": value, "in": in_expr } })
        }
        "IfThenElse" => {
            // Classic: ["IfThenElse", {attrs}, condition, then_branch, else_branch]
            let condition = if arr.len() > 2 {
                convert_value_expr_to_v4(&arr[2])
            } else {
                serde_json::Value::Null
            };
            let then_branch = if arr.len() > 3 {
                convert_value_expr_to_v4(&arr[3])
            } else {
                serde_json::Value::Null
            };
            let else_branch = if arr.len() > 4 {
                convert_value_expr_to_v4(&arr[4])
            } else {
                serde_json::Value::Null
            };
            serde_json::json!({ "IfThenElse": { "condition": condition, "then": then_branch, "else": else_branch } })
        }
        "PatternMatch" => {
            // Classic: ["PatternMatch", {attrs}, match_expr, [[pattern, body], ...]]
            let value = if arr.len() > 2 {
                convert_value_expr_to_v4(&arr[2])
            } else {
                serde_json::Value::Null
            };
            let cases: Vec<serde_json::Value> = if arr.len() > 3 {
                arr[3]
                    .as_array()
                    .map(|cases_arr| {
                        cases_arr
                            .iter()
                            .filter_map(|case| {
                                let case_arr = case.as_array()?;
                                if case_arr.len() < 2 {
                                    return None;
                                }
                                let pattern = convert_pattern_to_v4(&case_arr[0]);
                                let body = convert_value_expr_to_v4(&case_arr[1]);
                                Some(serde_json::json!({ "pattern": pattern, "body": body }))
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            } else {
                vec![]
            };
            serde_json::json!({ "PatternMatch": { "value": value, "cases": cases } })
        }
        "UpdateRecord" => {
            // Classic format can be either:
            // 1. Array format: ["UpdateRecord", {attrs}, target, [[field_name, value], ...]]
            // 2. Object format: ["UpdateRecord", {attrs}, target, [{"name": [...], "value": ...}, ...]]
            let target = if arr.len() > 2 {
                convert_value_expr_to_v4(&arr[2])
            } else {
                serde_json::Value::Null
            };
            let fields: IndexMap<String, serde_json::Value> = if arr.len() > 3 {
                arr[3]
                    .as_array()
                    .map(|fields_arr| {
                        fields_arr
                            .iter()
                            .filter_map(|field| {
                                // Try object format first: {"name": [...], "value": ...}
                                if let Some(obj) = field.as_object() {
                                    let name = obj.get("name").and_then(extract_name_from_json)?;
                                    let val = obj
                                        .get("value")
                                        .map(convert_value_expr_to_v4)
                                        .unwrap_or(serde_json::Value::Null);
                                    return Some((name.to_string(), val));
                                }
                                // Fall back to array format: [["name"], value_expr]
                                let field_arr = field.as_array()?;
                                if field_arr.len() < 2 {
                                    return None;
                                }
                                let name = extract_name_from_json(&field_arr[0])?;
                                let val = convert_value_expr_to_v4(&field_arr[1]);
                                Some((name.to_string(), val))
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            } else {
                IndexMap::new()
            };
            serde_json::json!({ "UpdateRecord": { "target": target, "fields": fields } })
        }
        _ => {
            // Unknown value tag, return as-is
            json.clone()
        }
    }
}

/// Convert a Classic pattern to V4 format.
fn convert_pattern_to_v4(json: &serde_json::Value) -> serde_json::Value {
    let arr = match json.as_array() {
        Some(a) if !a.is_empty() => a,
        _ => return json.clone(),
    };

    let tag = match arr[0].as_str() {
        Some(t) => t,
        None => return json.clone(),
    };

    match tag {
        "WildcardPattern" => serde_json::json!({ "WildcardPattern": {} }),
        "AsPattern" => {
            // Classic: ["AsPattern", {attrs}, pattern, ["name"]]
            let pattern = if arr.len() > 2 {
                convert_pattern_to_v4(&arr[2])
            } else {
                serde_json::Value::Null
            };
            let name = if arr.len() > 3 {
                extract_name_from_json(&arr[3])
                    .map(|n| n.to_string())
                    .unwrap_or_default()
            } else {
                String::new()
            };
            serde_json::json!({ "AsPattern": { "pattern": pattern, "name": name } })
        }
        "TuplePattern" => {
            // Classic: ["TuplePattern", {attrs}, [patterns...]]
            let patterns: Vec<serde_json::Value> = if arr.len() > 2 {
                arr[2]
                    .as_array()
                    .map(|a| a.iter().map(convert_pattern_to_v4).collect())
                    .unwrap_or_default()
            } else {
                vec![]
            };
            serde_json::json!({ "TuplePattern": { "patterns": patterns } })
        }
        "ConstructorPattern" => {
            // Classic: ["ConstructorPattern", {attrs}, fqname, [arg_patterns...]]
            let fqname = if arr.len() > 2 {
                extract_fqname_from_classic(&arr[2])
            } else {
                String::new()
            };
            let args: Vec<serde_json::Value> = if arr.len() > 3 {
                arr[3]
                    .as_array()
                    .map(|a| a.iter().map(convert_pattern_to_v4).collect())
                    .unwrap_or_default()
            } else {
                vec![]
            };
            serde_json::json!({ "ConstructorPattern": { "fqname": fqname, "args": args } })
        }
        "EmptyListPattern" => serde_json::json!({ "EmptyListPattern": {} }),
        "HeadTailPattern" => {
            // Classic: ["HeadTailPattern", {attrs}, head_pattern, tail_pattern]
            let head = if arr.len() > 2 {
                convert_pattern_to_v4(&arr[2])
            } else {
                serde_json::Value::Null
            };
            let tail = if arr.len() > 3 {
                convert_pattern_to_v4(&arr[3])
            } else {
                serde_json::Value::Null
            };
            serde_json::json!({ "HeadTailPattern": { "head": head, "tail": tail } })
        }
        "LiteralPattern" => {
            // Classic: ["LiteralPattern", {attrs}, literal_value]
            if arr.len() > 2 {
                let literal = convert_literal_to_v4(&arr[2]);
                serde_json::json!({ "LiteralPattern": { "value": literal } })
            } else {
                serde_json::json!({ "LiteralPattern": { "value": null } })
            }
        }
        "UnitPattern" => serde_json::json!({ "UnitPattern": {} }),
        _ => json.clone(),
    }
}

/// Convert a Classic literal to V4 format.
fn convert_literal_to_v4(json: &serde_json::Value) -> serde_json::Value {
    let arr = match json.as_array() {
        Some(a) if !a.is_empty() => a,
        _ => return json.clone(),
    };

    let tag = match arr[0].as_str() {
        Some(t) => t,
        None => return json.clone(),
    };

    match tag {
        "BoolLiteral" => {
            if arr.len() > 1 {
                serde_json::json!({ "BoolLiteral": arr[1] })
            } else {
                serde_json::json!({ "BoolLiteral": false })
            }
        }
        "CharLiteral" => {
            if arr.len() > 1 {
                serde_json::json!({ "CharLiteral": arr[1] })
            } else {
                serde_json::json!({ "CharLiteral": "" })
            }
        }
        "StringLiteral" => {
            if arr.len() > 1 {
                serde_json::json!({ "StringLiteral": arr[1] })
            } else {
                serde_json::json!({ "StringLiteral": "" })
            }
        }
        "WholeNumberLiteral" => {
            if arr.len() > 1 {
                serde_json::json!({ "WholeNumberLiteral": arr[1] })
            } else {
                serde_json::json!({ "WholeNumberLiteral": 0 })
            }
        }
        "FloatLiteral" => {
            if arr.len() > 1 {
                serde_json::json!({ "FloatLiteral": arr[1] })
            } else {
                serde_json::json!({ "FloatLiteral": 0.0 })
            }
        }
        "DecimalLiteral" => {
            if arr.len() > 1 {
                serde_json::json!({ "DecimalLiteral": arr[1] })
            } else {
                serde_json::json!({ "DecimalLiteral": "0" })
            }
        }
        _ => json.clone(),
    }
}

/// Convert a Classic inline value definition to V4 format.
fn convert_value_def_inline_to_v4(json: &serde_json::Value) -> serde_json::Value {
    // Classic format: {inputTypes: [...], outputType: ..., body: ...}
    if let Some(obj) = json.as_object() {
        let input_types: IndexMap<String, serde_json::Value> = obj
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
                        let typ = convert_type_expr_to_v4(&input_arr[2]);
                        Some((name.to_string(), typ))
                    })
                    .collect()
            })
            .unwrap_or_default();

        let output_type = obj
            .get("outputType")
            .map(convert_type_expr_to_v4)
            .unwrap_or(serde_json::Value::Null);

        let body = obj
            .get("body")
            .map(convert_value_expr_to_v4)
            .unwrap_or(serde_json::Value::Null);

        serde_json::json!({
            "inputTypes": input_types,
            "outputType": output_type,
            "body": body
        })
    } else {
        json.clone()
    }
}

/// Convert a Classic package to V4 format using compact (default) format.
///
/// # Arguments
/// * `pkg` - The Classic package to convert
///
/// # Returns
/// A V4 PackageDefinition with converted modules
pub fn classic_to_v4(pkg: classic::Package) -> v4::PackageDefinition {
    classic_to_v4_with_options(pkg, true)
}

/// Convert a Classic package to V4 format with configurable output format.
///
/// # Arguments
/// * `pkg` - The Classic package to convert
/// * `compact` - If true, use compact format; if false, use expanded format
///
/// # Returns
/// A V4 PackageDefinition with converted modules
pub fn classic_to_v4_with_options(pkg: classic::Package, compact: bool) -> v4::PackageDefinition {
    set_compact_format(compact);
    let modules: IndexMap<String, v4::AccessControlledModuleDefinition> =
        pkg.modules.into_iter().map(convert_module_to_v4).collect();
    // Reset to default (compact) after conversion
    set_compact_format(true);

    v4::PackageDefinition { modules }
}

/// Convert a V4 package to Classic format.
///
/// # Arguments
/// * `pkg` - The V4 package to convert
///
/// # Returns
/// A Classic Package with converted modules
pub fn v4_to_classic(pkg: v4::PackageDefinition) -> classic::Package {
    let modules = pkg
        .modules
        .into_iter()
        .map(|(name, module)| convert_module_to_classic(name, module))
        .collect();

    classic::Package {
        name: String::new(), // Name is stored at distribution level
        modules,
    }
}

/// Convert a Classic module to V4 format (returns tuple for IndexMap collection)
fn convert_module_to_v4(module: classic::Module) -> (String, v4::AccessControlledModuleDefinition) {
    let access = convert_access_to_v4(&module.detail.access);
    let types = convert_types_to_v4(&module.detail.value.types);
    let values = convert_values_to_v4(&module.detail.value.values);

    (
        module.name.to_string(),
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

/// Convert a V4 module to Classic Module
fn convert_module_to_classic(
    name: String,
    access_controlled: v4::AccessControlledModuleDefinition,
) -> classic::Module {
    classic::Module {
        name: crate::naming::Path::new(&name),
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

/// Convert Classic types array to V4 type definitions.
///
/// Classic format: `[[[name_parts], {access, value}], ...]`
/// V4 format: `IndexMap<String, AccessControlledTypeDefinition>`
fn convert_types_to_v4(
    types: &[serde_json::Value],
) -> IndexMap<String, v4::AccessControlledTypeDefinition> {
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

            Some((
                name.to_string(),
                v4::AccessControlledTypeDefinition {
                    access: convert_access_to_v4(access_str),
                    value: convert_type_definition_to_v4(&value),
                },
            ))
        })
        .collect()
}

/// Convert V4 type definitions to Classic types array
fn convert_types_to_classic(
    types: &IndexMap<String, v4::AccessControlledTypeDefinition>,
) -> Vec<serde_json::Value> {
    types
        .iter()
        .map(|(name, access_controlled)| {
            let name_obj = Name::from(name.as_str());
            let name_json = serde_json::to_value(&name_obj).unwrap_or(serde_json::Value::Null);
            let def_json = serde_json::json!({
                "access": convert_access_to_classic(&access_controlled.access),
                "value": convert_type_definition_to_classic(&access_controlled.value)
            });

            serde_json::json!([name_json, def_json])
        })
        .collect()
}

/// Convert Classic values array to V4 value definitions.
///
/// Classic format: `[[[name_parts], {access, value: {doc, value: {inputTypes, outputType, body}}}], ...]`
/// V4 format: `IndexMap<String, AccessControlledValueDefinition>`
fn convert_values_to_v4(
    values: &[serde_json::Value],
) -> IndexMap<String, v4::AccessControlledValueDefinition> {
    values
        .iter()
        .filter_map(|value_val| {
            // Classic format: [[name_parts], {access, value: {doc, value: {...}}}]
            let arr = value_val.as_array()?;
            if arr.len() < 2 {
                return None;
            }

            // Extract name from first element
            let name = extract_name_from_json(&arr[0])?;

            // Extract access-controlled definition from second element
            let def_obj = arr[1].as_object()?;
            let access_str = def_obj.get("access")?.as_str()?;

            // The value contains {doc, value: {inputTypes, outputType, body}}
            // We need to extract the inner 'value' field
            let outer_value = def_obj.get("value")?.as_object()?;
            let inner_value = outer_value.get("value")?.clone();

            Some((
                name.to_string(),
                v4::AccessControlledValueDefinition {
                    access: convert_access_to_v4(access_str),
                    value: convert_value_definition_to_v4(&inner_value),
                },
            ))
        })
        .collect()
}

/// Convert V4 value definitions to Classic values array
fn convert_values_to_classic(
    values: &IndexMap<String, v4::AccessControlledValueDefinition>,
) -> Vec<serde_json::Value> {
    values
        .iter()
        .map(|(name, access_controlled)| {
            let name_obj = Name::from(name.as_str());
            let name_json = serde_json::to_value(&name_obj).unwrap_or(serde_json::Value::Null);
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
    if let Some(obj) = value.as_object()
        && let Some(inner_value) = obj.get("value")
        && let Some(arr) = inner_value.as_array()
        && !arr.is_empty()
    {
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

    // Fallback: create empty type alias
    v4::TypeDefinition::TypeAliasDefinition {
        type_params: vec![],
        type_expr: serde_json::Value::Null,
    }
}

/// Convert V4 TypeDefinition to Classic JSON value
fn convert_type_definition_to_classic(type_def: &v4::TypeDefinition) -> serde_json::Value {
    match type_def {
        v4::TypeDefinition::TypeAliasDefinition {
            type_params,
            type_expr,
        } => {
            let type_params_json: Vec<serde_json::Value> = type_params
                .iter()
                .map(|n| serde_json::to_value(n).unwrap_or(serde_json::Value::Null))
                .collect();

            serde_json::json!({
                "value": ["TypeAliasDefinition", type_params_json, type_expr]
            })
        }
        v4::TypeDefinition::CustomTypeDefinition {
            type_params,
            constructors,
        } => {
            let type_params_json: Vec<serde_json::Value> = type_params
                .iter()
                .map(|n| serde_json::to_value(n).unwrap_or(serde_json::Value::Null))
                .collect();

            let constructors_json = convert_constructors_to_classic(constructors);

            serde_json::json!({
                "value": ["CustomTypeDefinition", type_params_json, constructors_json]
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

    v4::TypeDefinition::CustomTypeDefinition {
        type_params,
        constructors,
    }
}

/// Convert Classic TypeAliasDefinition array to V4
fn convert_type_alias_to_v4(arr: &[serde_json::Value]) -> v4::TypeDefinition {
    // Format: ["TypeAliasDefinition", [type_params], type_exp]
    let type_params = if arr.len() > 1 {
        extract_type_params(&arr[1])
    } else {
        vec![]
    };

    let type_expr = if arr.len() > 2 {
        convert_type_expr_to_v4(&arr[2])
    } else {
        serde_json::Value::Null
    };

    v4::TypeDefinition::TypeAliasDefinition {
        type_params,
        type_expr,
    }
}

/// Extract type parameters from JSON array as Name (kebab-case string serialization)
fn extract_type_params(json: &serde_json::Value) -> Vec<Name> {
    json.as_array()
        .map(|arr| arr.iter().filter_map(extract_name_from_json).collect())
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
            .map(|arr| arr.iter().filter_map(convert_constructor_to_v4).collect())
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
fn convert_constructors_to_classic(
    constructors: &v4::AccessControlledConstructors,
) -> serde_json::Value {
    let ctors: Vec<serde_json::Value> = constructors
        .value
        .iter()
        .map(|ctor| {
            let name_json = serde_json::to_value(&ctor.name).unwrap_or(serde_json::Value::Null);
            let args: Vec<serde_json::Value> = ctor
                .args
                .iter()
                .map(|arg| {
                    serde_json::json!([
                        serde_json::to_value(&arg.name).unwrap_or(serde_json::Value::Null),
                        arg.arg_type
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

    let args: Vec<v4::ConstructorArg> = arr[1]
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
                    let arg_type = convert_type_expr_to_v4(&arg_arr[1]);
                    Some(v4::ConstructorArg {
                        name: arg_name,
                        arg_type,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    Some(v4::ConstructorDefinition { name, args })
}

/// Convert a Classic value definition to V4 ValueDefinition.
fn convert_value_definition_to_v4(value: &serde_json::Value) -> v4::ValueDefinition {
    // Classic format: {inputTypes: [[name, annotation_type, actual_type], ...], outputType: ..., body: ...}
    // The annotation_type is the type annotation from source, actual_type is the inferred type
    if let Some(obj) = value.as_object() {
        let input_types: IndexMap<String, v4::InputTypeEntry> = obj
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
                        // Element 1 is the type annotation - convert it to V4 format
                        let annotation = convert_type_expr_to_v4(&input_arr[1]);
                        // Element 2 is the actual/inferred input type
                        let typ = convert_type_expr_to_v4(&input_arr[2]);
                        Some((
                            name.to_string(),
                            v4::InputTypeEntry {
                                // Store the converted type annotation if it's not empty
                                type_attributes: if annotation.is_null()
                                    || (annotation.is_object()
                                        && annotation.as_object().is_none_or(|o| o.is_empty()))
                                {
                                    None
                                } else {
                                    Some(annotation)
                                },
                                input_type: typ,
                            },
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default();

        let output_type = obj
            .get("outputType")
            .map(convert_type_expr_to_v4)
            .unwrap_or(serde_json::Value::Null);

        let body_json = obj
            .get("body")
            .map(convert_value_expr_to_v4)
            .unwrap_or(serde_json::Value::Null);
        let body = v4::ValueBody::ExpressionBody { body: body_json };

        v4::ValueDefinition {
            input_types,
            output_type,
            body,
        }
    } else {
        v4::ValueDefinition {
            input_types: IndexMap::new(),
            output_type: serde_json::Value::Null,
            body: v4::ValueBody::ExpressionBody {
                body: serde_json::Value::Null,
            },
        }
    }
}

/// Convert V4 ValueDefinition to Classic JSON value
fn convert_value_definition_to_classic(value_def: &v4::ValueDefinition) -> serde_json::Value {
    let input_types: Vec<serde_json::Value> = value_def
        .input_types
        .iter()
        .map(|(name, entry)| {
            let name_obj = Name::from(name.as_str());
            let attrs = entry
                .type_attributes
                .clone()
                .unwrap_or(serde_json::json!({}));
            serde_json::json!([
                serde_json::to_value(&name_obj).unwrap_or(serde_json::Value::Null),
                attrs,
                entry.input_type
            ])
        })
        .collect();

    // Extract body from ValueBody wrapper
    let body = match &value_def.body {
        v4::ValueBody::ExpressionBody { body } => body.clone(),
        v4::ValueBody::NativeBody { .. } => serde_json::Value::Null,
        v4::ValueBody::ExternalBody { .. } => serde_json::Value::Null,
        v4::ValueBody::IncompleteBody { .. } => serde_json::Value::Null,
    };

    serde_json::json!({
        "inputTypes": input_types,
        "outputType": value_def.output_type,
        "body": body
    })
}

// =============================================================================
// Visitor-Based Converters (Phase 5)
// =============================================================================
//
// The converters above work at the JSON level for backward compatibility.
// These visitor-based converters work with strongly-typed IR structures.

use crate::ir::attributes::{ClassicAttrs, TypeAttributes, ValueAttributes};
use crate::ir::type_expr::Type;
use crate::ir::value_expr::{HoleReason, NativeInfo, Value};
use crate::naming::FQName;
use crate::traversal::transform::{TypeTransformVisitor, ValueTransformVisitor};
use std::fmt;

/// Error type for visitor-based conversion failures.
#[derive(Debug, Clone)]
pub enum ConversionError {
    /// V4-only construct cannot be downgraded to Classic format
    CannotDowngrade(&'static str),
    /// Generic conversion error with message
    Message(String),
}

impl fmt::Display for ConversionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConversionError::CannotDowngrade(name) => {
                write!(
                    f,
                    "Cannot downgrade V4-only construct '{}' to Classic format",
                    name
                )
            }
            ConversionError::Message(msg) => write!(f, "{}", msg),
        }
    }
}

impl std::error::Error for ConversionError {}

/// Visitor-based converter from Classic (serde_json::Value attrs) to V4 (TypeAttributes).
///
/// This converter transforms IR structures from Classic format (V1-V3) to V4 format.
/// Classic uses empty `{}` for attributes, while V4 uses explicit TypeAttributes/ValueAttributes.
///
/// # Example
/// ```ignore
/// use morphir_ir::converter::ClassicToV4Converter;
/// use morphir_ir::traversal::transform::TypeTransformVisitor;
///
/// let converter = ClassicToV4Converter;
/// let v4_type = converter.transform_type(&classic_type)?;
/// ```
pub struct ClassicToV4Converter;

impl TypeTransformVisitor<ClassicAttrs, TypeAttributes> for ClassicToV4Converter {
    type Error = ConversionError;

    fn transform_type_attrs(&self, attrs: &ClassicAttrs) -> Result<TypeAttributes, Self::Error> {
        // Classic uses {} - convert to V4 TypeAttributes with empty fields
        // Preserve any existing data in extensions
        Ok(TypeAttributes {
            source: None,
            constraints: serde_json::Value::Null,
            extensions: attrs.clone(),
        })
    }
}

impl ValueTransformVisitor<ClassicAttrs, TypeAttributes, ClassicAttrs, ValueAttributes>
    for ClassicToV4Converter
{
    type Error = ConversionError;

    fn transform_type_attrs(&self, attrs: &ClassicAttrs) -> Result<TypeAttributes, Self::Error> {
        <Self as TypeTransformVisitor<ClassicAttrs, TypeAttributes>>::transform_type_attrs(
            self, attrs,
        )
    }

    fn transform_value_attrs(&self, attrs: &ClassicAttrs) -> Result<ValueAttributes, Self::Error> {
        // Classic uses {} - convert to V4 ValueAttributes with empty fields
        Ok(ValueAttributes {
            source: None,
            inferred_type: serde_json::Value::Null,
            extensions: attrs.clone(),
        })
    }

    // Classic never has Hole/Native/External, so default implementations work.
    // If we encounter them, it means the input is already V4 (error case).
    fn transform_hole(
        &self,
        _attrs: &ClassicAttrs,
        _reason: &HoleReason,
        _expected_type: &Option<Box<Type<ClassicAttrs>>>,
    ) -> Result<Value<TypeAttributes, ValueAttributes>, Self::Error> {
        Err(ConversionError::Message(
            "Unexpected Hole in Classic IR".to_string(),
        ))
    }

    fn transform_native(
        &self,
        _attrs: &ClassicAttrs,
        _fqname: &FQName,
        _info: &NativeInfo,
    ) -> Result<Value<TypeAttributes, ValueAttributes>, Self::Error> {
        Err(ConversionError::Message(
            "Unexpected Native in Classic IR".to_string(),
        ))
    }

    fn transform_external(
        &self,
        _attrs: &ClassicAttrs,
        _external_name: &str,
        _target_platform: &str,
    ) -> Result<Value<TypeAttributes, ValueAttributes>, Self::Error> {
        Err(ConversionError::Message(
            "Unexpected External in Classic IR".to_string(),
        ))
    }
}

/// Visitor-based converter from V4 (TypeAttributes) to Classic (serde_json::Value).
///
/// This converter transforms IR structures from V4 format to Classic format (V1-V3).
/// V4 structures are richer, so some information is lost (source locations, constraints).
///
/// # Note
/// V4-only constructs (Hole, Native, External) cannot be downgraded and will return errors.
///
/// # Example
/// ```ignore
/// use morphir_ir::converter::V4ToClassicConverter;
/// use morphir_ir::traversal::transform::TypeTransformVisitor;
///
/// let converter = V4ToClassicConverter;
/// let classic_type = converter.transform_type(&v4_type)?;
/// ```
pub struct V4ToClassicConverter;

impl TypeTransformVisitor<TypeAttributes, ClassicAttrs> for V4ToClassicConverter {
    type Error = ConversionError;

    fn transform_type_attrs(&self, attrs: &TypeAttributes) -> Result<ClassicAttrs, Self::Error> {
        // V4 -> Classic loses source, constraints, inferred types
        // Preserve extensions if any (otherwise empty object)
        if attrs.extensions.is_null() {
            Ok(serde_json::json!({}))
        } else {
            Ok(attrs.extensions.clone())
        }
    }
}

impl ValueTransformVisitor<TypeAttributes, ClassicAttrs, ValueAttributes, ClassicAttrs>
    for V4ToClassicConverter
{
    type Error = ConversionError;

    fn transform_type_attrs(&self, attrs: &TypeAttributes) -> Result<ClassicAttrs, Self::Error> {
        <Self as TypeTransformVisitor<TypeAttributes, ClassicAttrs>>::transform_type_attrs(
            self, attrs,
        )
    }

    fn transform_value_attrs(&self, attrs: &ValueAttributes) -> Result<ClassicAttrs, Self::Error> {
        // V4 -> Classic loses source, inferred_type
        // Preserve extensions if any (otherwise empty object)
        if attrs.extensions.is_null() {
            Ok(serde_json::json!({}))
        } else {
            Ok(attrs.extensions.clone())
        }
    }

    // V4-only variants cannot be downgraded
    fn transform_hole(
        &self,
        _attrs: &ValueAttributes,
        _reason: &HoleReason,
        _expected_type: &Option<Box<Type<TypeAttributes>>>,
    ) -> Result<Value<ClassicAttrs, ClassicAttrs>, Self::Error> {
        Err(ConversionError::CannotDowngrade("Hole"))
    }

    fn transform_native(
        &self,
        _attrs: &ValueAttributes,
        _fqname: &FQName,
        _info: &NativeInfo,
    ) -> Result<Value<ClassicAttrs, ClassicAttrs>, Self::Error> {
        Err(ConversionError::CannotDowngrade("Native"))
    }

    fn transform_external(
        &self,
        _attrs: &ValueAttributes,
        _external_name: &str,
        _target_platform: &str,
    ) -> Result<Value<ClassicAttrs, ClassicAttrs>, Self::Error> {
        Err(ConversionError::CannotDowngrade("External"))
    }
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

        let (name, v4_module) = convert_module_to_v4(classic_module.clone());
        let roundtrip = convert_module_to_classic(name, v4_module);

        assert_eq!(roundtrip.name.to_string(), classic_module.name.to_string());
        assert_eq!(roundtrip.detail.access, classic_module.detail.access);
        assert_eq!(roundtrip.detail.value.doc, classic_module.detail.value.doc);
    }

    // ==========================================================================
    // Visitor-based converter tests
    // ==========================================================================

    use crate::ir::attributes::{SourceLocation, TypeAttributes, ValueAttributes};
    use crate::ir::type_expr::Type;
    use crate::ir::value_expr::Value;
    use crate::traversal::transform::TypeTransformVisitor;

    #[test]
    fn test_classic_to_v4_type_attrs() {
        let converter = ClassicToV4Converter;
        let classic_attrs = serde_json::json!({});

        let v4_attrs =
            TypeTransformVisitor::transform_type_attrs(&converter, &classic_attrs).unwrap();

        assert!(v4_attrs.source.is_none());
        assert!(v4_attrs.constraints.is_null());
        assert_eq!(v4_attrs.extensions, serde_json::json!({}));
    }

    #[test]
    fn test_classic_to_v4_preserves_extensions() {
        let converter = ClassicToV4Converter;
        let classic_attrs = serde_json::json!({"custom": "data"});

        let v4_attrs =
            TypeTransformVisitor::transform_type_attrs(&converter, &classic_attrs).unwrap();

        assert_eq!(v4_attrs.extensions, serde_json::json!({"custom": "data"}));
    }

    #[test]
    fn test_classic_to_v4_type_variable() {
        let converter = ClassicToV4Converter;
        let classic_type: Type<serde_json::Value> =
            Type::Variable(serde_json::json!({}), Name::from("a"));

        let v4_type = TypeTransformVisitor::transform_type(&converter, &classic_type).unwrap();

        if let Type::Variable(attrs, name) = v4_type {
            assert!(attrs.source.is_none());
            assert_eq!(name.to_string(), "a");
        } else {
            panic!("Expected Variable type");
        }
    }

    #[test]
    fn test_v4_to_classic_type_attrs() {
        let converter = V4ToClassicConverter;
        let v4_attrs = TypeAttributes {
            source: Some(SourceLocation {
                start_line: 1,
                start_column: 0,
                end_line: 1,
                end_column: 10,
            }),
            constraints: serde_json::json!({"kind": "type"}),
            extensions: serde_json::json!({}),
        };

        let classic_attrs =
            TypeTransformVisitor::transform_type_attrs(&converter, &v4_attrs).unwrap();

        // Source and constraints are lost, empty object returned
        assert_eq!(classic_attrs, serde_json::json!({}));
    }

    #[test]
    fn test_v4_to_classic_preserves_extensions() {
        let converter = V4ToClassicConverter;
        let v4_attrs = TypeAttributes {
            source: None,
            constraints: serde_json::Value::Null,
            extensions: serde_json::json!({"custom": "data"}),
        };

        let classic_attrs =
            TypeTransformVisitor::transform_type_attrs(&converter, &v4_attrs).unwrap();

        assert_eq!(classic_attrs, serde_json::json!({"custom": "data"}));
    }

    #[test]
    fn test_v4_to_classic_type_unit() {
        let converter = V4ToClassicConverter;
        let v4_type: Type<TypeAttributes> = Type::Unit(TypeAttributes::default());

        let classic_type = TypeTransformVisitor::transform_type(&converter, &v4_type).unwrap();

        if let Type::Unit(attrs) = classic_type {
            assert_eq!(attrs, serde_json::json!({}));
        } else {
            panic!("Expected Unit type");
        }
    }

    #[test]
    fn test_v4_to_classic_value_unit() {
        use crate::traversal::transform::ValueTransformVisitor;

        let converter = V4ToClassicConverter;
        let v4_value: Value<TypeAttributes, ValueAttributes> =
            Value::Unit(ValueAttributes::default());

        let classic_value = ValueTransformVisitor::transform_value(&converter, &v4_value).unwrap();

        if let Value::Unit(attrs) = classic_value {
            assert_eq!(attrs, serde_json::json!({}));
        } else {
            panic!("Expected Unit value");
        }
    }

    #[test]
    fn test_v4_to_classic_hole_error() {
        use crate::ir::value_expr::HoleReason;
        use crate::traversal::transform::ValueTransformVisitor;

        let converter = V4ToClassicConverter;
        let v4_value: Value<TypeAttributes, ValueAttributes> =
            Value::Hole(ValueAttributes::default(), HoleReason::Draft, None);

        let result = ValueTransformVisitor::transform_value(&converter, &v4_value);

        assert!(result.is_err());
        if let Err(ConversionError::CannotDowngrade(name)) = result {
            assert_eq!(name, "Hole");
        } else {
            panic!("Expected CannotDowngrade error");
        }
    }

    #[test]
    fn test_conversion_error_display() {
        let err = ConversionError::CannotDowngrade("Hole");
        assert_eq!(
            err.to_string(),
            "Cannot downgrade V4-only construct 'Hole' to Classic format"
        );

        let err = ConversionError::Message("Test error".to_string());
        assert_eq!(err.to_string(), "Test error");
    }

    #[test]
    fn test_convert_type_expr_variable_compact_format() {
        // Classic Variable: ["Variable", {attrs}, ["name"]]
        let classic_var = serde_json::json!(["Variable", {}, ["a"]]);

        // Ensure compact mode is on (default)
        set_compact_format(true);
        let v4_json = convert_type_expr_to_v4(&classic_var);

        // Compact: bare name string
        assert_eq!(v4_json, serde_json::json!("a"));
    }

    #[test]
    fn test_convert_type_expr_variable_expanded_format() {
        // Classic Variable: ["Variable", {attrs}, ["name"]]
        let classic_var = serde_json::json!(["Variable", {}, ["a"]]);

        // Set expanded mode
        set_compact_format(false);
        let v4_json = convert_type_expr_to_v4(&classic_var);

        // Expanded: {"Variable": {"name": name}}
        assert_eq!(v4_json, serde_json::json!({"Variable": {"name": "a"}}));

        // Reset to compact mode for other tests
        set_compact_format(true);
    }

    #[test]
    fn test_convert_type_expr_reference_compact_format() {
        // Classic Reference without args: ["Reference", {attrs}, [pkg_path, mod_path, local_name], []]
        // Package path: [["morphir"]] -> "morphir"
        // Module path: [["sdk"]] -> "sdk"
        // Local name: ["int"] -> "int"
        let classic_ref =
            serde_json::json!(["Reference", {}, [[["morphir"]], [["sdk"]], ["int"]], []]);

        set_compact_format(true);
        let v4_json = convert_type_expr_to_v4(&classic_ref);

        // Compact without args: bare fqname string
        assert_eq!(v4_json, serde_json::json!("morphir:sdk#int"));
    }

    #[test]
    fn test_convert_type_expr_reference_with_args_compact_format() {
        // Classic Reference with args: ["Reference", {}, [pkg_path, mod_path, local_name], [type_args]]
        let classic_ref = serde_json::json!([
            "Reference",
            {},
            [[["morphir"]], [["sdk"]], ["list"]],
            [["Variable", {}, ["a"]]]
        ]);

        set_compact_format(true);
        let v4_json = convert_type_expr_to_v4(&classic_ref);

        // Compact with args: {"Reference": [fqname, arg1, ...]}
        assert_eq!(
            v4_json,
            serde_json::json!({"Reference": ["morphir:sdk#list", "a"]})
        );
    }

    #[test]
    fn test_convert_type_expr_reference_expanded_format() {
        // Classic Reference with args
        let classic_ref = serde_json::json!([
            "Reference",
            {},
            [[["morphir"]], [["sdk"]], ["list"]],
            [["Variable", {}, ["a"]]]
        ]);

        set_compact_format(false);
        let v4_json = convert_type_expr_to_v4(&classic_ref);

        // Expanded: {"Reference": {"fqname": ..., "args": [...]}}
        assert_eq!(
            v4_json,
            serde_json::json!({
                "Reference": {
                    "fqname": "morphir:sdk#list",
                    "args": [{"Variable": {"name": "a"}}]
                }
            })
        );

        // Reset to compact mode
        set_compact_format(true);
    }
}
