use crate::naming::Path;

/// A Cursor for navigating the IR with context.
#[derive(Debug, Clone, Default)]
pub struct Cursor {
    path_stack: Vec<String>,
    depth: usize,
}

impl Cursor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Enter a named segment of the IR tree (pushes to path stack).
    pub fn enter(&mut self, segment: &str) {
        self.path_stack.push(segment.to_string());
        self.depth += 1;
    }

    /// Exit the current segment (pops from path stack).
    pub fn exit(&mut self) {
        self.path_stack.pop();
        self.depth -= 1;
    }

    /// Advance to the next sibling (currently a placeholder for sibling tracking).
    pub fn next(&mut self) {
        // Future: track index or sibling state
    }

    /// Get the current traversal path as a Morphir Path.
    pub fn path(&self) -> Path {
        // Simple conversion: treating stack segments as path words
        let segments: Vec<crate::naming::Name> = self
            .path_stack
            .iter()
            .map(|s| crate::naming::Name::from(s.as_str()))
            .collect();
        Path { segments }
    }

    /// Get current nesting depth.
    pub fn depth(&self) -> usize {
        self.depth
    }
}
