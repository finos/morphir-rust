use std::collections::{BTreeMap, BTreeSet};

use crate::{AvroInternalError, AvroType, NamedSchema};

#[derive(Default)]
pub(super) struct InlineState {
    pub(super) active: BTreeSet<String>,
    pub(super) defined: BTreeSet<String>,
}

#[derive(Clone, Copy)]
pub(super) enum DefinitionScope<'names> {
    All,
    Owned,
    Selected(&'names BTreeSet<String>),
}

impl DefinitionScope<'_> {
    pub(super) fn allows(self, name: &str, linked_names: &BTreeSet<String>) -> bool {
        match self {
            Self::All => true,
            Self::Owned => !linked_names.contains(name),
            Self::Selected(names) => names.contains(name),
        }
    }
}

pub(super) fn collect_available_references(
    tpe: &AvroType,
    schemas: &BTreeMap<String, &NamedSchema>,
    references: &mut BTreeSet<String>,
) {
    match tpe {
        AvroType::Named(name) if schemas.contains_key(&name.to_string()) => {
            references.insert(name.to_string());
        }
        AvroType::Array(element, _)
        | AvroType::Map(element, _)
        | AvroType::Logical {
            physical: element, ..
        }
        | AvroType::Annotated {
            physical: element, ..
        } => collect_available_references(element, schemas, references),
        AvroType::Union(union) => {
            for branch in union.branches() {
                collect_available_references(branch, schemas, references);
            }
        }
        AvroType::Null
        | AvroType::Boolean
        | AvroType::Int
        | AvroType::Long
        | AvroType::Float
        | AvroType::Double
        | AvroType::Bytes
        | AvroType::String
        | AvroType::Named(_) => {}
    }
}

pub(super) fn strongly_connected_components(
    graph: &BTreeMap<String, BTreeSet<String>>,
) -> Result<Vec<Vec<String>>, AvroInternalError> {
    let mut tarjan = Tarjan::new(graph);
    for name in graph.keys() {
        if !tarjan.indices.contains_key(name) {
            tarjan.visit(name)?;
        }
    }
    Ok(tarjan.components)
}

struct Tarjan<'graph> {
    graph: &'graph BTreeMap<String, BTreeSet<String>>,
    next_index: usize,
    indices: BTreeMap<String, usize>,
    low_links: BTreeMap<String, usize>,
    stack: Vec<String>,
    on_stack: BTreeSet<String>,
    components: Vec<Vec<String>>,
}

impl<'graph> Tarjan<'graph> {
    fn new(graph: &'graph BTreeMap<String, BTreeSet<String>>) -> Self {
        Self {
            graph,
            next_index: 0,
            indices: BTreeMap::new(),
            low_links: BTreeMap::new(),
            stack: Vec::new(),
            on_stack: BTreeSet::new(),
            components: Vec::new(),
        }
    }

    fn visit(&mut self, name: &str) -> Result<(), AvroInternalError> {
        let index = self.next_index;
        self.next_index += 1;
        self.indices.insert(name.to_owned(), index);
        self.low_links.insert(name.to_owned(), index);
        self.stack.push(name.to_owned());
        self.on_stack.insert(name.to_owned());

        let dependencies = self.graph.get(name).cloned().ok_or_else(|| {
            AvroInternalError::invariant(format!("JSON schema graph lost node {name}"))
        })?;
        for dependency in &dependencies {
            if !self.indices.contains_key(dependency) {
                self.visit(dependency)?;
                let dependency_low = *self.low_links.get(dependency).ok_or_else(|| {
                    AvroInternalError::invariant(format!(
                        "Tarjan traversal has no low link for {dependency}"
                    ))
                })?;
                self.low_links
                    .entry(name.to_owned())
                    .and_modify(|low| *low = (*low).min(dependency_low));
            } else if self.on_stack.contains(dependency) {
                let dependency_index = *self.indices.get(dependency).ok_or_else(|| {
                    AvroInternalError::invariant(format!(
                        "Tarjan traversal has no index for {dependency}"
                    ))
                })?;
                self.low_links
                    .entry(name.to_owned())
                    .and_modify(|low| *low = (*low).min(dependency_index));
            }
        }

        let low_link = self.low_links.get(name).ok_or_else(|| {
            AvroInternalError::invariant(format!("Tarjan traversal lost low link for {name}"))
        })?;
        let index = self.indices.get(name).ok_or_else(|| {
            AvroInternalError::invariant(format!("Tarjan traversal lost index for {name}"))
        })?;
        if low_link != index {
            return Ok(());
        }
        let mut component = Vec::new();
        loop {
            let member = self.stack.pop().ok_or_else(|| {
                AvroInternalError::invariant(format!(
                    "Tarjan stack ended before component root {name}"
                ))
            })?;
            self.on_stack.remove(&member);
            let complete = member == name;
            component.push(member);
            if complete {
                break;
            }
        }
        component.sort();
        self.components.push(component);
        Ok(())
    }
}

pub(super) fn include_component_dependencies(
    component: usize,
    dependencies: &[BTreeSet<usize>],
    included: &mut BTreeSet<usize>,
) -> Result<(), AvroInternalError> {
    let direct = dependencies.get(component).ok_or_else(|| {
        AvroInternalError::invariant(format!(
            "JSON component dependency index {component} is missing"
        ))
    })?;
    for dependency in direct {
        if included.insert(*dependency) {
            include_component_dependencies(*dependency, dependencies, included)?;
        }
    }
    Ok(())
}

pub(super) fn component_dependency_order(
    components: &[Vec<String>],
    dependencies: &[BTreeSet<usize>],
) -> Result<Vec<usize>, AvroInternalError> {
    fn visit(
        component: usize,
        components: &[Vec<String>],
        dependencies: &[BTreeSet<usize>],
        visited: &mut BTreeSet<usize>,
        ordered: &mut Vec<usize>,
    ) -> Result<(), AvroInternalError> {
        if !visited.insert(component) {
            return Ok(());
        }
        let direct = dependencies.get(component).ok_or_else(|| {
            AvroInternalError::invariant(format!(
                "JSON component ordering index {component} is missing"
            ))
        })?;
        let mut component_dependencies = direct.iter().copied().collect::<Vec<_>>();
        component_dependencies.sort_by_key(|index| {
            components
                .get(*index)
                .and_then(|component| component.first())
                .cloned()
                .unwrap_or_default()
        });
        for dependency in component_dependencies {
            visit(dependency, components, dependencies, visited, ordered)?;
        }
        ordered.push(component);
        Ok(())
    }

    let mut seeds = (0..components.len()).collect::<Vec<_>>();
    seeds.sort_by_key(|index| {
        components
            .get(*index)
            .and_then(|component| component.first())
            .cloned()
            .unwrap_or_default()
    });
    let mut visited = BTreeSet::new();
    let mut ordered = Vec::new();
    for seed in seeds {
        visit(seed, components, dependencies, &mut visited, &mut ordered)?;
    }
    Ok(ordered)
}
