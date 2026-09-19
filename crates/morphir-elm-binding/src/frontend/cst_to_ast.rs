//! Lowers the tree-sitter CST produced by [`crate::frontend::parse`] into the
//! Elm type AST ([`crate::ast`]).
//!
//! This is the only module in the crate allowed to name tree-sitter node
//! kinds or field names; everything downstream works with [`crate::ast`]
//! types instead.

use crate::ast::{
    Constructor, Exposed, Exposing, Field, Import, Module, SkippedValue, TypeDecl, TypeExpr,
};
use crate::frontend::parse::ParsedTree;
use crate::span::Span;
use tree_sitter::Node;

/// Failure lowering the CST into an AST: a missing/malformed module
/// declaration, or an unsupported construct inside a type expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AstError {
    /// Where the construct is.
    pub span: Span,
    /// What the reader has to change.
    pub message: String,
}

struct Lower<'a> {
    src: &'a str,
    /// The node id of the block comment taken as the module's doc, once it is
    /// known. A declaration never claims that comment as its own.
    module_doc: Option<usize>,
}

impl<'a> Lower<'a> {
    fn text(&self, node: Node) -> &'a str {
        &self.src[node.start_byte()..node.end_byte()]
    }

    fn span(&self, node: Node) -> Span {
        Span {
            start: node.start_byte(),
            end: node.end_byte(),
        }
    }

    /// The dotted segments of an `upper_case_qid` node (e.g. `My.Domain.Types`).
    fn qid(&self, node: Node<'a>) -> Vec<String> {
        let mut cursor = node.walk();
        node.named_children(&mut cursor)
            .filter(|c| c.kind() == "upper_case_identifier")
            .map(|c| self.text(c).to_string())
            .collect()
    }

    /// The `{-| ... -}` doc comment text for `node`, given a named sibling
    /// that is (or is not) a doc block comment.
    fn doc_text(&self, comment: Node) -> Option<String> {
        let text = self.text(comment);
        let inner = text.strip_prefix("{-|")?;
        let inner = inner.strip_suffix("-}")?;
        Some(inner.trim().to_string())
    }

    /// Doc comment immediately preceding `node`, if any.
    ///
    /// The comment that follows the module header is the module's doc, and Elm
    /// gives it to the module even when the next thing in the file is a
    /// declaration — which is what a file with no imports looks like. So the
    /// module's own doc is never handed to a declaration as well.
    fn doc_before(&self, node: Node) -> Option<String> {
        let prev = node.prev_named_sibling()?;
        if prev.kind() != "block_comment" || Some(prev.id()) == self.module_doc {
            return None;
        }
        self.doc_text(prev)
    }

    /// The block comment immediately following `node`, if it is a doc comment
    /// (used for the module doc).
    fn doc_node_after(&self, node: Node<'a>) -> Option<(Node<'a>, String)> {
        let next = node.next_named_sibling()?;
        if next.kind() != "block_comment" {
            return None;
        }
        self.doc_text(next).map(|text| (next, text))
    }

    fn exposing_list(&self, node: Node<'a>) -> Exposing {
        if node.child_by_field_name("doubleDot").is_some() {
            return Exposing::All;
        }
        let mut cursor = node.walk();
        let exposed = node
            .named_children(&mut cursor)
            .filter_map(|child| self.exposed(child))
            .collect();
        Exposing::Explicit(exposed)
    }

    fn exposed(&self, node: Node<'a>) -> Option<Exposed> {
        match node.kind() {
            "exposed_type" => {
                let mut cursor = node.walk();
                let name = node
                    .named_children(&mut cursor)
                    .find(|c| c.kind() == "upper_case_identifier")
                    .map(|c| self.text(c).to_string())?;
                let mut cursor = node.walk();
                let constructors = node
                    .named_children(&mut cursor)
                    .any(|c| c.kind() == "exposed_union_constructors");
                Some(Exposed::Type { name, constructors })
            }
            "exposed_value" => {
                let mut cursor = node.walk();
                let name = node
                    .named_children(&mut cursor)
                    .find(|c| c.kind() == "lower_case_identifier")
                    .map(|c| self.text(c).to_string())?;
                Some(Exposed::Value(name))
            }
            _ => None,
        }
    }

    fn import_clause(&self, node: Node<'a>) -> Import {
        let module = node
            .child_by_field_name("moduleName")
            .map(|n| self.qid(n))
            .unwrap_or_default();
        let alias = node.child_by_field_name("asClause").and_then(|as_clause| {
            as_clause
                .child_by_field_name("name")
                .map(|n| self.text(n).to_string())
        });
        let exposing = node
            .child_by_field_name("exposing")
            .map(|n| self.exposing_list(n));
        Import {
            module,
            alias,
            exposing,
            span: self.span(node),
        }
    }

    /// One segment of a `type_expression`, or one `part` of a `type_ref` or
    /// `union_variant`: `type_ref`, `type_variable`, `record_type`,
    /// `tuple_type`, or a parenthesised (nested) `type_expression`.
    fn type_expr_part(&self, node: Node<'a>) -> Result<TypeExpr, AstError> {
        match node.kind() {
            "type_ref" => self.type_ref(node),
            "type_variable" => Ok(TypeExpr::Var {
                name: self.text(node).to_string(),
                span: self.span(node),
            }),
            "record_type" => self.record_type(node),
            "tuple_type" => self.tuple_type(node),
            "type_expression" => self.type_expression(node),
            other => Err(AstError {
                span: self.span(node),
                message: format!("unsupported type syntax `{other}`"),
            }),
        }
    }

    /// A `type_expression` node: one or more segments joined by `arrow`s,
    /// folded right-associatively into nested [`TypeExpr::Function`]s. A
    /// single segment (including a parenthesised type expression) unwraps to
    /// that segment directly.
    ///
    /// The segments are the node's named children in source order, and *not*
    /// its `part`-tagged children: tree-sitter-elm leaves a segment untagged
    /// when it is a `type_ref` carrying arguments, so `List Int -> Bool` tags
    /// only `Bool`. Reading the field would drop `List Int` silently and lower
    /// the alias to `Bool`. Everything between the segments — the `->` token
    /// and any comment written mid-type — is skipped; anything else that turns
    /// up is handed to [`Lower::type_expr_part`], which reports it rather than
    /// ignoring it.
    fn type_expression(&self, node: Node<'a>) -> Result<TypeExpr, AstError> {
        let mut cursor = node.walk();
        let parts: Vec<Node> = node
            .named_children(&mut cursor)
            .filter(|child| !matches!(child.kind(), "arrow" | "line_comment" | "block_comment"))
            .collect();
        if parts.is_empty() {
            return Err(AstError {
                span: self.span(node),
                message: "empty type expression".to_string(),
            });
        }
        let exprs: Vec<TypeExpr> = parts
            .into_iter()
            .map(|part| self.type_expr_part(part))
            .collect::<Result<_, _>>()?;
        let span = self.span(node);
        let mut it = exprs.into_iter().rev();
        let mut acc = it.next().expect("checked non-empty above");
        for expr in it {
            acc = TypeExpr::Function {
                arg: Box::new(expr),
                result: Box::new(acc),
                span,
            };
        }
        Ok(acc)
    }

    fn type_ref(&self, node: Node<'a>) -> Result<TypeExpr, AstError> {
        let mut cursor = node.walk();
        let qid_node = node
            .children(&mut cursor)
            .find(|c| c.kind() == "upper_case_qid")
            .ok_or_else(|| AstError {
                span: self.span(node),
                message: "type reference is missing a name".to_string(),
            })?;
        let mut segments = self.qid(qid_node);
        let name = segments.pop().ok_or_else(|| AstError {
            span: self.span(qid_node),
            message: "type reference has an empty name".to_string(),
        })?;
        let mut cursor = node.walk();
        let args = node
            .children_by_field_name("part", &mut cursor)
            .map(|part| self.type_expr_part(part))
            .collect::<Result<_, _>>()?;
        Ok(TypeExpr::Ref {
            module: segments,
            name,
            args,
            span: self.span(node),
        })
    }

    fn record_type(&self, node: Node<'a>) -> Result<TypeExpr, AstError> {
        let mut cursor = node.walk();
        let fields = node
            .children_by_field_name("fieldType", &mut cursor)
            .map(|field_type| self.field_type(field_type))
            .collect::<Result<Vec<_>, _>>()?;
        let span = self.span(node);
        match node.child_by_field_name("baseRecord") {
            Some(base) => Ok(TypeExpr::ExtensibleRecord {
                base: self.text(base).to_string(),
                fields,
                span,
            }),
            None => Ok(TypeExpr::Record { fields, span }),
        }
    }

    fn field_type(&self, node: Node<'a>) -> Result<Field, AstError> {
        let name = node
            .child_by_field_name("name")
            .map(|n| self.text(n).to_string())
            .ok_or_else(|| AstError {
                span: self.span(node),
                message: "record field is missing a name".to_string(),
            })?;
        let ty_node = node
            .child_by_field_name("typeExpression")
            .ok_or_else(|| AstError {
                span: self.span(node),
                message: "record field is missing a type".to_string(),
            })?;
        let ty = self.type_expression(ty_node)?;
        Ok(Field { name, ty })
    }

    fn tuple_type(&self, node: Node<'a>) -> Result<TypeExpr, AstError> {
        let span = self.span(node);
        if node.child_by_field_name("unitExpr").is_some() {
            return Ok(TypeExpr::Unit { span });
        }
        let mut cursor = node.walk();
        let items = node
            .children_by_field_name("typeExpression", &mut cursor)
            .map(|item| self.type_expression(item))
            .collect::<Result<_, _>>()?;
        Ok(TypeExpr::Tuple { items, span })
    }

    fn type_alias(&self, node: Node<'a>) -> Result<TypeDecl, AstError> {
        let name = node
            .child_by_field_name("name")
            .map(|n| self.text(n).to_string())
            .ok_or_else(|| AstError {
                span: self.span(node),
                message: "type alias is missing a name".to_string(),
            })?;
        let mut cursor = node.walk();
        let params = node
            .children_by_field_name("typeVariable", &mut cursor)
            .map(|n| self.text(n).to_string())
            .collect();
        let body_node = node
            .child_by_field_name("typeExpression")
            .ok_or_else(|| AstError {
                span: self.span(node),
                message: "type alias is missing a body".to_string(),
            })?;
        let body = self.type_expression(body_node)?;
        Ok(TypeDecl::Alias {
            name,
            params,
            body,
            doc: self.doc_before(node),
            span: self.span(node),
        })
    }

    fn type_declaration(&self, node: Node<'a>) -> Result<TypeDecl, AstError> {
        let name = node
            .child_by_field_name("name")
            .map(|n| self.text(n).to_string())
            .ok_or_else(|| AstError {
                span: self.span(node),
                message: "custom type is missing a name".to_string(),
            })?;
        let mut cursor = node.walk();
        let params = node
            .children_by_field_name("typeName", &mut cursor)
            .map(|n| self.text(n).to_string())
            .collect();
        let mut cursor = node.walk();
        let constructors = node
            .children_by_field_name("unionVariant", &mut cursor)
            .map(|variant| self.union_variant(variant))
            .collect::<Result<_, _>>()?;
        Ok(TypeDecl::Custom {
            name,
            params,
            constructors,
            doc: self.doc_before(node),
            span: self.span(node),
        })
    }

    fn union_variant(&self, node: Node<'a>) -> Result<Constructor, AstError> {
        let name = node
            .child_by_field_name("name")
            .map(|n| self.text(n).to_string())
            .ok_or_else(|| AstError {
                span: self.span(node),
                message: "union variant is missing a name".to_string(),
            })?;
        let mut cursor = node.walk();
        let args = node
            .children_by_field_name("part", &mut cursor)
            .map(|part| self.type_expr_part(part))
            .collect::<Result<_, _>>()?;
        Ok(Constructor {
            name,
            args,
            span: self.span(node),
        })
    }

    /// The name of a `type_annotation` or `value_declaration`, used for
    /// [`SkippedValue`]. A pattern-destructuring value declaration (no
    /// `functionDeclarationLeft`) uses the text of its whole left side.
    fn skipped_value_name(&self, node: Node<'a>) -> Option<String> {
        match node.kind() {
            "type_annotation" => node
                .child_by_field_name("name")
                .map(|n| self.text(n).to_string()),
            "value_declaration" => {
                if let Some(left) = node.child_by_field_name("functionDeclarationLeft") {
                    let mut cursor = left.walk();
                    left.children(&mut cursor)
                        .find(|c| c.kind() == "lower_case_identifier")
                        .map(|c| self.text(c).to_string())
                        .or_else(|| Some(self.text(left).to_string()))
                } else {
                    node.child_by_field_name("pattern")
                        .map(|p| self.text(p).to_string())
                }
            }
            _ => None,
        }
    }
}

/// Lowers a parsed Elm source file into the type AST.
///
/// Fails only when the module declaration is missing or malformed, or when a
/// type expression contains a construct this frontend does not model. Value
/// declarations are never an error: they are recorded in
/// [`Module::skipped_values`].
pub fn to_ast(parsed: &ParsedTree, source: &str) -> Result<Module, AstError> {
    let mut lower = Lower {
        src: source,
        module_doc: None,
    };
    let root = parsed.tree.root_node();

    let module_decl = root
        .child_by_field_name("moduleDeclaration")
        .ok_or_else(|| AstError {
            span: lower.span(root),
            message: "missing module declaration".to_string(),
        })?;
    let name_node = module_decl
        .child_by_field_name("name")
        .ok_or_else(|| AstError {
            span: lower.span(module_decl),
            message: "module declaration is missing a name".to_string(),
        })?;
    let exposing_node = module_decl
        .child_by_field_name("exposing")
        .ok_or_else(|| AstError {
            span: lower.span(module_decl),
            message: "module declaration is missing an exposing list".to_string(),
        })?;
    let name = lower.qid(name_node);
    let exposing = lower.exposing_list(exposing_node);
    let doc = match lower.doc_node_after(module_decl) {
        Some((node, text)) => {
            lower.module_doc = Some(node.id());
            Some(text)
        }
        None => None,
    };
    let lower = lower;

    let mut imports = Vec::new();
    let mut types = Vec::new();
    let mut skipped_values: Vec<SkippedValue> = Vec::new();

    let mut cursor = root.walk();
    for child in root.named_children(&mut cursor) {
        match child.kind() {
            "module_declaration" | "block_comment" => {}
            "import_clause" => imports.push(lower.import_clause(child)),
            "type_alias_declaration" => types.push(lower.type_alias(child)?),
            "type_declaration" => types.push(lower.type_declaration(child)?),
            "type_annotation" | "value_declaration" => {
                if let Some(name) = lower.skipped_value_name(child)
                    && !skipped_values.iter().any(|v: &SkippedValue| v.name == name)
                {
                    skipped_values.push(SkippedValue {
                        name,
                        span: lower.span(child),
                    });
                }
            }
            _ => {}
        }
    }

    Ok(Module {
        name,
        exposing,
        imports,
        doc,
        types,
        skipped_values,
        span: lower.span(root),
    })
}
