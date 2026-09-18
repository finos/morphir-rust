use morphir_extension_sdk::{
    CompileOptions, CompilePackage, CompileRequest, Frontend, SourceDocument,
};
use morphir_rust_binding::RustExtension;

fn compile(source: &str, version: &str) -> morphir_extension_sdk::CompileResult {
    RustExtension
        .compile(CompileRequest {
            language_id: "rust".into(),
            documents: vec![SourceDocument {
                uri: "file:///models.rs".into(),
                language_id: "rust".into(),
                text: source.into(),
                ..Default::default()
            }],
            package: CompilePackage {
                name: "acme/example".into(),
                exposed_modules: Some(vec!["Models".into()]),
            },
            options: CompileOptions {
                ir_version: version.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .unwrap()
}

#[test]
fn named_calls_function_pointers_and_typed_lambdas_compile() {
    for version in ["3", "4"] {
        for source in [
            "fn use_it(x:i64)->i64{identity(x)} fn identity<T>(x:T)->T{x}",
            "fn id(x:i64)->i64{x} fn invoke(f:fn(i64)->i64,x:i64)->i64{f(x)} fn run(x:i64)->i64{invoke(id,x)}",
            "fn f(limit:i64,x:i64)->bool{let above=|n:i64| n>limit; above(x)}",
            "fn f(x:i64)->(i64,i64){let id=|n:i64|n; (id(x),id(x))}",
            "fn constant()->i64{42} fn f()->i64{constant()} fn pointer()->fn()->i64{constant}",
            "fn first(a:i64,b:bool)->i64{a} fn f()->i64{first(3,true)}",
            "fn f()->fn(i64)->i64{|n:i64| -> i64 {n}}",
            "fn identity<T>(x:T)->T{x} fn f(x:i64)->i64{identity::<i64>(x)}",
            "fn f(x:i64)->i64{let id=|n:i64|n; let id=|n:bool|n; if id(true){x}else{0}}",
            "type F = fn(i64)->bool; fn f(g:F,x:i64)->bool{g(x)}",
            "fn f(x:i64)->bool{let both=|a:i64,b:i64|a>b;both(x,0)}",
            "fn f(x:i64)->i64{let thunk=||x;thunk()}",
            "fn f(x:i64)->i64{let id=move |n:i64|x;id(1)}",
        ] {
            let result = compile(source, version);
            assert!(
                result.success,
                "v{version} {source}: {:?}",
                result.diagnostics
            );
        }
    }
}

#[test]
fn invalid_calls_captures_and_coercions_are_rejected() {
    for version in ["3", "4"] {
        for source in [
            "fn id(x:i64)->i64{x} fn f()->i64{id(true)}",
            "fn id(x:i64)->i64{x} fn f()->i64{id()}",
            "fn f()->i64{missing(1)}",
            "fn f(x:i64)->i64{f(x)}",
            "fn f(x:i64)->i64{g(x)} fn g(x:i64)->i64{f(x)}",
            "fn f()->fn(i64)->i64{|n|n}",
            "fn f()->fn(i64)->i64{|n:i64|->bool{n}}",
            "fn f(x:String)->String{let g=||x;g()}",
            "fn f(x:i64)->i64{let g=||{x=1;x};g()}",
            "fn f(x:i64)->fn(i64)->i64{|n:i64|x}",
            "fn use_it(g:fn(i64)->i64)->i64{g(1)} fn f(x:i64)->i64{use_it(|n:i64|x)}",
            "fn id(x:i64)->i64{x} fn f()->i64{let id=1;id(1)}",
            "fn f(x:i64)->i64{let g=|a:i64,b:i64|a;g(x)}",
            "fn f(g:fn(i64,i64)->i64)->i64{g(1)}",
            "fn f(g:fn(Box<i64>)->i64)->i64{0}",
        ] {
            let result = compile(source, version);
            assert!(!result.success, "v{version} accepted {source}");
            assert!(result.ir.is_none());
        }
    }
}

#[test]
fn contextual_generics_and_callable_shapes_are_preserved() {
    for version in ["3", "4"] {
        for source in [
            "fn id<T>(x:T)->T{x} fn f()->fn(i64)->i64{id}",
            "fn id<T>(x:T)->T{x} fn f(x:i64)->i64{let g:fn(i64)->i64=id;g(x)}",
            "fn id<T>(x:T)->T{x} fn invoke(g:fn(i64)->i64,x:i64)->i64{g(x)} fn f(x:i64)->i64{invoke(id,x)}",
            "fn id<T>(x:T)->T{x} fn f<T>(x:T)->T{id(x)}",
            "fn f(x:i64)->i64{let g=|(a,b):(i64,bool)|if b{a}else{x};g((1,true))}",
            "fn f(x:i64)->i64{let g=|_:i64|x;g(1)}",
            "type F<T>=fn(T)->T; fn f(g:F<i64>,x:i64)->i64{g(x)}",
            "fn f(g:fn(i64)->i64,x:i64)->i64{match (g,x){(h,n)=>h(n)}}",
        ] {
            let result = compile(source, version);
            assert!(
                result.success,
                "v{version} {source}: {:?}",
                result.diagnostics
            );
        }
    }
}

#[test]
fn callable_identity_and_rust_arity_are_not_erased() {
    for version in ["3", "4"] {
        for source in [
            "fn f()->fn(i64)->fn(i64)->i64{|a:i64,b:i64|a}",
            "fn f(g:fn(i64,i64)->i64)->i64{g(1)(2)}",
            "fn f(x:i64)->(fn(i64)->i64,){(|n:i64|x,)}",
            "fn f(x:i64,b:bool)->fn(i64)->i64{if b{|n:i64|x}else{|n:i64|n}}",
            "fn f(x:i64)->i64{let g=|n:i64,n:i64|n;g(x,x)}",
            "fn f<T>(x:T,y:i64)->T{id(x,y)} fn id<U>(x:U,y:U)->U{x}",
            "fn f(x:Option<i64>)->i64{let g=||match x{Some(n)=>n,None=>0};g()}",
            "fn f(g:fn()->i64)->i64{let c=||g();c()}",
            "fn f(x:i64)->i64{let g=|a:String,b:i64|match a{_=>b};g(\"x\",x)}",
        ] {
            let result = compile(source, version);
            assert!(!result.success, "v{version} accepted {source}");
        }
    }
}

#[test]
fn generic_aliases_preserve_source_callable_arity() {
    for version in ["3", "4"] {
        for source in [
            "type Identity<T>=T; type Pred=fn(i64)->bool; fn f(p:Identity<Pred>,x:i64)->bool{p(x)}",
            "type Pair<T>=(T,bool); fn f(pair:Pair<fn(i64)->i64>,x:i64)->i64{match pair{(g,_)=>g(x)}}",
            "type A<T>=fn(T)->T; fn f(g:A<i64>,x:i64)->i64{g(x)}",
            "type A<T>=T; fn f<T>(g:A<fn(T)->T>,x:T)->T{g(x)}",
        ] {
            let result = compile(source, version);
            assert!(
                result.success,
                "v{version} {source}: {:?}",
                result.diagnostics
            );
        }
        for source in [
            "type A<T>=T; fn f()->A<fn(i64)->fn(i64)->i64>{|a:i64,b:i64|a}",
            "type A<T>=T; fn f(x:i64)->A<fn(i64)->i64>{|a:i64|x}",
        ] {
            assert!(!compile(source, version).success, "accepted {source}");
        }
    }
}

#[test]
fn generic_calls_can_transport_callable_values() {
    for version in ["3", "4"] {
        for source in [
            "fn id<T>(x:T)->T{x} fn same(x:i64)->i64{x} fn f(x:i64)->i64{id(same)(x)}",
            "fn id<T>(x:T)->T{x} fn f(g:fn(i64)->i64,x:i64)->i64{id(g)(x)}",
            "fn id<T>(x:T)->T{x} fn f(g:fn(i64,i64)->i64,x:i64)->i64{id(g)(x,x)}",
            "fn id<T>(x:T)->T{x} fn f(x:i64)->i64{let g=|n:i64|n;let h=id(g);h(x)}",
            "fn id<T>(x:T)->T{x} fn f()->(fn(i64)->i64,){(id,)}",
            "fn f()->fn()->fn(i64)->i64 {|| |x:i64|x}",
        ] {
            let result = compile(source, version);
            assert!(
                result.success,
                "v{version} {source}: {:?}",
                result.diagnostics
            );
        }
        for source in [
            "fn id<T>(x:T)->T{x} fn f(x:i64)->fn(i64)->i64{id(|n:i64|x)}",
            "fn id<T>(x:T)->T{x} fn f(g:fn(i64,i64)->i64,x:i64)->i64{id(g)(x)(x)}",
        ] {
            assert!(!compile(source, version).success, "accepted {source}");
        }
    }
}

#[test]
fn explicit_callable_generic_arity_and_lambda_result_context() {
    for version in ["3", "4"] {
        for source in [
            "fn id<T>(x:T)->T{x} fn f(g:fn(i64,i64)->i64,x:i64)->i64{let h=id::<fn(i64,i64)->i64>;h(g)(x,x)}",
            "fn id<T>(x:T)->T{x} fn f()->fn(i64)->i64{let g=||->fn(i64)->i64{id};g()}",
        ] {
            let result = compile(source, version);
            assert!(
                result.success,
                "v{version} {source}: {:?}",
                result.diagnostics
            );
        }
    }
}

#[test]
fn explicit_generic_types_reject_different_source_callable_arity() {
    for version in ["3", "4"] {
        let source = "fn id<T>(x:T)->T{x} fn f(g:fn(i64)->fn(i64)->i64,x:i64)->i64{let h=id::<fn(i64,i64)->i64>;h(g)(x)(x)}";
        assert!(
            !compile(source, version).success,
            "v{version} accepted {source}"
        );
    }
}

#[test]
fn tuples_of_copy_data_and_function_values_can_be_reused() {
    for version in ["3", "4"] {
        let source = "fn f(g:fn(i64)->i64,x:i64)->i64{let p=(g,x);let a=match p{(h,n)=>h(n)};match p{(h,n)=>h(n)}}";
        let result = compile(source, version);
        assert!(result.success, "v{version}: {:?}", result.diagnostics);
    }
}

#[test]
fn explicit_generic_call_preserves_source_arity_and_pointer_coercion() {
    for version in ["3", "4"] {
        let bad = "fn id<T>(x:T)->T{x} fn f(g:fn(i64)->fn(i64)->i64,x:i64)->i64{id::<fn(i64,i64)->i64>(g)(x)(x)}";
        assert!(!compile(bad, version).success, "v{version} accepted {bad}");
        let good = "fn id<T>(x:T)->T{x} fn f(x:i64)->i64{id::<fn(i64)->i64>(|n:i64|n)(x)}";
        let result = compile(good, version);
        assert!(result.success, "v{version}: {:?}", result.diagnostics);
    }
}

#[test]
fn annotated_local_closures_become_function_pointers() {
    for version in ["3", "4"] {
        let source = "fn first<T>(x:T,y:T)->T{x} fn f()->i64{let a:fn(i64)->i64=|x:i64|x;let b:fn(i64)->i64=|x:i64|x;first(a,b)(1)}";
        let result = compile(source, version);
        assert!(result.success, "v{version}: {:?}", result.diagnostics);
    }
}

#[test]
fn explicit_lambda_results_coerce_nested_closures_to_pointers() {
    for version in ["3", "4"] {
        let source = "fn first<T>(x:T,y:T)->T{x} fn f()->i64{let maker=||->fn(i64)->i64{|x:i64|x};let a=maker();let b:fn(i64)->i64=|x:i64|x;first(a,b)(1)}";
        let result = compile(source, version);
        assert!(result.success, "v{version}: {:?}", result.diagnostics);
    }
}

#[test]
fn generic_nullary_function_values_are_instantiated() {
    for version in ["3", "4"] {
        let source = "fn maker<T>()->fn(T)->T{|x:T|x} fn f()->fn()->fn(i64)->i64{maker::<i64>}";
        let result = compile(source, version);
        assert!(result.success, "v{version}: {:?}", result.diagnostics);
    }
}

#[test]
fn distinct_named_function_items_require_pointer_coercion_before_generic_unification() {
    for version in ["3", "4"] {
        let prelude = "fn first<T>(x:T,y:T)->T{x} fn a(x:i64)->i64{x} fn b(x:i64)->i64{x}";
        let invalid = format!("{prelude} fn f()->i64{{first(a,b)(1)}}");
        assert!(
            !compile(&invalid, version).success,
            "v{version} accepted {invalid}"
        );
        for body in [
            "first(a,a)(1)",
            "first::<fn(i64)->i64>(a,b)(1)",
            "{let x:fn(i64)->i64=a;let y:fn(i64)->i64=b;first(x,y)(1)}",
        ] {
            let source = format!("{prelude} fn f()->i64{{{body}}}");
            let result = compile(&source, version);
            assert!(result.success, "v{version}: {:?}", result.diagnostics);
        }
    }
}

#[test]
fn contextual_generic_results_coerce_function_items_and_closures() {
    for version in ["3", "4"] {
        for source in [
            "fn first<T>(a:T,b:T)->T{a} fn a(x:i64)->i64{x} fn b(x:i64)->i64{x} fn f()->fn(i64)->i64{first(a,b)}",
            "fn first<T>(a:T,b:T)->T{a} fn f()->fn(i64)->i64{first(|x:i64|x,|x:i64|x)}",
            "fn first<T>(a:T,b:T)->T{a} fn a(x:i64,y:i64)->i64{x} fn b(x:i64,y:i64)->i64{y} fn f()->fn(i64,i64)->i64{first(a,b)}",
        ] {
            let result = compile(source, version);
            assert!(
                result.success,
                "v{version} {source}: {:?}",
                result.diagnostics
            );
        }
        for source in [
            "fn first<T>(a:T,b:T)->T{a} fn f(x:i64)->fn(i64)->i64{first(|n:i64|x,|n:i64|n)}",
            "fn first<T>(a:T,b:T)->T{a} fn a(x:i64,y:i64)->i64{x} fn b(x:i64,y:i64)->i64{y} fn f()->fn(i64)->fn(i64)->i64{first(a,b)}",
        ] {
            assert!(
                !compile(source, version).success,
                "v{version} accepted {source}"
            );
        }
    }
}
