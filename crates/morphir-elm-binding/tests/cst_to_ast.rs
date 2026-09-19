use morphir_elm_binding::ast::*;
use morphir_elm_binding::frontend::{
    cst_to_ast::{DocComments, to_ast},
    parse::parse,
};

const SRC: &str = include_str!("fixtures/Types.elm");

fn module() -> Module {
    to_ast(&parse(SRC), SRC, DocComments::Trimmed).expect("fixture lowers")
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
    let m = to_ast(&parse(src), src, DocComments::Trimmed).expect("the module lowers");

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
    let m = to_ast(&parse(src), src, DocComments::Trimmed).expect("the module lowers");

    assert_eq!(m.doc.as_deref(), Some("The module."));
    let TypeDecl::Alias { doc, .. } = &m.types[0] else {
        panic!("alias")
    };
    assert_eq!(doc.as_deref(), Some("The type."));
}

// ----------------------------------------------------------------------------
// Doc comment modes
// ----------------------------------------------------------------------------

/// The module doc and the first declaration's doc, lowered in `mode`.
fn docs(src: &str, mode: DocComments) -> (Option<String>, Option<String>) {
    let module = to_ast(&parse(src), src, mode).expect("the module lowers");
    let declaration = match module.types.first() {
        Some(TypeDecl::Alias { doc, .. } | TypeDecl::Custom { doc, .. }) => doc.clone(),
        None => None,
    };
    (module.doc, declaration)
}

/// `morphir-elm` keeps the text between the delimiters exactly as it was
/// written; `trimmed` takes the surrounding whitespace off.
#[test]
fn the_two_doc_comment_modes_differ_in_the_whitespace_they_keep() {
    let src = "module A exposing (..)\n\n\
               {-| Values of several kinds.\n-}\n\n\n\
               {-|    Leading and trailing whitespace.   \n-}\n\
               type alias T =\n    Int\n";

    assert_eq!(
        docs(src, DocComments::MorphirElm),
        (
            Some(" Values of several kinds.".to_string()),
            Some("    Leading and trailing whitespace.   \n".to_string())
        )
    );
    assert_eq!(
        docs(src, DocComments::Trimmed),
        (
            Some("Values of several kinds.".to_string()),
            Some("Leading and trailing whitespace.".to_string())
        )
    );
}

/// A doc that runs over several lines keeps its interior newlines in both
/// modes; only the outside differs.
#[test]
fn a_multi_line_doc_keeps_its_interior_in_both_modes() {
    let src = "module A exposing (..)\n\n\
               {-| Type aliases of every shape.\n\n\
               \x20 - primitives\n  - collections\n\n-}\n\n\n\
               {-| First line.\n\nSecond paragraph.\n-}\n\
               type alias T =\n    Int\n";

    assert_eq!(
        docs(src, DocComments::MorphirElm),
        (
            Some(" Type aliases of every shape.\n\n  - primitives\n  - collections\n".to_string()),
            Some(" First line.\n\nSecond paragraph.\n".to_string())
        )
    );
    assert_eq!(
        docs(src, DocComments::Trimmed),
        (
            Some("Type aliases of every shape.\n\n  - primitives\n  - collections".to_string()),
            Some("First line.\n\nSecond paragraph.".to_string())
        )
    );
}

/// A `|` in the doc body is body text: only the `{-|` opener is a delimiter.
#[test]
fn a_doc_with_a_leading_bar_keeps_the_bar() {
    let src = "module A exposing (..)\n\n\
               import Dict\n\n\n\
               {-| | Doc with a leading bar.\n-}\n\
               type alias T =\n    Int\n";

    assert_eq!(
        docs(src, DocComments::MorphirElm).1,
        Some(" | Doc with a leading bar.\n".to_string())
    );
    assert_eq!(
        docs(src, DocComments::Trimmed).1,
        Some("| Doc with a leading bar.".to_string())
    );
}

/// No doc comment is no doc in either mode — it is the emitters that write the
/// `""` a Morphir document uses for an undocumented type, and `null` for an
/// undocumented module.
#[test]
fn an_undocumented_declaration_has_no_doc_in_either_mode() {
    let src = "module A exposing (..)\n\nimport Dict\n\n\ntype alias T =\n    Int\n";
    for mode in [DocComments::MorphirElm, DocComments::Trimmed] {
        assert_eq!(docs(src, mode), (None, None), "{mode:?}");
    }
}

/// A plain `{- ... -}` block comment is not a doc comment, so it is not taken
/// for one in either mode.
#[test]
fn a_plain_block_comment_is_not_a_doc_in_either_mode() {
    let src = "module A exposing (..)\n\n{- not a doc -}\n\n\n\
               {- nor this -}\ntype alias T =\n    Int\n";
    for mode in [DocComments::MorphirElm, DocComments::Trimmed] {
        assert_eq!(docs(src, mode), (None, None), "{mode:?}");
    }
}

/// The body of `type alias T = <src>`, lowered.
fn alias_body(src: &str) -> TypeExpr {
    let text = format!("module A exposing (..)\n\ntype alias T =\n    {src}\n");
    let module = to_ast(&parse(&text), &text, DocComments::Trimmed)
        .unwrap_or_else(|error| panic!("{error:?}\n{text}"));
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
