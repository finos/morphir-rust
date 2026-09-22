//! Closed preflight inputs, fresh snapshots and callback ordering.
use morphir_package::local_registry::assurance::*;
use serde_json::{Value, json};
use std::sync::Arc;

fn selection(mode: &str) -> Value {
    json!({"profile":"restore-filesystem-assurance","profileVersion":"0.1.0-draft.1","mode":mode})
}
fn identity() -> Value {
    json!({"id":"test-host","version":"test-version"})
}
fn qualification(mode: &str) -> Value {
    json!({"mode":mode,"environment":{"runtime":{"name":"test","version":"1"},"os":{"name":"test","version":"1"},"architecture":"test","filesystem":"test","assumptions":["trusted directories"]},"evidence":["test-only"]})
}
fn provider(modes: &[&str]) -> Value {
    json!({"identity":identity(),"qualification":{"kind":"qualified","entries":modes.iter().map(|m|qualification(m)).collect::<Vec<_>>()}})
}
fn selected(mode: &str) -> Value {
    json!({"kind":"selected","context":{"selection":selection(mode),"provider":identity(),"qualification":qualification(mode)}})
}
fn request() -> Value {
    json!({"selection":selection("portable"),"provider":provider(&["portable","hardened"])})
}
fn run<F: std::future::Future>(future: F) -> F::Output {
    struct Wake(std::thread::Thread);
    impl std::task::Wake for Wake {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = std::task::Waker::from(Arc::new(Wake(std::thread::current())));
    let mut context = std::task::Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        match future.as_mut().poll(&mut context) {
            std::task::Poll::Ready(value) => return value,
            std::task::Poll::Pending => std::thread::park(),
        }
    }
}
#[test]
fn explicit_modes_preserve_exact_receipt_and_invoke_callback_once() {
    for mode in ["portable", "hardened"] {
        let mut accesses = 0;
        let result = with_restore_assurance(
            &selection(mode),
            &provider(&["portable", "hardened"]),
            |context| {
                accesses += 1;
                assert_eq!(
                    serde_json::to_value(context).unwrap(),
                    selected(mode)["context"]
                );
                Ok::<_, ()>(17)
            },
        )
        .unwrap();
        assert_eq!(accesses, 1);
        assert_eq!(
            serde_json::to_value(result).unwrap(),
            json!({"kind":"executed","receipt":selected(mode),"value":17})
        );
    }
}
#[test]
fn rejection_never_accesses_packages_or_infers_another_mode() {
    for mode in ["portable", "hardened"] {
        for (host, reason) in [
            (
                json!({"identity":identity(),"qualification":{"kind":"unqualified"}}),
                "provider-unqualified",
            ),
            (
                provider(&[if mode == "portable" {
                    "hardened"
                } else {
                    "portable"
                }]),
                "mode-unavailable",
            ),
        ] {
            let result = with_restore_assurance(&selection(mode), &host, |_| -> Result<(), ()> {
                panic!("callback must not run")
            })
            .unwrap();
            assert_eq!(
                serde_json::to_value(result).unwrap(),
                json!({"kind":"rejected","receipt":{"kind":"rejected","selection":selection(mode),"provider":identity(),"reason":reason}})
            );
        }
    }
    for os in ["Linux", "macOS", "Windows"] {
        let mut host = provider(&["portable"]);
        host["qualification"]["entries"][0]["environment"]["os"]["name"] = json!(os);
        let result = with_restore_assurance(&selection("hardened"), &host, |_| -> Result<(), ()> {
            panic!("OS does not grant support")
        })
        .unwrap();
        assert_eq!(serde_json::to_value(result).unwrap()["kind"], "rejected");
    }
}
#[test]
fn async_preflight_owns_inputs_and_propagates_the_original_error_once() {
    let mut input = request();
    let mut accesses = 0;
    let failure = Arc::new("state-commit failure");
    let original = failure.clone();
    let execution =
        with_restore_assurance_async(&input["selection"], &input["provider"], |context| {
            accesses += 1;
            async move {
                assert_eq!(
                    serde_json::to_value(context).unwrap(),
                    selected("portable")["context"]
                );
                Err::<(), _>(failure)
            }
        });
    input["selection"]["mode"] = json!("hardened");
    input["provider"] = Value::Null;
    match run(execution).unwrap_err() {
        AssuranceExecutionError::Callback(error) => assert!(Arc::ptr_eq(&error, &original)),
        other => panic!("{other:?}"),
    }
    assert_eq!(accesses, 1);
}
#[test]
fn synchronous_callback_error_is_unchanged() {
    let original = Arc::new("lock failure");
    let failure = original.clone();
    let mut accesses = 0;
    let result = with_restore_assurance(&selection("portable"), &provider(&["portable"]), |_| {
        accesses += 1;
        Err::<(), _>(failure)
    });
    match result.unwrap_err() {
        AssuranceExecutionError::Callback(error) => assert!(Arc::ptr_eq(&error, &original)),
        other => panic!("{other:?}"),
    }
    assert_eq!(accesses, 1);
}
fn mutations(value: &Value) -> Vec<Value> {
    let mut variants = vec![Value::Null];
    match value {
        Value::Object(fields) => {
            let mut extra = value.clone();
            extra["extra"] = json!(true);
            variants.push(extra);
            for (key, child) in fields {
                let mut missing = value.clone();
                missing.as_object_mut().unwrap().remove(key);
                variants.push(missing);
                for mutation in mutations(child) {
                    let mut copy = value.clone();
                    copy[key] = mutation;
                    variants.push(copy);
                }
            }
        }
        Value::Array(items) => {
            variants.push(json!({}));
            variants.push(json!([]));
            for (i, child) in items.iter().enumerate() {
                for mutation in mutations(child) {
                    let mut copy = value.clone();
                    copy[i] = mutation;
                    variants.push(copy);
                }
            }
        }
        Value::String(_) => {
            variants.extend([json!(""), json!(1), json!(true), json!([]), json!({})])
        }
        _ => {}
    };
    variants
}
#[test]
fn every_request_and_receipt_field_is_required_closed_and_typed() {
    let inputs = [
        request(),
        json!({"selection":selection("portable"),"provider":{"identity":identity(),"qualification":{"kind":"unqualified"}}}),
    ];
    for input in inputs {
        assert_eq!(
            serde_json::to_value(parse_restore_assurance_request(&input).unwrap()).unwrap(),
            input
        );
        for bad in mutations(&input) {
            assert!(
                parse_restore_assurance_request(&bad).is_err(),
                "accepted {bad}"
            );
            if let Some(fields) = bad.as_object().filter(|o| {
                o.keys()
                    .all(|key| matches!(key.as_str(), "selection" | "provider"))
            }) {
                assert!(
                    with_restore_assurance(
                        fields.get("selection").unwrap_or(&Value::Null),
                        fields.get("provider").unwrap_or(&Value::Null),
                        |_| -> Result<(), ()> { panic!("invalid input callback") }
                    )
                    .is_err()
                );
            }
        }
    }
    for input in [
        selected("portable"),
        selected("hardened"),
        json!({"kind":"rejected","selection":selection("portable"),"provider":identity(),"reason":"mode-unavailable"}),
    ] {
        assert_eq!(
            serde_json::to_value(parse_restore_assurance_receipt(&input).unwrap()).unwrap(),
            input
        );
        for bad in mutations(&input) {
            assert!(
                parse_restore_assurance_receipt(&bad).is_err(),
                "accepted {bad}"
            );
        }
    }
}
#[test]
fn mismatched_modes_unknown_versions_and_duplicate_attestations_are_rejected() {
    let mut receipt = selected("portable");
    receipt["context"]["qualification"]["mode"] = json!("hardened");
    assert!(parse_restore_assurance_receipt(&receipt).is_err());
    let host = provider(&["portable", "portable"]);
    assert!(parse_restore_assurance_provider(&host).is_err());
    for key in ["profile", "profileVersion", "mode"] {
        let mut input = selection("portable");
        input[key] = json!("future");
        assert!(parse_restore_assurance_selection(&input).is_err());
    }
    assert!(
        with_restore_assurance(
            &selected("portable"),
            &provider(&["portable"]),
            |_| -> Result<(), ()> { panic!("receipt cannot authorize access") }
        )
        .is_err()
    );
}

#[test]
fn async_rejection_and_invalid_inputs_never_invoke_callback() {
    let denied = json!({"identity":identity(),"qualification":{"kind":"unqualified"}});
    let rejected = run(with_restore_assurance_async(
        &selection("portable"),
        &denied,
        |_| async {
            panic!("no access");
            #[allow(unreachable_code)]
            Ok::<(), ()>(())
        },
    ))
    .unwrap();
    assert_eq!(serde_json::to_value(rejected).unwrap()["kind"], "rejected");
    assert!(
        run(with_restore_assurance_async(
            &Value::Null,
            &denied,
            |_| async {
                panic!("invalid input");
                #[allow(unreachable_code)]
                Ok::<(), ()>(())
            }
        ))
        .is_err()
    );
}
