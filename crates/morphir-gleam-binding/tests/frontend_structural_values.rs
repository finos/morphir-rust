//! Structural lowering preserves list shape, bindings and expression order.
use morphir_core::ir::v4::{self, Literal, Pattern, Value, ValueAttributes};
use morphir_core::naming::{FQName, ModuleName, Name, PackageName};
use morphir_extension_sdk::prelude::*;
use morphir_gleam_binding::GleamExtension;

fn compile(source: &str) -> CompileResult {
    GleamExtension
        .compile(CompileRequest {
            language_id: "gleam".into(),
            documents: vec![SourceDocument {
                uri: "file:///src/main.gleam".into(),
                language_id: "gleam".into(),
                version: 1,
                text: source.into(),
            }],
            package: CompilePackage {
                name: "example/structural".into(),
                exposed_modules: None,
            },
            options: CompileOptions {
                ir_version: "4".into(),
                types_only: false,
                extra: [("emitParseStage".into(), false.into())].into(),
            },
            ..Default::default()
        })
        .unwrap()
}
fn body(source: &str) -> Value {
    let result = compile(source);
    assert!(result.success, "{:?}", result.diagnostics);
    let ir: v4::IRFile = serde_json::from_value(result.ir.unwrap()).unwrap();
    let v4::Distribution::Library(library) = ir.distribution else {
        panic!("library")
    };
    let v4::ValueBody::Expression(body) = &library.def.modules["main"].value.values["run"]
        .value
        .value
        .body
    else {
        panic!("expression")
    };
    body.clone()
}
fn attrs() -> ValueAttributes {
    Default::default()
}
fn binding(name: &str) -> Pattern {
    Pattern::AsPattern(
        attrs(),
        Box::new(Pattern::WildcardPattern(attrs())),
        Name::from(name),
    )
}
fn variable(name: &str) -> Value {
    Value::Variable(attrs(), Name::from(name))
}
fn int(value: i64) -> Value {
    Value::Literal(attrs(), Literal::Integer(value.into()))
}
fn empty() -> Pattern {
    Pattern::EmptyListPattern(attrs())
}
fn cons_pattern(head: Pattern, tail: Pattern) -> Pattern {
    Pattern::HeadTailPattern(attrs(), Box::new(head), Box::new(tail))
}
fn cons(head: Value, tail: Value) -> Value {
    let reference = Value::Reference(
        attrs(),
        FQName {
            package_path: PackageName::parse("morphir/SDK").into(),
            module_path: ModuleName::parse("list").into(),
            local_name: Name::from("cons"),
        },
    );
    Value::Apply(
        attrs(),
        Box::new(Value::Apply(attrs(), Box::new(reference), Box::new(head))),
        Box::new(tail),
    )
}
fn destructure(pattern: Pattern, value: Value, next: Value) -> Value {
    Value::Destructure(attrs(), pattern, Box::new(value), Box::new(next))
}

#[test]
fn list_patterns_preserve_empty_fixed_and_explicit_tail_shapes() {
    for (source_pattern, expected) in [
        ("[]", empty()),
        (
            "[x, y]",
            cons_pattern(binding("x"), cons_pattern(binding("y"), empty())),
        ),
        ("[x, ..rest]", cons_pattern(binding("x"), binding("rest"))),
        (
            "[x, .._]",
            cons_pattern(binding("x"), Pattern::WildcardPattern(attrs())),
        ),
        (
            "[[x], ..rest] as all",
            Pattern::AsPattern(
                attrs(),
                Box::new(cons_pattern(
                    cons_pattern(binding("x"), empty()),
                    binding("rest"),
                )),
                Name::from("all"),
            ),
        ),
    ] {
        let actual = body(&format!(
            "pub fn run(items) {{ case items {{ {source_pattern} -> 1\n _ -> 0 }} }}"
        ));
        let Value::PatternMatch(_, _, cases) = actual else {
            panic!("case")
        };
        assert_eq!(cases[0].0, expected, "{source_pattern}");
    }
}

#[test]
fn list_construction_preserves_tail_order_and_nested_tail_expressions() {
    assert_eq!(
        body("pub fn run(tail: List(Int)) -> List(Int) { [1, 2, ..tail] }"),
        cons(int(1), cons(int(2), variable("tail")))
    );
    assert_eq!(
        body("pub fn run(tail: List(Int)) -> List(Int) { [1, ..[2, ..tail]] }"),
        cons(int(1), cons(int(2), variable("tail")))
    );
    assert_eq!(
        body("pub fn run() -> List(Int) { [1, 2] }"),
        Value::List(attrs(), vec![int(1), int(2)])
    );
}

#[test]
fn block_bindings_preserve_shadowing_and_nontrivial_right_hand_sides() {
    assert_eq!(
        body(
            "pub fn run(value: Int) -> Int { let value = #(value, 2)\n let #(value, other) = value\n value }"
        ),
        destructure(
            binding("value"),
            Value::Tuple(attrs(), vec![variable("value"), int(2)]),
            destructure(
                Pattern::TuplePattern(attrs(), vec![binding("value"), binding("other")]),
                variable("value"),
                variable("value")
            )
        )
    );
}

#[test]
fn nested_blocks_keep_their_bindings_and_discarded_expressions_in_order() {
    assert_eq!(
        body(
            "pub fn run(value: Int) -> Int { #(value, 0)\n let result = { let inner = value\n inner }\n result }"
        ),
        destructure(
            Pattern::WildcardPattern(attrs()),
            Value::Tuple(attrs(), vec![variable("value"), int(0)]),
            destructure(
                binding("result"),
                destructure(binding("inner"), variable("value"), variable("inner")),
                variable("result")
            )
        )
    );
}

#[test]
fn constructor_and_nested_tuple_assignment_patterns_preserve_bindings() {
    assert_eq!(
        body(
            "pub type Box { Box(#(#(Int, Int), Int)) }\npub fn run(box: Box) -> Int { let Box(#(#(head, tail), other)) = box\n head }"
        ),
        destructure(
            Pattern::ConstructorPattern(
                attrs(),
                FQName {
                    package_path: PackageName::parse("example/structural").into(),
                    module_path: ModuleName::parse("main").into(),
                    local_name: Name::from("Box"),
                },
                vec![Pattern::TuplePattern(
                    attrs(),
                    vec![
                        Pattern::TuplePattern(attrs(), vec![binding("head"), binding("tail")]),
                        binding("other")
                    ]
                )]
            ),
            variable("box"),
            variable("head")
        )
    );
}

#[test]
fn unsupported_expressions_cannot_hide_in_preserved_structures() {
    for source in [
        "pub fn run() { todo\n 1 }",
        "pub fn run() { let discarded = { todo }\n 1 }",
        "pub fn run() { [1, ..{ todo }] }",
        "pub fn run() { let x = 1\n use y <- todo\n x }",
    ] {
        let result = compile(source);
        assert!(!result.success, "{source}");
        assert!(result.ir.is_none());
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.code.as_deref() == Some("GLEAM_UNSUPPORTED_VALUE")),
            "{:?}",
            result.diagnostics
        );
    }
}

#[test]
fn a_final_assignment_returns_its_right_hand_side_without_losing_its_pattern() {
    let pattern = Pattern::TuplePattern(attrs(), vec![binding("left"), binding("right")]);
    assert_eq!(
        body("pub fn run() -> #(Int, Int) { let #(left, right) = #(1, 2) }"),
        destructure(
            Pattern::AsPattern(
                attrs(),
                Box::new(pattern),
                Name::from("morphir_block_result")
            ),
            Value::Tuple(attrs(), vec![int(1), int(2)]),
            variable("morphir_block_result")
        )
    );
}

#[test]
fn final_assignment_alias_avoids_pattern_binders_and_evaluates_rhs_once() {
    let pattern = Pattern::TuplePattern(
        attrs(),
        vec![
            binding("morphir_block_result"),
            binding("morphir_block_result_1"),
        ],
    );
    assert_eq!(
        body(
            "pub fn run(make: fn(Int) -> #(Int, Int)) -> #(Int, Int) { let #(morphir_block_result, morphir_block_result_1) = make(42) }"
        ),
        destructure(
            Pattern::AsPattern(
                attrs(),
                Box::new(pattern),
                Name::from("morphir_block_result_2")
            ),
            Value::Apply(attrs(), Box::new(variable("make")), Box::new(int(42))),
            variable("morphir_block_result_2")
        )
    );
}
