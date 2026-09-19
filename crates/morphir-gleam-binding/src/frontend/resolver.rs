//! Resolve Gleam type references against local declarations and dependency interfaces.
//!
//! ```
//! use morphir_gleam_binding::frontend::{parse_gleam, resolver::resolve_modules};
//! use morphir_core::naming::PackageName;
//! let module = parse_gleam("main.gleam", "pub type Id = Int").unwrap();
//! let resolved = resolve_modules(&PackageName::parse("demo"), &[module], &Default::default());
//! assert!(resolved.is_ok());
//! ```
use super::ast::{Access, ModuleIR, Span, TypeExpr};
use indexmap::IndexMap;
use morphir_core::ir::v4::{
    Access as IrAccess, AccessControlled, ModuleDefinition, PackageSpecification, TypeDefinition,
    TypeSpecification,
};
use morphir_core::naming::{FQName, ModuleName, Name, PackageName};
use std::collections::{BTreeMap, BTreeSet};

mod annotations;
mod cycles;
mod symbols;

use annotations::resolve_body_annotations;
pub use cycles::validate_alias_cycles;
pub(crate) use symbols::builtin;
pub use symbols::module_name;
use symbols::{Interface, Symbol, ast_interface, dependency_interfaces, fqname, sdk_interfaces};

/// A type resolution error attached to its declaration.
#[derive(Debug, Clone)]
pub struct ResolutionError {
    /// Stable diagnostic code.
    pub code: &'static str,
    /// Source module name.
    pub module: String,
    /// Declaration byte span.
    pub span: Span,
    /// Actionable explanation.
    pub message: String,
}

/// Resolve a complete package of parsed modules.
pub fn resolve_modules(
    package: &PackageName,
    modules: &[ModuleIR],
    dependencies: &IndexMap<String, PackageSpecification>,
) -> Result<Vec<ModuleIR>, Vec<ResolutionError>> {
    let interfaces = modules
        .iter()
        .map(|module| ast_interface(package, module))
        .chain(dependency_interfaces(dependencies))
        .chain(sdk_interfaces())
        .collect::<Vec<_>>();
    let mut errors = Vec::new();
    let resolved = modules
        .iter()
        .filter_map(|module| match resolve(package, module, &interfaces) {
            Ok(module) => Some(module),
            Err(failures) => {
                errors.extend(failures);
                None
            }
        })
        .collect::<Vec<_>>();
    errors.extend(validate_alias_cycles(&resolved));
    if errors.is_empty() {
        Ok(resolved)
    } else {
        Err(errors)
    }
}

/// Resolve one module against the current or baseline local definitions.
pub fn resolve_one(
    package: &PackageName,
    module: &ModuleIR,
    local_definitions: &IndexMap<String, AccessControlled<ModuleDefinition>>,
    dependencies: &IndexMap<String, PackageSpecification>,
) -> Result<ModuleIR, Vec<ResolutionError>> {
    let mut interfaces = local_definitions
        .iter()
        .filter(|(name, _)| module_name(name) != module_name(&module.name))
        .map(|(module_path, definition)| Interface {
            package: package.to_string(),
            module: module_name(module_path),
            types: definition
                .value
                .types
                .iter()
                .map(|(name, ty)| {
                    let arity = match &ty.value.value {
                        TypeDefinition::TypeAliasDefinition { type_params, .. }
                        | TypeDefinition::CustomTypeDefinition { type_params, .. }
                        | TypeDefinition::IncompleteTypeDefinition { type_params, .. } => {
                            type_params.len()
                        }
                    };
                    (
                        Name::from(name.as_str()).to_string(),
                        Symbol {
                            name: fqname(package, module_path, name),
                            arity,
                            public: ty.access == IrAccess::Public,
                        },
                    )
                })
                .collect(),
        })
        .collect::<Vec<_>>();
    interfaces.push(ast_interface(package, module));
    interfaces.extend(dependency_interfaces(dependencies));
    interfaces.extend(sdk_interfaces());
    let resolved = resolve(package, module, &interfaces)?;
    let errors = validate_alias_cycles(std::slice::from_ref(&resolved));
    if errors.is_empty() {
        Ok(resolved)
    } else {
        Err(errors)
    }
}

struct Scope<'a> {
    package: &'a PackageName,
    module: &'a ModuleIR,
    interfaces: &'a [Interface],
    span: Span,
    params: BTreeSet<String>,
    errors: Vec<ResolutionError>,
}
impl Scope<'_> {
    fn error(&mut self, code: &'static str, message: String) {
        self.errors.push(ResolutionError {
            code,
            message,
            module: self.module.name.clone(),
            span: self.span,
        });
    }
    fn import_targets(&self, path: &str) -> Vec<&Interface> {
        let canonical = module_name(path);
        self.interfaces
            .iter()
            .filter(|interface| interface.module == canonical)
            .collect()
    }
    fn lookup(&mut self, qualifier: Option<&str>, name: &str) -> Option<Symbol> {
        let canonical_name = Name::from(name).to_string();
        if qualifier.is_none() {
            let local = self.interfaces.iter().find(|interface| {
                interface.package == self.package.to_string()
                    && interface.module == module_name(&self.module.name)
            });
            if let Some(symbol) = local.and_then(|interface| interface.types.get(&canonical_name)) {
                return Some(symbol.clone());
            }
        }
        let imports = self
            .module
            .imports
            .iter()
            .filter(|import| match qualifier {
                Some(qualifier) => {
                    import.alias.as_deref().unwrap_or_else(|| {
                        import.module.rsplit('/').next().unwrap_or(&import.module)
                    }) == qualifier
                }
                None => import.types.iter().any(|(_, alias)| alias == name),
            })
            .cloned()
            .collect::<Vec<_>>();
        let mut candidates = Vec::new();
        for import in imports {
            let original = if qualifier.is_none() {
                import
                    .types
                    .iter()
                    .find(|(_, alias)| alias == name)
                    .map(|(original, _)| original.as_str())
                    .unwrap_or(name)
            } else {
                name
            };
            let targets = self.import_targets(&import.module);
            if targets.len() > 1 {
                self.error(
                    "GLEAM_RESOLVE_AMBIGUOUS",
                    format!(
                        "Imported module '{}' exists in multiple packages",
                        import.module
                    ),
                );
                return None;
            }
            if let Some(symbol) = targets
                .first()
                .and_then(|interface| interface.types.get(&Name::from(original).to_string()))
            {
                candidates.push(symbol.clone());
            }
        }
        if candidates.len() > 1 {
            self.error(
                "GLEAM_RESOLVE_AMBIGUOUS",
                format!("Type '{name}' is imported more than once"),
            );
            return None;
        }
        if let Some(symbol) = candidates.pop() {
            if !symbol.public {
                self.error(
                    "GLEAM_PRIVATE_TYPE",
                    format!("Type '{name}' is private in its declaring module"),
                );
                return None;
            }
            return Some(symbol);
        }
        if qualifier.is_none()
            && let Some((name, arity)) = builtin(name)
        {
            return Some(Symbol {
                name,
                arity,
                public: true,
            });
        }
        self.error(
            "GLEAM_RESOLVE_NOT_FOUND",
            format!(
                "Unknown type '{}{}'",
                qualifier.map(|q| format!("{q}.")).unwrap_or_default(),
                name
            ),
        );
        None
    }
    fn ty(&mut self, ty: &TypeExpr) -> TypeExpr {
        match ty {
            TypeExpr::Named {
                module,
                name,
                parameters,
            } => {
                let mut parameters = parameters.iter().map(|ty| self.ty(ty)).collect::<Vec<_>>();
                if module.is_none() && name == "Nil" {
                    if !parameters.is_empty() {
                        self.error("GLEAM_TYPE_ARITY", "Nil expects no type arguments".into());
                    }
                    return TypeExpr::Unit;
                }
                if let Some(symbol) = self.lookup(module.as_deref(), name) {
                    if parameters.len() != symbol.arity {
                        self.error(
                            "GLEAM_TYPE_ARITY",
                            format!(
                                "Type '{name}' expects {} type arguments, got {}",
                                symbol.arity,
                                parameters.len()
                            ),
                        );
                    }
                    if symbol.name == fqname(&PackageName::parse("morphir/SDK"), "result", "result")
                        && parameters.len() == 2
                    {
                        parameters.swap(0, 1);
                    }
                    TypeExpr::Resolved {
                        name: symbol.name,
                        parameters,
                    }
                } else {
                    ty.clone()
                }
            }
            TypeExpr::Variable { name } => {
                if !self.params.contains(name) {
                    self.error(
                        "GLEAM_UNBOUND_TYPE_VARIABLE",
                        format!("Type variable '{name}' is not declared"),
                    );
                }
                ty.clone()
            }
            TypeExpr::Function {
                parameters,
                return_type,
            } => TypeExpr::Function {
                parameters: parameters.iter().map(|ty| self.ty(ty)).collect(),
                return_type: Box::new(self.ty(return_type)),
            },
            TypeExpr::Tuple { elements } => TypeExpr::Tuple {
                elements: elements.iter().map(|ty| self.ty(ty)).collect(),
            },
            TypeExpr::Record { .. } => {
                self.error("GLEAM_UNSUPPORTED_TYPE", "Gleam has no anonymous structural record types; declare a named custom type with labelled constructor fields".into());
                ty.clone()
            }
            TypeExpr::CustomType { variants } => TypeExpr::CustomType {
                variants: variants
                    .iter()
                    .map(|variant| {
                        let mut variant = variant.clone();
                        variant.fields = variant.fields.iter().map(|ty| self.ty(ty)).collect();
                        variant
                    })
                    .collect(),
            },
            TypeExpr::Resolved { .. } | TypeExpr::Unit | TypeExpr::Hole { .. } => ty.clone(),
        }
    }
}

fn resolve(
    package: &PackageName,
    module: &ModuleIR,
    interfaces: &[Interface],
) -> Result<ModuleIR, Vec<ResolutionError>> {
    let mut scope = Scope {
        package,
        module,
        interfaces,
        span: Span::default(),
        params: BTreeSet::new(),
        errors: Vec::new(),
    };
    for import in &module.imports {
        match scope.import_targets(&import.module).len() {
            0 => scope.error(
                "GLEAM_RESOLVE_NOT_FOUND",
                format!("Unknown imported module '{}'", import.module),
            ),
            1 => {}
            _ => scope.error(
                "GLEAM_RESOLVE_AMBIGUOUS",
                format!(
                    "Imported module '{}' exists in multiple packages",
                    import.module
                ),
            ),
        }
        for (name, alias) in &import.types {
            let _ = name;
            scope.lookup(None, alias);
        }
    }
    let mut names = BTreeSet::new();
    let mut constructors = BTreeSet::new();
    let mut resolved = module.clone();
    for definition in &mut resolved.types {
        scope.span = definition.span;
        if !names.insert(Name::from(definition.name.as_str()).to_string()) {
            scope.error(
                "GLEAM_DUPLICATE_TYPE",
                format!("Type '{}' is declared more than once", definition.name),
            );
        }
        scope.params = definition.params.iter().cloned().collect();
        if scope.params.len() != definition.params.len() {
            scope.error(
                "GLEAM_DUPLICATE_TYPE_PARAMETER",
                format!("Type '{}' repeats a type parameter", definition.name),
            );
        }
        if let TypeExpr::CustomType { variants } = &definition.body {
            for variant in variants {
                if !constructors.insert(Name::from(variant.name.as_str()).to_string()) {
                    scope.error(
                        "GLEAM_DUPLICATE_CONSTRUCTOR",
                        format!("Constructor '{}' is declared more than once", variant.name),
                    );
                }
                let labels = variant.labels.iter().flatten().collect::<BTreeSet<_>>();
                if labels.len() != variant.labels.iter().flatten().count() {
                    scope.error(
                        "GLEAM_DUPLICATE_LABEL",
                        format!("Constructor '{}' repeats an argument label", variant.name),
                    );
                }
            }
        }
        definition.body = scope.ty(&definition.body);
    }
    for definition in &mut resolved.values {
        scope.span = definition.span;
        scope.params.clear();
        if let Some(annotation) = &definition.type_annotation {
            collect_variables(annotation, &mut scope.params);
            definition.type_annotation = Some(scope.ty(annotation));
        }
        resolve_body_annotations(&mut definition.body, &mut scope);
    }
    if scope.errors.is_empty() {
        Ok(resolved)
    } else {
        Err(scope.errors)
    }
}

fn collect_variables(ty: &TypeExpr, variables: &mut BTreeSet<String>) {
    if let TypeExpr::Variable { name } = ty {
        variables.insert(name.clone());
    }
    for child in children(ty) {
        collect_variables(child, variables);
    }
}
fn children(ty: &TypeExpr) -> Vec<&TypeExpr> {
    match ty {
        TypeExpr::Function {
            parameters,
            return_type,
        } => parameters
            .iter()
            .chain(std::iter::once(return_type.as_ref()))
            .collect(),
        TypeExpr::Named { parameters, .. } | TypeExpr::Resolved { parameters, .. } => {
            parameters.iter().collect()
        }
        TypeExpr::Tuple { elements } => elements.iter().collect(),
        TypeExpr::Record { fields } => fields.iter().map(|(_, ty)| ty).collect(),
        TypeExpr::CustomType { variants } => variants
            .iter()
            .flat_map(|variant| &variant.fields)
            .collect(),
        _ => Vec::new(),
    }
}
