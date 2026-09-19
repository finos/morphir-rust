//! Reject incomplete nodes that the upstream parser retains for editor recovery.
//!
//! Walk even unsupported expressions so type-only compilation still requires
//! syntactically complete input. This checks AST structure, not source grammar.
use super::*;

pub(super) fn validate(module: &gleam::UntypedModule, source: &str) -> Result<(), ParseError> {
    for definition in &module.definitions {
        match &definition.definition {
            gleam::Definition::Function(function) => {
                if function.body_start.is_none()
                    && function.external_erlang.is_none()
                    && function.external_javascript.is_none()
                {
                    let location = function
                        .return_annotation
                        .as_ref()
                        .map_or(function.full_location(), |annotation| {
                            function.full_location().merge(&annotation.location())
                        });
                    return Err(located_error("Missing function body", location, source));
                }
                arguments(&function.arguments, source)?;
                annotation(function.return_annotation.as_ref(), source)?;
                statements(&function.body, source)?;
            }
            gleam::Definition::ModuleConstant(value) => constant(&value.value, source)?,
            _ => {}
        }
    }
    Ok(())
}

fn annotation(value: Option<&gleam::TypeAst>, source: &str) -> Result<(), ParseError> {
    value
        .map(|value| type_expr(value, source))
        .transpose()
        .map(|_| ())
}

fn arguments(arguments: &[gleam::UntypedArg], source: &str) -> Result<(), ParseError> {
    arguments
        .iter()
        .try_for_each(|argument| annotation(argument.annotation.as_ref(), source))
}

fn statements(values: &[gleam::UntypedStatement], source: &str) -> Result<(), ParseError> {
    for statement in values {
        match statement {
            gleam::Statement::Expression(value) => expr(value, source)?,
            gleam::Statement::Assignment(assignment) => {
                annotation(assignment.annotation.as_ref(), source)?;
                expr(&assignment.value, source)?;
                if let gleam::AssignmentKind::Assert {
                    message: Some(message),
                    ..
                } = &assignment.kind
                {
                    expr(message, source)?;
                }
            }
            gleam::Statement::Use(use_) => {
                expr(&use_.call, source)?;
                for assignment in &use_.assignments {
                    annotation(assignment.annotation.as_ref(), source)?;
                }
            }
            gleam::Statement::Assert(assertion) => {
                expr(&assertion.value, source)?;
                if let Some(message) = &assertion.message {
                    expr(message, source)?;
                }
            }
        }
    }
    Ok(())
}

fn expr(value: &gleam::UntypedExpr, source: &str) -> Result<(), ParseError> {
    use gleam::UntypedExpr as G;
    match value {
        G::Block {
            statements: body, ..
        } => statements(body, source)?,
        G::Fn {
            arguments: args,
            body,
            return_annotation,
            ..
        } => {
            arguments(args, source)?;
            annotation(return_annotation.as_ref(), source)?;
            statements(body, source)?;
        }
        G::List { elements, tail, .. } => {
            for element in elements {
                expr(element, source)?;
            }
            if let Some(tail) = tail {
                expr(tail, source)?;
            }
        }
        G::Tuple { elements, .. } => {
            for element in elements {
                expr(element, source)?;
            }
        }
        G::Call { fun, arguments, .. } => {
            expr(fun, source)?;
            for argument in arguments {
                expr(&argument.value, source)?;
            }
        }
        G::BinOp { left, right, .. } => {
            expr(left, source)?;
            expr(right, source)?;
        }
        G::PipeLine { expressions } => {
            for value in expressions {
                expr(value, source)?;
            }
        }
        G::Case {
            subjects,
            clauses,
            location,
        } => {
            let clauses = clauses
                .as_ref()
                .ok_or_else(|| located_error("Missing case expression body", *location, source))?;
            for subject in subjects {
                expr(subject, source)?;
            }
            for clause in clauses {
                expr(&clause.then, source)?;
            }
        }
        G::FieldAccess {
            label,
            container,
            location,
            ..
        } => {
            if label.is_empty() {
                return Err(located_error("Missing field name", *location, source));
            }
            expr(container, source)?;
        }
        G::TupleIndex { tuple, .. } => expr(tuple, source)?,
        G::Todo {
            kind: gleam::TodoKind::IncompleteUse,
            location,
            ..
        } => {
            return Err(located_error(
                "Incomplete use expression",
                *location,
                source,
            ));
        }
        G::Todo { message, .. } | G::Panic { message, .. } => {
            if let Some(message) = message {
                expr(message, source)?;
            }
        }
        G::Echo {
            expression,
            message,
            ..
        } => {
            if let Some(value) = expression {
                expr(value, source)?;
            }
            if let Some(message) = message {
                expr(message, source)?;
            }
        }
        G::BitArray { segments, .. } => {
            for segment in segments {
                expr(&segment.value, source)?;
                for value in segment
                    .options
                    .iter()
                    .filter_map(gleam::BitArrayOption::value)
                {
                    expr(value, source)?;
                }
            }
        }
        G::RecordUpdate {
            constructor,
            record,
            arguments,
            ..
        } => {
            expr(constructor, source)?;
            expr(&record.base, source)?;
            for argument in arguments {
                expr(&argument.value, source)?;
            }
        }
        G::NegateBool { value, .. } | G::NegateInt { value, .. } => expr(value, source)?,
        G::Int { .. } | G::Float { .. } | G::String { .. } | G::Var { .. } => {}
    }
    Ok(())
}

fn constant(value: &gleam::UntypedConstant, source: &str) -> Result<(), ParseError> {
    use gleam::Constant as G;
    match value {
        G::Tuple { elements, .. } => {
            for value in elements {
                constant(value, source)?;
            }
        }
        G::List { elements, tail, .. } => {
            for value in elements {
                constant(value, source)?;
            }
            if let Some(value) = tail {
                constant(value, source)?;
            }
        }
        G::Record { arguments, .. } => {
            for argument in arguments.iter().flatten() {
                constant(&argument.value, source)?;
            }
        }
        G::RecordUpdate {
            record, arguments, ..
        } => {
            constant(&record.base, source)?;
            for argument in arguments {
                constant(&argument.value, source)?;
            }
        }
        G::BitArray { segments, .. } => {
            for segment in segments {
                constant(&segment.value, source)?;
                for value in segment
                    .options
                    .iter()
                    .filter_map(gleam::BitArrayOption::value)
                {
                    constant(value, source)?;
                }
            }
        }
        G::StringConcatenation { left, right, .. } => {
            constant(left, source)?;
            constant(right, source)?;
        }
        G::Todo { message, .. } => {
            if let Some(value) = message {
                constant(value, source)?;
            }
        }
        G::Invalid { location, .. } => {
            return Err(located_error(
                "Invalid constant expression",
                *location,
                source,
            ));
        }
        G::Int { .. } | G::Float { .. } | G::String { .. } | G::Var { .. } => {}
    }
    Ok(())
}
