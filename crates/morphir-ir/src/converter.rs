use crate::ir::{classic, v4};

pub fn classic_to_v4(pkg: classic::Package) -> v4::Package {
    // Placeholder conversion
    v4::PackageDefinition {
        modules: vec![]
    }
}

pub fn v4_to_classic(pkg: v4::Package) -> classic::Package {
    // Placeholder conversion
    classic::Package {
        name: "placeholder".to_string(),
        modules: vec![]
    }
}
