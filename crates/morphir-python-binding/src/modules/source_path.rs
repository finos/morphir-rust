//! Normalize document identifiers without reading the filesystem.

use crate::{Outcome, error};
use percent_encoding::percent_decode_str;
use url::Url;

pub(super) fn normalize(value: &str) -> Outcome<String> {
    let raw = value
        .split(['?', '#'])
        .next()
        .unwrap_or(value)
        .replace('\\', "/");
    // Check the original spelling before URL parsing can erase dot segments.
    decode_path(&raw)?;
    let windows_drive =
        raw.as_bytes().get(1) == Some(&b':') && raw.as_bytes().get(2) == Some(&b'/');
    if !windows_drive && raw.contains(':') {
        let url =
            Url::parse(&raw).map_err(|e| error("PY001", format!("Invalid source URI: {e}")))?;
        if url.cannot_be_a_base() {
            return Err(error("PY001", "Source URI must have a hierarchical path"));
        }
        Ok(format!(
            "{}://{}{}",
            url.scheme(),
            url.authority(),
            decode_path(url.path())?
        ))
    } else {
        decode_path(&raw)
    }
}

fn decode_path(path: &str) -> Outcome<String> {
    path.split('/')
        .map(|segment| {
            let decoded = percent_decode_str(segment)
                .decode_utf8()
                .map_err(|_| error("PY001", "Source path contains invalid UTF-8 encoding"))?;
            if matches!(decoded.as_ref(), "." | "..") || decoded.contains(['/', '\\', '\0']) {
                return Err(error(
                    "PY001",
                    "Source path contains a dot segment or encoded separator",
                ));
            }
            Ok(decoded.into_owned())
        })
        .collect::<Outcome<Vec<_>>>()
        .map(|parts| parts.join("/"))
}
