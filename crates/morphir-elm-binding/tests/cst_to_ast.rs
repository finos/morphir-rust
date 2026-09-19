use morphir_elm_binding::ast::*;
use morphir_elm_binding::frontend::{cst_to_ast::to_ast, parse::parse};

const SRC: &str = include_str!("fixtures/Types.elm");

fn module() -> Module {
    to_ast(&parse(SRC), SRC).expect("fixture lowers")
}

#[test]
fn header_imports_and_docs() {
    let m = module();
    assert_eq!(m.name, vec!["My", "Domain", "Types"]);
    assert_eq!(
        m.exposing,
        Exposing::Explicit(vec![
            Exposed::Type {
                name: "Account".into(),
                constructors: false
            },
            Exposed::Type {
                name: "Status".into(),
                constructors: true
            },
            Exposed::Type {
                name: "Id".into(),
                constructors: false
            },
        ])
    );
    assert_eq!(m.doc.as_deref(), Some("Module docs."));
    assert_eq!(m.imports.len(), 2);
    assert_eq!(m.imports[0].module, vec!["Dict"]);
    assert_eq!(
        m.imports[0].exposing,
        Some(Exposing::Explicit(vec![Exposed::Type {
            name: "Dict".into(),
            constructors: false
        }]))
    );
    assert_eq!(m.imports[1].alias.as_deref(), Some("Other"));
}

#[test]
fn alias_with_record_body() {
    let m = module();
    let TypeDecl::Alias {
        name,
        params,
        body,
        doc,
        ..
    } = &m.types[0]
    else {
        panic!("alias")
    };
    assert_eq!(name, "Account");
    assert_eq!(params, &vec!["a".to_string()]);
    assert_eq!(doc.as_deref(), Some("An account."));
    let TypeExpr::Record { fields, .. } = body else {
        panic!("record")
    };
    assert_eq!(
        fields.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(),
        ["id", "tags", "extra", "pair", "f"]
    );
    assert!(
        matches!(&fields[1].ty, TypeExpr::Ref { module, name, args, .. } if module.is_empty() && name == "List" && args.len() == 1)
    );
    assert!(
        matches!(&fields[3].ty, TypeExpr::Tuple { items, .. } if items.len() == 2 && matches!(&items[1], TypeExpr::Ref { module, name, .. } if module == &vec!["Other".to_string()] && name == "Thing"))
    );
    let TypeExpr::Function { arg, result, .. } = &fields[4].ty else {
        panic!("function")
    };
    assert!(matches!(**arg, TypeExpr::Ref { ref name, .. } if name == "Int"));
    let TypeExpr::Function {
        arg: arg2,
        result: result2,
        ..
    } = &**result
    else {
        panic!("curried")
    };
    assert!(
        matches!(**arg2, TypeExpr::ExtensibleRecord { ref base, ref fields, .. } if base == "r" && fields.len() == 1)
    );
    assert!(matches!(**result2, TypeExpr::Unit { .. }));
}

#[test]
fn custom_type_constructors_and_skipped_values() {
    let m = module();
    let TypeDecl::Custom {
        name,
        constructors,
        doc,
        ..
    } = &m.types[1]
    else {
        panic!("custom")
    };
    assert_eq!(name, "Status");
    assert_eq!(doc.as_deref(), Some("Account lifecycle."));
    assert_eq!(
        constructors
            .iter()
            .map(|c| (c.name.as_str(), c.args.len()))
            .collect::<Vec<_>>(),
        [("Active", 0), ("Closed", 2), ("Pending", 1)]
    );
    assert!(matches!(constructors[2].args[0], TypeExpr::Record { .. }));
    assert_eq!(
        m.skipped_values
            .iter()
            .map(|v| v.name.as_str())
            .collect::<Vec<_>>(),
        ["greet"]
    );
}

/// With no imports, the comment after the module header sits directly in front
/// of the first declaration. Elm gives it to the module, so the declaration is
/// undocumented rather than borrowing the module's doc.
#[test]
fn the_module_doc_is_not_also_the_first_declarations_doc() {
    let src = "module A exposing (..)\n\n{-| What this module is for. -}\ntype alias T = Int\n";
    let m = to_ast(&parse(src), src).expect("the module lowers");

    assert_eq!(m.doc.as_deref(), Some("What this module is for."));
    assert!(m.imports.is_empty());
    let TypeDecl::Alias { name, doc, .. } = &m.types[0] else {
        panic!("alias")
    };
    assert_eq!(name, "T");
    assert_eq!(doc.as_deref(), None);
}

/// A declaration that has a doc comment of its own still gets it: only the one
/// comment the module claimed is withheld.
#[test]
fn a_declaration_after_the_module_doc_keeps_its_own_doc() {
    let src =
        "module A exposing (..)\n\n{-| The module. -}\n{-| The type. -}\ntype alias T = Int\n";
    let m = to_ast(&parse(src), src).expect("the module lowers");

    assert_eq!(m.doc.as_deref(), Some("The module."));
    let TypeDecl::Alias { doc, .. } = &m.types[0] else {
        panic!("alias")
    };
    assert_eq!(doc.as_deref(), Some("The type."));
}

/// The body of `type alias T = <src>`, lowered.
fn alias_body(src: &str) -> TypeExpr {
    let text = format!("module A exposing (..)\n\ntype alias T =\n    {src}\n");
    let module = to_ast(&parse(&text), &text).unwrap_or_else(|error| panic!("{error:?}\n{text}"));
    let TypeDecl::Alias { body, .. } = module.types.into_iter().next().expect("one declaration")
    else {
        panic!("alias")
    };
    body
}

/// A rendering of a lowered type that names only what these tests are about:
/// the shape of the spine and which references carry which arguments.
fn sketch(ty: &TypeExpr) -> String {
    match ty {
        TypeExpr::Function { arg, result, .. } => {
            format!("({} -> {})", sketch(arg), sketch(result))
        }
        TypeExpr::Ref {
            module, name, args, ..
        } => {
            let qualified = module
                .iter()
                .map(String::as_str)
                .chain(std::iter::once(name.as_str()))
                .collect::<Vec<_>>()
                .join(".");
            if args.is_empty() {
                qualified
            } else {
                format!(
                    "{qualified}[{}]",
                    args.iter().map(sketch).collect::<Vec<_>>().join(", ")
                )
            }
        }
        TypeExpr::Var { name, .. } => name.clone(),
        TypeExpr::Record { fields, .. } => format!(
            "{{{}}}",
            fields
                .iter()
                .map(|field| format!("{}: {}", field.name, sketch(&field.ty)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        TypeExpr::ExtensibleRecord { base, fields, .. } => format!(
            "{{{base} | {}}}",
            fields
                .iter()
                .map(|field| format!("{}: {}", field.name, sketch(&field.ty)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        TypeExpr::Tuple { items, .. } => format!(
            "({})",
            items.iter().map(sketch).collect::<Vec<_>>().join(", ")
        ),
        TypeExpr::Unit { .. } => "()".to_string(),
    }
}

/// tree-sitter-elm tags a `part` field on a segment of a function type only
/// when the segment is not a `type_ref` carrying arguments, so a lowering that
/// read the field dropped `List Int` from `List Int -> Bool` and answered
/// `Bool` — with no diagnostic. Every segment is a part, whatever the grammar
/// tagged.
#[test]
fn every_segment_of_a_function_type_survives_lowering() {
    assert_eq!(
        sketch(&alias_body("(Int -> Int) -> List Int -> List Int")),
        "((Int -> Int) -> (List[Int] -> List[Int]))"
    );
    assert_eq!(
        sketch(&alias_body("List Int -> Bool")),
        "(List[Int] -> Bool)"
    );
    assert_eq!(
        sketch(&alias_body("Int -> Maybe Int -> Bool")),
        "(Int -> (Maybe[Int] -> Bool))"
    );
    assert_eq!(
        sketch(&alias_body("Maybe Int -> Maybe Int -> Maybe Int")),
        "(Maybe[Int] -> (Maybe[Int] -> Maybe[Int]))"
    );
}

/// A parenthesised function used as an argument stays one segment, and the
/// segments on either side of it keep their own arguments.
#[test]
fn a_parenthesised_function_argument_is_one_segment() {
    assert_eq!(
        sketch(&alias_body("(List Int -> Bool) -> Maybe Int -> Bool")),
        "((List[Int] -> Bool) -> (Maybe[Int] -> Bool))"
    );
}

/// Records and tuples are segments too, and were never the ones the grammar
/// left untagged — so this is the case the old lowering got right, kept.
#[test]
fn records_and_tuples_are_segments_of_a_function_type() {
    assert_eq!(
        sketch(&alias_body(
            "{ a : List Int } -> ( Int, List String ) -> Bool"
        )),
        "({a: List[Int]} -> ((Int, List[String]) -> Bool))"
    );
}

/// A type expression with no arrow at all is its one segment, arguments and
/// all: it must not become a function, and must not lose the arguments.
#[test]
fn a_single_applied_reference_is_not_a_function() {
    assert_eq!(sketch(&alias_body("List Int")), "List[Int]");
    assert_eq!(
        sketch(&alias_body("Dict String (List Int)")),
        "Dict[String, List[Int]]"
    );
}

/// A comment written between the segments is not a segment.
#[test]
fn a_comment_inside_a_function_type_is_not_a_segment() {
    assert_eq!(
        sketch(&alias_body("List Int {- why -} -> Bool")),
        "(List[Int] -> Bool)"
    );
}

#[test]
fn source_ranges_are_zero_based_utf16() {
    use morphir_elm_binding::frontend::source::range;
    let src = "module A exposing (..)\n\ntype alias \u{00cf} = Int\n";
    let span = morphir_elm_binding::span::Span {
        start: src.find("Int").unwrap(),
        end: src.len() - 1,
    };
    let r = range(src, span);
    assert_eq!((r.start.line, r.start.character), (2, 15));
    assert_eq!((r.end.line, r.end.character), (2, 18));
}
