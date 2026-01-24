//! Shared Code Generation Utilities
//!
//! Common utilities for code generation across language bindings.

/// Indentation helper
pub struct Indent {
    level: usize,
    size: usize,
}

impl Indent {
    pub fn new(level: usize, size: usize) -> Self {
        Self { level, size }
    }

    pub fn to_string(&self) -> String {
        " ".repeat(self.level * self.size)
    }

    pub fn increment(&mut self) {
        self.level += 1;
    }

    pub fn decrement(&mut self) {
        if self.level > 0 {
            self.level -= 1;
        }
    }
}

impl Default for Indent {
    fn default() -> Self {
        Self { level: 0, size: 2 }
    }
}

/// Format code with proper indentation
pub fn format_with_indent(code: &str, indent: &Indent) -> String {
    let indent_str = indent.to_string();
    code.lines()
        .map(|line| {
            if line.trim().is_empty() {
                String::new()
            } else {
                format!("{}{}", indent_str, line)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Generate comment
pub fn comment(text: &str) -> String {
    format!("// {}", text)
}

/// Generate multi-line comment
pub fn multi_line_comment(lines: &[&str]) -> String {
    lines
        .iter()
        .map(|line| format!("// {}", line))
        .collect::<Vec<_>>()
        .join("\n")
}
