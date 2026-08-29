use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use regex::Regex;

static GLOB_CACHE: LazyLock<Mutex<HashMap<String, Regex>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Glob matching over `/`-separated relative paths. Supports `*`, `?` and `**`.
pub fn glob_matches(glob: &str, path: &str) -> bool {
    let regex = {
        let mut cache = GLOB_CACHE.lock().expect("glob cache poisoned");
        cache
            .entry(glob.to_string())
            .or_insert_with(|| compile_glob(glob))
            .clone()
    };
    regex.is_match(path)
}

fn compile_glob(glob: &str) -> Regex {
    let chars: Vec<char> = glob.chars().collect();
    let mut sb = String::from("^");
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '*' && i + 1 < chars.len() && chars[i + 1] == '*' {
            // `**/` also matches zero directories, so `docs/**/x.md` finds `docs/x.md`.
            if i + 2 < chars.len() && chars[i + 2] == '/' {
                sb.push_str("(?:.*/)?");
                i += 3;
            } else {
                sb.push_str(".*");
                i += 2;
            }
        } else {
            i += 1;
            match c {
                '*' => sb.push_str("[^/]*"),
                '?' => sb.push_str("[^/]"),
                ch if "\\.+()^$|{}[]".contains(ch) => {
                    sb.push('\\');
                    sb.push(ch);
                }
                ch => sb.push(ch),
            }
        }
    }
    sb.push('$');
    Regex::new(&sb).expect("compiled glob is a valid regex")
}
