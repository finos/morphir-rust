//! Gleam names and imports for one generated module.

use morphir_core::naming::{FQName, ModuleName, Path};
use std::collections::BTreeMap;
use std::io::{Error, ErrorKind, Result};

pub(super) fn module_path(path: &Path) -> String {
    path.segments
        .iter()
        .map(|part| part.to_snake_case())
        .collect::<Vec<_>>()
        .join("/")
}

#[derive(Default)]
pub(super) struct ModuleNames {
    pub module: Option<ModuleName>,
    imports: BTreeMap<String, (String, Path)>,
}

impl ModuleNames {
    pub fn reference(&mut self, package: &str, name: &FQName, local: String) -> Result<String> {
        if name.package_path == Path::new(package)
            && self
                .module
                .as_ref()
                .is_some_and(|module| module.as_path() == &name.module_path)
        {
            return Ok(local);
        }
        let path = module_path(&name.module_path);
        self.import(&path, &name.package_path)
            .map(|alias| format!("{alias}.{local}"))
    }

    pub fn import(&mut self, path: &str, package: &Path) -> Result<String> {
        if let Some((alias, existing_package)) = self.imports.get(path) {
            if existing_package != package {
                return Err(Error::new(
                    ErrorKind::InvalidInput,
                    format!("Gleam module '{path}' is supplied by more than one package"),
                ));
            }
            return Ok(alias.clone());
        }
        let base = path.rsplit('/').next().unwrap_or(path);
        let mut alias = base.to_string();
        let mut suffix = 2;
        while self.imports.values().any(|(used, _)| used == &alias) {
            alias = format!("{base}_{suffix}");
            suffix += 1;
        }
        self.imports
            .insert(path.into(), (alias.clone(), package.clone()));
        Ok(alias)
    }

    pub fn declarations(&self) -> String {
        self.imports
            .iter()
            .map(|(path, (alias, _))| {
                if path.rsplit('/').next() == Some(alias.as_str()) {
                    format!("import {path}\n")
                } else {
                    format!("import {path} as {alias}\n")
                }
            })
            .collect()
    }
}
