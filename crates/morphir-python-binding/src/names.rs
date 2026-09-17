use crate::{Outcome, error};
use morphir_core::naming::Name;

const RESERVED: &[&str] = &[
    "False",
    "None",
    "True",
    "and",
    "as",
    "assert",
    "async",
    "await",
    "break",
    "class",
    "continue",
    "def",
    "del",
    "elif",
    "else",
    "except",
    "finally",
    "for",
    "from",
    "global",
    "if",
    "import",
    "in",
    "is",
    "lambda",
    "nonlocal",
    "not",
    "or",
    "pass",
    "raise",
    "return",
    "try",
    "while",
    "with",
    "yield",
    "dataclass",
    "int",
    "str",
    "float",
    "bool",
    "tuple",
];

pub(crate) fn identifier(text: &str) -> Outcome<Name> {
    if text.is_empty()
        || !text.as_bytes()[0].is_ascii_alphabetic()
        || !text.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
        || RESERVED.contains(&text)
    {
        return Err(error(
            "PY003",
            format!("Unsupported or reserved Python identifier: {text}"),
        ));
    }
    Ok(Name::from(text))
}

pub(crate) fn type_name(name: &Name) -> Outcome<String> {
    let text = name.to_pascal_case();
    faithful(name, text)
}

pub(crate) fn field_name(name: &Name) -> Outcome<String> {
    faithful(name, name.to_canonical_string().replace('-', "_"))
}

pub(crate) fn module_file_stem(name: &Name) -> Outcome<String> {
    let stem = field_name(name)?;
    let lower = stem.to_ascii_lowercase();
    if ["con", "prn", "aux", "nul"].contains(&lower.as_str())
        || ["com", "lpt"].iter().any(|prefix| {
            lower.strip_prefix(prefix).is_some_and(|suffix| {
                suffix.len() == 1 && matches!(suffix.as_bytes()[0], b'1'..=b'9')
            })
        })
    {
        return Err(error(
            "PY003",
            format!("Module name is reserved on Windows: {stem}"),
        ));
    }
    Ok(stem)
}

fn faithful(name: &Name, text: String) -> Outcome<String> {
    if identifier(&text)? != *name {
        return Err(error(
            "PY003",
            format!("Name cannot be represented losslessly in Python: {name}"),
        ));
    }
    Ok(text)
}
