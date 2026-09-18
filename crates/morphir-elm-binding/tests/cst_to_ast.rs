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
