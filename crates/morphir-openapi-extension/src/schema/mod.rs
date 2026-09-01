//! The dialect-neutral schema model shared by both generation targets.
//!
//! The model carries Morphir meaning only. It names no JSON Schema keyword and
//! no OpenAPI keyword, so the JSON Schema renderer and the OpenAPI renderer
//! consume the same [`SchemaProjection`] and produce the same schema for a
//! type, apart from the base a `$ref` is written against.

mod names;
mod operations;
mod types;

pub use names::{operation_id, schema_name};
pub use operations::{Operation, project_operations};

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use morphir_projection::{ProjectionPackage, TypeDeclaration};

use crate::{SchemaDiagnostic, SchemaOptions, Unsupported};
use types::{Context, project_declaration};

/// A Morphir type expressed as target-independent schema meaning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Schema {
    /// A boolean value.
    Boolean,
    /// A whole number, with an optional width hint.
    Integer {
        /// Width hint carried through to the renderer.
        format: Option<&'static str>,
    },
    /// A fractional number, with an optional precision hint.
    Number {
        /// Precision hint carried through to the renderer.
        format: Option<&'static str>,
    },
    /// Text, optionally bounded in length.
    Text {
        /// Maximum length, when the Morphir type bounds it.
        max_length: Option<u32>,
    },
    /// The absence of a value.
    Null,
    /// A homogeneous sequence.
    Array {
        /// Element schema.
        items: Box<Schema>,
        /// Whether the elements are a set rather than a list.
        unique: bool,
    },
    /// A fixed-length, positionally typed product.
    Tuple(Vec<Schema>),
    /// Text-keyed values of one schema.
    Map {
        /// Value schema.
        values: Box<Schema>,
    },
    /// A named product.
    Object {
        /// Fields in canonical Morphir order.
        fields: Vec<SchemaField>,
        /// Names of the fields that must be present.
        required: Vec<String>,
    },
    /// A closed set of named values with no payload.
    Enumeration(Vec<String>),
    /// A tagged choice between named variants.
    OneOf {
        /// Name of the property that carries the variant name.
        discriminator: String,
        /// The variants, in canonical Morphir order.
        variants: Vec<SchemaVariant>,
    },
    /// An untagged choice, used for optional values.
    Union(Vec<Schema>),
    /// A reference to a schema registered in [`SchemaProjection::definitions`].
    Reference(String),
}

/// One field of an [`Schema::Object`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaField {
    /// Projected field name.
    pub name: String,
    /// Field schema.
    pub schema: Schema,
    /// Whether the field must be present.
    pub required: bool,
    /// Optional source documentation.
    pub doc: Option<String>,
}

/// One variant of a [`Schema::OneOf`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaVariant {
    /// Projected variant name, also the discriminator value.
    pub name: String,
    /// Schema of the variant payload.
    pub schema: Schema,
    /// Exact canonical Morphir constructor FQName.
    pub source_name: String,
}

/// A schema registered under a projected name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedSchema {
    /// Projected schema name.
    pub name: String,
    /// Exact canonical Morphir FQName this schema was projected from.
    pub source_name: String,
    /// The projected schema.
    pub schema: Schema,
    /// Optional source documentation.
    pub doc: Option<String>,
}

/// The whole projection of one Morphir package.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SchemaProjection {
    /// Canonical Morphir package name, taken directly from
    /// [`ProjectionPackage::package_name`] rather than reconstructed from any
    /// schema, so it is correct even when the package has no roots.
    pub package_name: String,
    /// Declarations owned by the projected package, in declaration order.
    pub roots: Vec<NamedSchema>,
    /// Every reachable schema, keyed by projected name.
    pub definitions: BTreeMap<String, NamedSchema>,
    /// Synthesized OpenAPI operations. Always empty for the `json-schema`
    /// target and for [`crate::Projection::Schemas`]; populated by
    /// [`project_operations`] for the `openapi` target under the other
    /// projection modes.
    pub operations: Vec<Operation>,
    /// Diagnostics raised while projecting, paired with `true` for a warning.
    pub diagnostics: Vec<(SchemaDiagnostic, bool)>,
}

/// Project a normalized Morphir package into the dialect-neutral schema model.
///
/// Every public declaration of `package` becomes a root, and every declaration
/// a root reaches — including declarations owned by a dependency — is added to
/// `definitions`. Two declarations that claim one projected name are a
/// `JSC004` error rather than an implicit rename, whatever `options` says,
/// because renaming one of them would silently break a caller's `$ref`.
///
/// A Morphir form with no schema is a `JSC003` diagnostic. Under
/// [`Unsupported::Error`] it fails the whole projection. Under
/// [`Unsupported::WarnAndSkip`] its declaration is omitted, the diagnostic is
/// recorded as a warning, and the remaining declarations still project.
///
/// This is a projection entry point, so it re-validates `options` itself
/// (`JSC002` on failure) rather than trusting the caller: [`SchemaOptions`]
/// and this function are both public, so a library caller can build
/// `options` directly instead of going through [`SchemaOptions::from_map`],
/// which validates already. Calling `validate` again on the normal
/// `from_map` path is harmless — it is pure and re-checks the same already-
/// valid struct.
pub fn project(
    package: &ProjectionPackage,
    options: &SchemaOptions,
) -> Result<SchemaProjection, SchemaDiagnostic> {
    options.validate()?;
    let declared = declared_types(package);
    let context = Context {
        declared: &declared,
    };

    let mut owned = Vec::new();
    let mut queue = VecDeque::new();
    let mut visited = BTreeSet::new();
    for module in &package.modules {
        for declaration in &module.types {
            let source_name = declaration.source_name().to_owned();
            if visited.insert(source_name.clone()) {
                owned.push(source_name.clone());
                queue.push_back(source_name);
            }
        }
    }

    let mut claimed: BTreeMap<String, String> = BTreeMap::new();
    let mut definitions: BTreeMap<String, NamedSchema> = BTreeMap::new();
    let mut diagnostics = Vec::new();

    close_definitions(
        &context,
        options,
        &mut queue,
        &mut visited,
        &mut claimed,
        &mut definitions,
        &mut diagnostics,
    )?;

    let roots = owned
        .iter()
        .map(|source_name| schema_name(source_name))
        .filter_map(|name| definitions.get(&name).cloned())
        .collect();
    Ok(SchemaProjection {
        package_name: package.package_name.clone(),
        roots,
        definitions,
        operations: Vec::new(),
        diagnostics,
    })
}

/// Close a queue of source names into `definitions`, claiming each
/// projected schema name in `claimed`, enqueueing every further reference
/// [`project_declaration`] reports, and sweeping away anything left
/// dangling once the queue drains.
///
/// Shared by [`project`] — seeded from every public type declaration — and
/// by [`operations::project_operations`], seeded from a request or response
/// type the declaration walk did not already reach. One implementation
/// means the two walks cannot diverge, on any of three points: collision
/// detection (`claimed` and `visited` are caller-owned, so a second call
/// started from `project`'s own `definitions` still catches a projected
/// name a dependency type would otherwise silently alias onto an unrelated
/// definition, as a `JSC004` collision rather than a wrong `$ref`);
/// [`Unsupported`] handling; and dangling-reference cleanup — a skipped
/// declaration under [`Unsupported::WarnAndSkip`] can leave an
/// already-registered definition referring to a name that no longer
/// resolves, whichever call populated it, and [`drop_dangling`] runs here,
/// against the whole `definitions` map, so neither caller can render a
/// `$ref` with no `components/schemas` entry behind it.
fn close_definitions(
    context: &Context<'_>,
    options: &SchemaOptions,
    queue: &mut VecDeque<String>,
    visited: &mut BTreeSet<String>,
    claimed: &mut BTreeMap<String, String>,
    definitions: &mut BTreeMap<String, NamedSchema>,
    diagnostics: &mut Vec<(SchemaDiagnostic, bool)>,
) -> Result<(), SchemaDiagnostic> {
    while let Some(source_name) = queue.pop_front() {
        let Some(declaration) = context.declared.get(&source_name) else {
            continue;
        };
        let name = schema_name(&source_name);
        if let Some(claimant) = claimed.get(&name)
            && claimant != &source_name
        {
            return Err(SchemaDiagnostic::name_collision(
                &source_name,
                format!("projects to schema name '{name}', already claimed by '{claimant}'"),
            ));
        }
        claimed.insert(name.clone(), source_name.clone());

        let mut referenced = BTreeSet::new();
        match project_declaration(context, declaration, &mut referenced) {
            Ok(schema) => {
                definitions.insert(
                    name.clone(),
                    NamedSchema {
                        name,
                        source_name,
                        schema,
                        doc: declaration_doc(declaration),
                    },
                );
                for reference in referenced {
                    if visited.insert(reference.clone()) {
                        queue.push_back(reference);
                    }
                }
            }
            Err(diagnostic) => {
                if options.unsupported == Unsupported::Error {
                    return Err(diagnostic);
                }
                diagnostics.push((diagnostic, true));
            }
        }
    }
    drop_dangling(definitions, diagnostics);
    Ok(())
}

/// Index every declaration the package can see by its canonical FQName.
fn declared_types(package: &ProjectionPackage) -> BTreeMap<String, TypeDeclaration> {
    package
        .modules
        .iter()
        .chain(
            package
                .dependencies
                .iter()
                .flat_map(|dependency| &dependency.modules),
        )
        .flat_map(|module| &module.types)
        .map(|declaration| (declaration.source_name().to_owned(), declaration.clone()))
        .collect()
}

/// Documentation attached to a declaration, if the IR carried any.
fn declaration_doc(declaration: &TypeDeclaration) -> Option<String> {
    match declaration {
        TypeDeclaration::Alias { doc, .. }
        | TypeDeclaration::Opaque { doc, .. }
        | TypeDeclaration::Custom { doc, .. }
        | TypeDeclaration::Incomplete { doc, .. } => doc.clone(),
    }
}

/// Drop, to a fixed point, every definition that refers to a name no longer
/// registered, so a skipped declaration never leaves an unresolvable reference.
///
/// Under [`Unsupported::Error`] nothing is ever skipped, so this does nothing.
fn drop_dangling(
    definitions: &mut BTreeMap<String, NamedSchema>,
    diagnostics: &mut Vec<(SchemaDiagnostic, bool)>,
) {
    loop {
        let dangling = definitions
            .iter()
            .filter(|(_, named)| {
                references(&named.schema)
                    .iter()
                    .any(|reference| !definitions.contains_key(*reference))
            })
            .map(|(name, _)| name.clone())
            .collect::<Vec<_>>();
        if dangling.is_empty() {
            return;
        }
        for name in dangling {
            let Some(named) = definitions.remove(&name) else {
                continue;
            };
            diagnostics.push((
                SchemaDiagnostic::unsupported_form(
                    &named.source_name,
                    "refers to a declaration that was skipped, so it was skipped too",
                ),
                true,
            ));
        }
    }
}

/// Every schema name this schema refers to.
pub(crate) fn references(schema: &Schema) -> Vec<&str> {
    match schema {
        Schema::Reference(name) => vec![name.as_str()],
        Schema::Array { items, .. } => references(items),
        Schema::Map { values } => references(values),
        Schema::Tuple(elements) | Schema::Union(elements) => {
            elements.iter().flat_map(references).collect()
        }
        Schema::Object { fields, .. } => fields
            .iter()
            .flat_map(|field| references(&field.schema))
            .collect(),
        Schema::OneOf { variants, .. } => variants
            .iter()
            .flat_map(|variant| references(&variant.schema))
            .collect(),
        Schema::Boolean
        | Schema::Integer { .. }
        | Schema::Number { .. }
        | Schema::Text { .. }
        | Schema::Null
        | Schema::Enumeration(_) => Vec::new(),
    }
}
