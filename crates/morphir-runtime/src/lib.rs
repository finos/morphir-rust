//! Pure evaluation of classic Morphir IR values.

use morphir_core::ir::classic as ir;
use std::collections::HashMap;
use std::rc::Rc;

type Expression = ir::Value<ir::Attrs, ir::Type<ir::Attrs>>;
type Definition = ir::ValueDefinition<ir::Attrs, ir::Type<ir::Attrs>>;
type Environment = HashMap<ir::Name, Evaluated>;
type Library<'a> = (
    &'a ir::Path,
    &'a [(ir::Path, ir::PackageSpecification<ir::Attrs>)],
    &'a ir::PackageDefinition<ir::Attrs, ir::Type<ir::Attrs>>,
);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeValue {
    Integer(i64),
    Boolean(bool),
    String(String),
    Unit,
    List(Vec<Self>),
    Tuple(Vec<Self>),
    Constructor(ir::FQName, Vec<Self>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EvaluationLimits {
    pub fuel: u64,
    pub max_call_depth: usize,
}

impl Default for EvaluationLimits {
    fn default() -> Self {
        Self {
            fuel: 100_000,
            max_call_depth: 256,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvaluationError {
    UnsupportedVersion(u32),
    MissingEntrypoint(ir::FQName),
    MissingDependency(ir::Path),
    UnknownReference(ir::FQName),
    UnknownVariable(ir::Name),
    ArityMismatch { expected: usize, actual: usize },
    TypeMismatch(&'static str),
    NonExhaustivePattern,
    FuelExhausted,
    CallDepthExceeded,
    IntegerOverflow,
    UnsupportedExpression(&'static str),
}

#[derive(Clone)]
enum Callable {
    User(ir::FQName),
    Local(Box<Definition>, Box<Environment>),
    Recursive(
        ir::Name,
        Rc<HashMap<ir::Name, Definition>>,
        Box<Environment>,
    ),
    Lambda(
        ir::Pattern<ir::Type<ir::Attrs>>,
        Box<Expression>,
        Box<Environment>,
    ),
    Constructor(ir::FQName),
    Add,
    Equal,
}

#[derive(Clone)]
enum Evaluated {
    Data(RuntimeValue),
    Function(Callable, Vec<RuntimeValue>),
}

struct Evaluator<'a> {
    distribution: &'a ir::Distribution,
    remaining_fuel: u64,
    max_call_depth: usize,
}

/// Evaluate one top-level V3 value with typed runtime arguments.
pub fn evaluate_v3(
    distribution: &ir::Distribution,
    entrypoint: &ir::FQName,
    arguments: Vec<RuntimeValue>,
    limits: EvaluationLimits,
) -> Result<RuntimeValue, EvaluationError> {
    if distribution.format_version != 3 {
        return Err(EvaluationError::UnsupportedVersion(
            distribution.format_version,
        ));
    }
    let mut evaluator = Evaluator {
        distribution,
        remaining_fuel: limits.fuel,
        max_call_depth: limits.max_call_depth,
    };
    let definition = evaluator
        .definition(entrypoint)
        .ok_or_else(|| EvaluationError::MissingEntrypoint(entrypoint.clone()))?;
    if arguments.len() != definition.input_types.len() {
        return Err(EvaluationError::ArityMismatch {
            expected: definition.input_types.len(),
            actual: arguments.len(),
        });
    }
    evaluator.call_user(&definition, arguments, &Environment::new(), 0)
}

impl<'a> Evaluator<'a> {
    fn library(&self) -> Library<'_> {
        let ir::DistributionBody::Library(package, dependencies, definition) =
            &self.distribution.distribution;
        (package, dependencies, definition)
    }

    fn definition(&self, name: &ir::FQName) -> Option<Definition> {
        let (package, _, definition) = self.library();
        if &name.package_path != package {
            return None;
        }
        definition
            .modules
            .iter()
            .find(|module| module.path == name.module_path)?
            .definition
            .value
            .values
            .iter()
            .find(|(local, _)| local == &name.local_name)
            .map(|(_, value)| value.value.value.clone())
    }

    fn constructor_arity(&self, name: &ir::FQName) -> Option<usize> {
        let (package, _, definition) = self.library();
        if &name.package_path != package {
            return None;
        }
        let module = definition
            .modules
            .iter()
            .find(|module| module.path == name.module_path)?;
        module.definition.value.types.iter().find_map(|(_, ty)| {
            let ir::TypeDefinition::Custom(_, constructors) = &ty.value.value else {
                return None;
            };
            constructors
                .value
                .iter()
                .find(|constructor| constructor.name == name.local_name)
                .map(|constructor| constructor.args.len())
        })
    }

    fn tick(&mut self) -> Result<(), EvaluationError> {
        self.remaining_fuel = self
            .remaining_fuel
            .checked_sub(1)
            .ok_or(EvaluationError::FuelExhausted)?;
        Ok(())
    }

    fn call_user(
        &mut self,
        definition: &Definition,
        arguments: Vec<RuntimeValue>,
        captured: &Environment,
        depth: usize,
    ) -> Result<RuntimeValue, EvaluationError> {
        if depth >= self.max_call_depth {
            return Err(EvaluationError::CallDepthExceeded);
        }
        self.tick()?;
        let mut environment = captured.clone();
        environment.extend(
            definition
                .input_types
                .iter()
                .zip(arguments)
                .map(|(parameter, value)| (parameter.name.clone(), Evaluated::Data(value))),
        );
        Self::data(self.eval(&definition.body, &environment, depth + 1)?)
    }

    fn data(value: Evaluated) -> Result<RuntimeValue, EvaluationError> {
        match value {
            Evaluated::Data(value) => Ok(value),
            Evaluated::Function(_, _) => Err(EvaluationError::TypeMismatch(
                "Expected a value, found a function",
            )),
        }
    }

    fn resolve_reference(&self, name: &ir::FQName) -> Result<Evaluated, EvaluationError> {
        if self.definition(name).is_some() {
            return Ok(Evaluated::Function(Callable::User(name.clone()), vec![]));
        }
        let (package, dependencies, _) = self.library();
        if &name.package_path != package {
            if !dependencies
                .iter()
                .any(|(dependency, _)| dependency == &name.package_path)
            {
                return Err(EvaluationError::MissingDependency(
                    name.package_path.clone(),
                ));
            }
            let sdk = ir::Path::new(vec![
                ir::Name::from_str("morphir"),
                ir::Name::from_str("SDK"),
            ]);
            let basics = ir::Path::new(vec![ir::Name::from_str("basics")]);
            if name.package_path == sdk && name.module_path == basics {
                if name.local_name == ir::Name::from_str("add") {
                    return Ok(Evaluated::Function(Callable::Add, vec![]));
                }
                if name.local_name == ir::Name::from_str("equal") {
                    return Ok(Evaluated::Function(Callable::Equal, vec![]));
                }
            }
        }
        Err(EvaluationError::UnknownReference(name.clone()))
    }

    fn apply(
        &mut self,
        function: Evaluated,
        argument: RuntimeValue,
        depth: usize,
    ) -> Result<Evaluated, EvaluationError> {
        let Evaluated::Function(callable, mut arguments) = function else {
            return Err(EvaluationError::TypeMismatch(
                "Application target is not a function",
            ));
        };
        arguments.push(argument);
        let arity = match &callable {
            Callable::User(name) => self
                .definition(name)
                .ok_or_else(|| EvaluationError::UnknownReference(name.clone()))?
                .input_types
                .len(),
            Callable::Local(definition, _) => definition.input_types.len(),
            Callable::Recursive(name, definitions, _) => definitions
                .get(name)
                .ok_or_else(|| EvaluationError::UnknownVariable(name.clone()))?
                .input_types
                .len(),
            Callable::Lambda(..) => 1,
            Callable::Constructor(name) => self
                .constructor_arity(name)
                .ok_or_else(|| EvaluationError::UnknownReference(name.clone()))?,
            Callable::Add | Callable::Equal => 2,
        };
        if arguments.len() < arity {
            return Ok(Evaluated::Function(callable, arguments));
        }
        if arguments.len() > arity {
            return Err(EvaluationError::ArityMismatch {
                expected: arity,
                actual: arguments.len(),
            });
        }
        Ok(Evaluated::Data(match callable {
            Callable::User(name) => {
                let definition = self
                    .definition(&name)
                    .ok_or(EvaluationError::UnknownReference(name))?;
                self.call_user(&definition, arguments, &Environment::new(), depth)?
            }
            Callable::Local(definition, captured) => {
                self.call_user(&definition, arguments, &captured, depth)?
            }
            Callable::Recursive(name, definitions, captured) => {
                let definition = definitions
                    .get(&name)
                    .ok_or_else(|| EvaluationError::UnknownVariable(name.clone()))?;
                let mut recursive_environment = *captured.clone();
                for local in definitions.keys() {
                    recursive_environment.insert(
                        local.clone(),
                        Evaluated::Function(
                            Callable::Recursive(
                                local.clone(),
                                definitions.clone(),
                                captured.clone(),
                            ),
                            vec![],
                        ),
                    );
                }
                self.call_user(definition, arguments, &recursive_environment, depth)?
            }
            Callable::Lambda(pattern, body, captured) => {
                let mut environment = *captured;
                if !match_pattern(&pattern, &arguments[0], &mut environment) {
                    return Err(EvaluationError::NonExhaustivePattern);
                }
                Self::data(self.eval(&body, &environment, depth)?)?
            }
            Callable::Constructor(name) => RuntimeValue::Constructor(name, arguments),
            Callable::Add => {
                let [RuntimeValue::Integer(left), RuntimeValue::Integer(right)] =
                    arguments.as_slice()
                else {
                    return Err(EvaluationError::TypeMismatch(
                        "SDK addition needs two integers",
                    ));
                };
                RuntimeValue::Integer(
                    left.checked_add(*right)
                        .ok_or(EvaluationError::IntegerOverflow)?,
                )
            }
            Callable::Equal => {
                let [RuntimeValue::Integer(left), RuntimeValue::Integer(right)] =
                    arguments.as_slice()
                else {
                    return Err(EvaluationError::TypeMismatch(
                        "SDK equality needs two integers",
                    ));
                };
                RuntimeValue::Boolean(left == right)
            }
        }))
    }

    fn eval(
        &mut self,
        expr: &Expression,
        environment: &Environment,
        depth: usize,
    ) -> Result<Evaluated, EvaluationError> {
        self.tick()?;
        Ok(match expr {
            ir::Value::Literal(_, ir::Literal::WholeNumber(number)) => {
                Evaluated::Data(RuntimeValue::Integer(*number))
            }
            ir::Value::Literal(_, ir::Literal::Bool(boolean)) => {
                Evaluated::Data(RuntimeValue::Boolean(*boolean))
            }
            ir::Value::Literal(_, ir::Literal::String(string)) => {
                Evaluated::Data(RuntimeValue::String(string.clone()))
            }
            ir::Value::Literal(..) => {
                return Err(EvaluationError::UnsupportedExpression("literal kind"));
            }
            ir::Value::Unit(_) => Evaluated::Data(RuntimeValue::Unit),
            ir::Value::List(_, values) => Evaluated::Data(RuntimeValue::List(
                values
                    .iter()
                    .map(|value| Self::data(self.eval(value, environment, depth)?))
                    .collect::<Result<_, _>>()?,
            )),
            ir::Value::Tuple(_, values) => Evaluated::Data(RuntimeValue::Tuple(
                values
                    .iter()
                    .map(|value| Self::data(self.eval(value, environment, depth)?))
                    .collect::<Result<_, _>>()?,
            )),
            ir::Value::Variable(_, name) => environment
                .get(name)
                .cloned()
                .ok_or_else(|| EvaluationError::UnknownVariable(name.clone()))?,
            ir::Value::Reference(_, name) => {
                if let Some(definition) = self
                    .definition(name)
                    .filter(|definition| definition.input_types.is_empty())
                {
                    Evaluated::Data(self.call_user(
                        &definition,
                        vec![],
                        &Environment::new(),
                        depth,
                    )?)
                } else {
                    self.resolve_reference(name)?
                }
            }
            ir::Value::Constructor(_, name) => match self.constructor_arity(name) {
                Some(0) => Evaluated::Data(RuntimeValue::Constructor(name.clone(), vec![])),
                Some(_) => Evaluated::Function(Callable::Constructor(name.clone()), vec![]),
                None => return Err(EvaluationError::UnknownReference(name.clone())),
            },
            ir::Value::Apply(_, function, argument) => {
                let function = self.eval(function, environment, depth)?;
                let argument = Self::data(self.eval(argument, environment, depth)?)?;
                self.apply(function, argument, depth)?
            }
            ir::Value::Lambda(_, pattern, body) => Evaluated::Function(
                Callable::Lambda(pattern.clone(), body.clone(), Box::new(environment.clone())),
                vec![],
            ),
            ir::Value::LetDefinition(_, name, definition, in_expr) => {
                let binding = if definition.input_types.is_empty() {
                    self.eval(&definition.body, environment, depth)?
                } else {
                    Evaluated::Function(
                        Callable::Local(definition.clone(), Box::new(environment.clone())),
                        vec![],
                    )
                };
                let mut next = environment.clone();
                next.insert(name.clone(), binding);
                self.eval(in_expr, &next, depth)?
            }
            ir::Value::LetRecursion(_, definitions, in_expr) => {
                let group: Rc<HashMap<ir::Name, Definition>> = Rc::new(
                    definitions
                        .iter()
                        .map(|(name, definition)| (name.clone(), *definition.clone()))
                        .collect(),
                );
                let mut next = environment.clone();
                for name in group.keys() {
                    next.insert(
                        name.clone(),
                        Evaluated::Function(
                            Callable::Recursive(
                                name.clone(),
                                group.clone(),
                                Box::new(environment.clone()),
                            ),
                            vec![],
                        ),
                    );
                }
                self.eval(in_expr, &next, depth)?
            }
            ir::Value::Destructure(_, pattern, value, in_expr) => {
                let value = Self::data(self.eval(value, environment, depth)?)?;
                let mut next = environment.clone();
                if !match_pattern(pattern, &value, &mut next) {
                    return Err(EvaluationError::NonExhaustivePattern);
                }
                self.eval(in_expr, &next, depth)?
            }
            ir::Value::PatternMatch(_, subject, cases) => {
                let subject = Self::data(self.eval(subject, environment, depth)?)?;
                let mut result = None;
                for (pattern, body) in cases {
                    let mut bindings = environment.clone();
                    if match_pattern(pattern, &subject, &mut bindings) {
                        result = Some(self.eval(body, &bindings, depth)?);
                        break;
                    }
                }
                result.ok_or(EvaluationError::NonExhaustivePattern)?
            }
            ir::Value::IfThenElse(_, condition, yes, no) => {
                match Self::data(self.eval(condition, environment, depth)?)? {
                    RuntimeValue::Boolean(true) => self.eval(yes, environment, depth)?,
                    RuntimeValue::Boolean(false) => self.eval(no, environment, depth)?,
                    _ => return Err(EvaluationError::TypeMismatch("Condition is not boolean")),
                }
            }
            _ => return Err(EvaluationError::UnsupportedExpression("value form")),
        })
    }
}

fn match_pattern(
    pattern: &ir::Pattern<ir::Type<ir::Attrs>>,
    value: &RuntimeValue,
    environment: &mut Environment,
) -> bool {
    match (pattern, value) {
        (ir::Pattern::Wildcard(_), _) => true,
        (ir::Pattern::As(_, inner, name), _) if match_pattern(inner, value, environment) => {
            environment.insert(name.clone(), Evaluated::Data(value.clone()));
            true
        }
        (ir::Pattern::EmptyList(_), RuntimeValue::List(values)) => values.is_empty(),
        (ir::Pattern::HeadTail(_, head, tail), RuntimeValue::List(values))
            if !values.is_empty() =>
        {
            match_pattern(head, &values[0], environment)
                && match_pattern(tail, &RuntimeValue::List(values[1..].to_vec()), environment)
        }
        (
            ir::Pattern::Constructor(_, name, patterns),
            RuntimeValue::Constructor(actual, arguments),
        ) => {
            name == actual
                && patterns.len() == arguments.len()
                && patterns
                    .iter()
                    .zip(arguments)
                    .all(|(pattern, value)| match_pattern(pattern, value, environment))
        }
        (ir::Pattern::Literal(_, ir::Literal::Bool(expected)), RuntimeValue::Boolean(actual)) => {
            expected == actual
        }
        (
            ir::Pattern::Literal(_, ir::Literal::WholeNumber(expected)),
            RuntimeValue::Integer(actual),
        ) => expected == actual,
        (ir::Pattern::Unit(_), RuntimeValue::Unit) => true,
        (ir::Pattern::Tuple(_, patterns), RuntimeValue::Tuple(values)) => {
            patterns.len() == values.len()
                && patterns
                    .iter()
                    .zip(values)
                    .all(|(pattern, value)| match_pattern(pattern, value, environment))
        }
        _ => false,
    }
}
