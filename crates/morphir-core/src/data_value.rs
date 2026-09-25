//! Pure validation of closed JSON data against Morphir type declarations.
//!
//! The validator checks one value after metadata expansion has decided that it
//! is data. A node reference is a distinct object term, and a bare JSON `null`
//! that produced no fact never reaches this module.

use crate::ir::{classic, v4};
use crate::metadata::ObjectTerm;
use crate::naming::FQName;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

mod declarations;
use declarations::{
    classic_path, collect_v3_definitions, collect_v3_specifications, collect_v4_definitions,
    collect_v4_specifications, v3_shape, v4_shape,
};

/// The reason a closed data value was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataValueErrorKind {
    /// The declared shape is supported, but the value does not match it.
    Mismatch,
    /// The tag is absent from a resolved custom type.
    UnknownConstructor,
    /// The declared type has no supported closed JSON representation.
    UnsupportedType,
}

/// A data validation failure at a path within one JSON value.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message} at {path}")]
pub struct DataValueError {
    /// Failure category for callers that report contract diagnostics.
    pub kind: DataValueErrorKind,
    /// JSON path rooted at `$`.
    pub path: String,
    /// Human-readable detail.
    pub message: String,
}

fn error(kind: DataValueErrorKind, path: &str, message: impl Into<String>) -> DataValueError {
    DataValueError {
        kind,
        path: path.to_owned(),
        message: message.into(),
    }
}

fn mismatch(path: &str, message: impl Into<String>) -> DataValueError {
    error(DataValueErrorKind::Mismatch, path, message)
}

fn unsupported(path: &str, message: impl Into<String>) -> DataValueError {
    error(DataValueErrorKind::UnsupportedType, path, message)
}

#[derive(Clone, PartialEq, Eq, Hash)]
enum Shape {
    Unit,
    Variable(String),
    Reference(String, Vec<Self>),
    Record(Vec<(String, Self)>),
    Tuple(Vec<Self>),
    Unsupported(&'static str),
}

#[derive(Clone)]
enum Definition {
    Alias {
        params: Vec<String>,
        body: Shape,
    },
    Custom {
        params: Vec<String>,
        constructors: HashMap<String, Vec<Shape>>,
    },
    Unsupported(&'static str),
}

/// A declaration closure used to validate Morphir data without I/O.
///
/// Build this from the supplied V3 or V4 distribution, then validate the
/// predicate's declared type. Semantic interpreters, including target-name
/// language-ID policy, run separately after this shared shape check.
///
/// # Example
///
/// ```
/// use indexmap::IndexMap;
/// use morphir_core::data_value::DataValueValidator;
/// use morphir_core::ir::v4;
/// use morphir_core::naming::{FQName, PackageName, Path};
///
/// let distribution = v4::Distribution::Specs(v4::SpecsContent {
///     package_name: PackageName::new(Path::new("example")),
///     dependencies: IndexMap::new(),
///     spec: v4::PackageSpecification { modules: IndexMap::new() },
/// });
/// let string = v4::Type::reference(
///     v4::TypeAttributes::default(),
///     FQName::from_canonical_string("morphir/SDK:string#string")?,
///     vec![],
/// );
/// let validator = DataValueValidator::v4(&distribution)?;
/// assert!(validator.validate_v4_data(&string, &serde_json::json!("order")).is_ok());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub struct DataValueValidator {
    definitions: HashMap<String, Definition>,
}

impl DataValueValidator {
    /// Collect all type definitions and specifications visible to a V4 distribution.
    pub fn v4(distribution: &v4::Distribution) -> Result<Self, DataValueError> {
        let mut definitions = HashMap::new();
        match distribution {
            v4::Distribution::Library(library) => {
                collect_v4_definitions(
                    &mut definitions,
                    &library.package_name.to_canonical_string(),
                    &library.def,
                )?;
                for (package, spec) in &library.dependencies {
                    collect_v4_specifications(&mut definitions, package, spec)?;
                }
            }
            v4::Distribution::Specs(specs) => {
                collect_v4_specifications(
                    &mut definitions,
                    &specs.package_name.to_canonical_string(),
                    &specs.spec,
                )?;
                for (package, spec) in &specs.dependencies {
                    collect_v4_specifications(&mut definitions, package, spec)?;
                }
            }
            v4::Distribution::Application(application) => {
                collect_v4_definitions(
                    &mut definitions,
                    &application.package_name.to_canonical_string(),
                    &application.def,
                )?;
                for (package, def) in &application.dependencies {
                    collect_v4_definitions(&mut definitions, package, def)?;
                }
            }
        }
        Ok(Self { definitions })
    }

    /// Collect all type definitions and specifications visible to a V3 distribution.
    pub fn v3(distribution: &classic::Distribution) -> Result<Self, DataValueError> {
        if distribution.format_version != 3 {
            return Err(unsupported("$", "data type IR must be V3"));
        }
        let mut definitions = HashMap::new();
        let dependencies = match &distribution.distribution {
            classic::DistributionBody::Library(package, dependencies, definition) => {
                collect_v3_definitions(&mut definitions, &classic_path(package), definition)?;
                dependencies
            }
            classic::DistributionBody::Specs(package, dependencies, specification) => {
                collect_v3_specifications(&mut definitions, &classic_path(package), specification)?;
                dependencies
            }
        };
        for (package, specification) in dependencies {
            collect_v3_specifications(&mut definitions, &classic_path(package), specification)?;
        }
        Ok(Self { definitions })
    }

    /// Validate one V4 data value against a declared Morphir type.
    pub fn validate_v4_data(&self, ty: &v4::Type, value: &Value) -> Result<(), DataValueError> {
        self.validate(&v4_shape(ty), value, &HashMap::new(), "$", 0)
    }

    /// Validate one V3 data value against a declared Morphir type.
    pub fn validate_v3_data(
        &self,
        ty: &classic::Type<classic::Attrs>,
        value: &Value,
    ) -> Result<(), DataValueError> {
        self.validate(&v3_shape(ty), value, &HashMap::new(), "$", 0)
    }

    /// Validate a V4 metadata object only when it is a data term.
    pub fn validate_v4_object(
        &self,
        ty: &v4::Type,
        object: &ObjectTerm,
    ) -> Result<(), DataValueError> {
        match object {
            ObjectTerm::Value(value) => self.validate_v4_data(ty, value.value()),
            ObjectTerm::NodeRef(_) => Err(mismatch("$", "expected data, found node reference")),
        }
    }

    /// Validate a named type entry point, as used by decorator sidecars.
    pub fn validate_reference(&self, name: &FQName, value: &Value) -> Result<(), DataValueError> {
        self.validate(
            &Shape::Reference(name.to_canonical_string(), vec![]),
            value,
            &HashMap::new(),
            "$",
            0,
        )
    }

    fn validate(
        &self,
        shape: &Shape,
        value: &Value,
        vars: &HashMap<String, Shape>,
        path: &str,
        depth: usize,
    ) -> Result<(), DataValueError> {
        if depth > 128 {
            return Err(unsupported(path, "data type/value nesting exceeds 128"));
        }
        let next = depth + 1;
        match shape {
            Shape::Unit if value.is_null() => Ok(()),
            Shape::Unit => Err(mismatch(path, "expected unit (null)")),
            Shape::Variable(name) => {
                let bound = vars
                    .get(name)
                    .ok_or_else(|| unsupported(path, format!("unbound type variable {name}")))?;
                self.validate(bound, value, vars, path, next)
            }
            Shape::Reference(name, args) => {
                if let Some((module, local)) = sdk_type(name) {
                    return self.validate_sdk((module, local), args, value, vars, path, next);
                }
                let definition = self.definitions.get(name).ok_or_else(|| {
                    unsupported(
                        path,
                        format!("referenced Morphir type {name} is unavailable"),
                    )
                })?;
                match definition {
                    Definition::Alias { params, body } => {
                        let bound = bind_type_args(params, args, vars, path)?;
                        self.validate(body, value, &bound, path, next)
                    }
                    Definition::Custom {
                        params,
                        constructors,
                    } => {
                        let bound = bind_type_args(params, args, vars, path)?;
                        let items = value
                            .as_array()
                            .ok_or_else(|| mismatch(path, "expected constructor array"))?;
                        let tag = items.first().and_then(Value::as_str).ok_or_else(|| {
                            mismatch(path, "constructor array needs a string tag")
                        })?;
                        let fields = constructors.get(tag).ok_or_else(|| {
                            error(
                                DataValueErrorKind::UnknownConstructor,
                                path,
                                format!("unknown constructor {tag} for {name}"),
                            )
                        })?;
                        if fields.len() + 1 != items.len() {
                            return Err(mismatch(
                                path,
                                format!("constructor {tag} expects {} arguments", fields.len()),
                            ));
                        }
                        for (index, (field_type, field_value)) in
                            fields.iter().zip(&items[1..]).enumerate()
                        {
                            self.validate(
                                field_type,
                                field_value,
                                &bound,
                                &format!("{path}[{}]", index + 1),
                                next,
                            )?;
                        }
                        Ok(())
                    }
                    Definition::Unsupported(kind) => Err(unsupported(
                        path,
                        format!("{kind} has no closed JSON data mapping"),
                    )),
                }
            }
            Shape::Record(fields) => {
                let members = value
                    .as_object()
                    .ok_or_else(|| mismatch(path, "expected record object"))?;
                for (name, field_type) in fields {
                    let field = members
                        .get(name)
                        .ok_or_else(|| mismatch(path, format!("missing field {name}")))?;
                    self.validate(field_type, field, vars, &member_path(path, name), next)?;
                }
                if members.len() != fields.len() {
                    return Err(mismatch(path, "record has unknown fields"));
                }
                Ok(())
            }
            Shape::Tuple(elements) => {
                let items = value
                    .as_array()
                    .ok_or_else(|| mismatch(path, "expected tuple array"))?;
                if items.len() != elements.len() {
                    return Err(mismatch(
                        path,
                        format!("tuple expects {} elements", elements.len()),
                    ));
                }
                for (index, (item_type, item)) in elements.iter().zip(items).enumerate() {
                    self.validate(item_type, item, vars, &format!("{path}[{index}]"), next)?;
                }
                Ok(())
            }
            Shape::Unsupported(kind) => Err(unsupported(
                path,
                format!("{kind} has no closed JSON data mapping"),
            )),
        }
    }

    fn validate_sdk(
        &self,
        sdk: (&str, &str),
        args: &[Shape],
        value: &Value,
        vars: &HashMap<String, Shape>,
        path: &str,
        next: usize,
    ) -> Result<(), DataValueError> {
        match (sdk.0, sdk.1, args) {
            ("basics", "bool", []) => expect_primitive(value.is_boolean(), path, "Bool"),
            ("basics", "int", []) => expect_primitive(
                v4::serde_tagged::integer_from_json(value).is_some(),
                path,
                "Int",
            ),
            ("basics", "float", []) => expect_primitive(value.is_number(), path, "Float"),
            ("string", "string", []) => expect_primitive(value.is_string(), path, "String"),
            ("char", "char", []) => expect_primitive(
                value.as_str().is_some_and(|text| text.chars().count() == 1),
                path,
                "Char",
            ),
            ("decimal", "decimal", []) => expect_primitive(
                value
                    .as_str()
                    .is_some_and(crate::ir::decimal::is_decimal_lexeme),
                path,
                "Decimal",
            ),
            ("list", "list", [item_type]) => {
                let items = value
                    .as_array()
                    .ok_or_else(|| mismatch(path, "expected list"))?;
                for (index, item) in items.iter().enumerate() {
                    self.validate(item_type, item, vars, &format!("{path}[{index}]"), next)?;
                }
                Ok(())
            }
            ("maybe", "maybe", [_]) if value.is_null() => Ok(()),
            ("maybe", "maybe", [item_type]) => self.validate(item_type, value, vars, path, next),
            ("dict", "dict", [key_type, value_type]) => {
                if !self.is_string_key_type(key_type, vars, path, &mut HashSet::new(), 0)? {
                    return Err(unsupported(
                        path,
                        "Dict keys other than String have no JSON object mapping",
                    ));
                }
                let entries = value
                    .as_object()
                    .ok_or_else(|| mismatch(path, "expected Dict String object"))?;
                for (key, entry) in entries {
                    self.validate(value_type, entry, vars, &member_path(path, key), next)?;
                }
                Ok(())
            }
            _ => Err(unsupported(
                path,
                format!(
                    "SDK type {}#{} has no closed JSON data mapping with {} arguments",
                    sdk.0,
                    sdk.1,
                    args.len()
                ),
            )),
        }
    }

    fn is_string_key_type(
        &self,
        key_type: &Shape,
        vars: &HashMap<String, Shape>,
        path: &str,
        visiting: &mut HashSet<(String, Vec<Shape>)>,
        depth: usize,
    ) -> Result<bool, DataValueError> {
        if depth > 128 {
            return Err(unsupported(path, "Dict key alias nesting exceeds 128"));
        }
        let resolved = substitute_type(key_type, vars, &mut HashSet::new(), path)?;
        let Shape::Reference(name, args) = resolved else {
            return Ok(false);
        };
        if let Some((module, local)) = sdk_type(&name) {
            return Ok((module, local) == ("string", "string") && args.is_empty());
        }
        let Some(Definition::Alias { params, body }) = self.definitions.get(&name) else {
            return Ok(false);
        };
        let instance = (name.clone(), args.clone());
        if !visiting.insert(instance.clone()) {
            return Err(unsupported(path, format!("cyclic Dict key alias {name}")));
        }
        let result = bind_type_args(params, &args, vars, path)
            .and_then(|bound| self.is_string_key_type(body, &bound, path, visiting, depth + 1));
        visiting.remove(&instance);
        result
    }
}

fn expect_primitive(valid: bool, path: &str, name: &str) -> Result<(), DataValueError> {
    if valid {
        Ok(())
    } else {
        Err(mismatch(path, format!("expected {name}")))
    }
}

fn member_path(path: &str, name: &str) -> String {
    let mut bytes = name.bytes();
    let safe_start = bytes
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == b'_');
    if safe_start && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_') {
        format!("{path}.{name}")
    } else {
        let quoted = serde_json::to_string(name).expect("JSON member names serialize");
        format!("{path}[{quoted}]")
    }
}

fn sdk_type(name: &str) -> Option<(&str, &str)> {
    let (package, rest) = name.split_once(':')?;
    if package != "morphir/SDK" && package != "morphir/s-d-k" {
        return None;
    }
    rest.split_once('#')
}

fn bind_type_args(
    params: &[String],
    args: &[Shape],
    outer: &HashMap<String, Shape>,
    path: &str,
) -> Result<HashMap<String, Shape>, DataValueError> {
    if params.len() != args.len() {
        return Err(unsupported(
            path,
            format!("type expects {} arguments", params.len()),
        ));
    }
    let arguments = args
        .iter()
        .map(|arg| substitute_type(arg, outer, &mut HashSet::new(), path))
        .collect::<Result<Vec<_>, _>>()?;
    let mut bound = outer.clone();
    bound.extend(params.iter().cloned().zip(arguments));
    Ok(bound)
}

fn substitute_type(
    shape: &Shape,
    vars: &HashMap<String, Shape>,
    visiting: &mut HashSet<String>,
    path: &str,
) -> Result<Shape, DataValueError> {
    match shape {
        Shape::Variable(name) => {
            let Some(bound) = vars.get(name) else {
                return Ok(shape.clone());
            };
            if !visiting.insert(name.clone()) {
                return Err(unsupported(path, format!("cyclic type variable {name}")));
            }
            let resolved = substitute_type(bound, vars, visiting, path);
            visiting.remove(name);
            resolved
        }
        Shape::Reference(name, args) => Ok(Shape::Reference(
            name.clone(),
            args.iter()
                .map(|arg| substitute_type(arg, vars, visiting, path))
                .collect::<Result<_, _>>()?,
        )),
        Shape::Record(fields) => Ok(Shape::Record(
            fields
                .iter()
                .map(|(name, shape)| {
                    substitute_type(shape, vars, visiting, path).map(|shape| (name.clone(), shape))
                })
                .collect::<Result<_, _>>()?,
        )),
        Shape::Tuple(elements) => Ok(Shape::Tuple(
            elements
                .iter()
                .map(|shape| substitute_type(shape, vars, visiting, path))
                .collect::<Result<_, _>>()?,
        )),
        Shape::Unit | Shape::Unsupported(_) => Ok(shape.clone()),
    }
}
