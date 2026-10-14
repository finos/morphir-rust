//! Consumer-specific checks that typed Rust roundtrip equality cannot prove.
use super::ProjectionError;
use morphir_core::{
    ir::v4,
    traversal::{
        IrCursor,
        v4::{V4Visitor, walk_definition},
    },
};

struct ParameterOrder(Result<(), ProjectionError>);

impl V4Visitor for ParameterOrder {
    fn visit_definition(&mut self, cursor: &mut IrCursor, value: &v4::ValueDefinition) {
        if !value.input_types.keys().is_sorted() {
            self.0 = Err(ProjectionError::ParameterOrder);
        }
        if self.0.is_ok() {
            walk_definition(self, cursor, value);
        }
    }
}

pub(super) fn check_parameter_order(file: &v4::IRFile) -> Result<(), ProjectionError> {
    let v4::Distribution::Library(library) = &file.distribution else {
        return Err(ProjectionError::UnsupportedSemantics);
    };
    let mut order = ParameterOrder(Ok(()));
    for dependency in library.dependencies.values() {
        for module in dependency.modules.values() {
            for specification in module.values.values() {
                if !specification.value.inputs.keys().is_sorted() {
                    return Err(ProjectionError::ParameterOrder);
                }
            }
        }
    }
    for module in library.def.modules.values() {
        for definition in module.value.values.values() {
            order.visit_definition(&mut IrCursor::root(), &definition.value.value);
        }
    }
    order.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn a_library_with_dependency_parameters(names: [&str; 2]) -> v4::IRFile {
        let mut inputs = serde_json::Map::new();
        for name in names {
            inputs.insert(name.into(), json!("morphir/SDK:basics#bool"));
        }
        let document = json!({
            "formatVersion": "4.1.0", "distribution": {"Library": {
                "packageName": "example/consumer",
                "dependencies": {"example/provider": {"modules": {"api": {
                    "types": {}, "values": {"choose": {"inputs": inputs,
                        "output": "morphir/SDK:basics#bool"}}
                }}}}, "def": {"modules": {}}
            }}
        });
        morphir_core::ir::json::read_ir_file(&document.to_string())
            .unwrap()
            .0
    }

    #[test]
    fn dependency_signature_order_is_checked_even_without_local_definitions() {
        let native = a_library_with_dependency_parameters(["z", "a"]);
        assert!(matches!(
            check_parameter_order(&native),
            Err(ProjectionError::ParameterOrder)
        ));
    }

    #[test]
    fn dependency_signatures_with_lexical_parameters_remain_supported() {
        let native = a_library_with_dependency_parameters(["a", "z"]);
        assert!(check_parameter_order(&native).is_ok());
    }
}
