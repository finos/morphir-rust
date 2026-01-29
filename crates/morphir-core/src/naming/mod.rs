#![allow(clippy::duplicate_mod)]
pub mod fqname;
pub mod module_name;
pub mod name;
pub mod package_name;
pub mod path;
pub mod qname;
pub mod interner;

// Re-export common types
pub use interner::{intern, resolve, Word};
pub use fqname::FQName;
pub use module_name::ModuleName;
pub use name::Name;
pub use package_name::PackageName;
pub use path::Path;
pub use qname::QName;

/// Namespace for serialization codecs
pub mod codecs {
    /// Classic (Legacy/V3) serialization logic
    pub mod classic {
        use crate::naming::{Name, Path};
        use serde::Serializer;

        pub fn serialize_name<S>(name: &Name, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            // Legacy Name: ["word", "word"]
            serializer.collect_seq(&name.words)
        }

        pub fn serialize_path<S>(path: &Path, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            // Legacy Path: [Name, Name] where Name is ["word", "word"]
            let parts: Vec<&Vec<String>> = path.segments.iter().map(|n| &n.words).collect();
            serializer.collect_seq(parts)
        }
    }
}
