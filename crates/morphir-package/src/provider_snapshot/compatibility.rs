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
    for module in library.def.modules.values() {
        for definition in module.value.values.values() {
            order.visit_definition(&mut IrCursor::root(), &definition.value.value);
        }
    }
    order.0
}
