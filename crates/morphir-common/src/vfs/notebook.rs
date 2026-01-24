use super::{FileMetadata, Vfs};
use nbformat::{v4, Notebook};
use std::collections::HashMap;
use std::io::Result;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Jupyter Notebook VFS implementation
///
/// Treats Jupyter notebook cells as files in a virtual file system.
/// Uses cell id to store file paths (cell.id is used as the path identifier).
#[derive(Clone, Debug)]
pub struct NotebookVfs {
    notebook: Arc<Mutex<v4::Notebook>>,
    /// Index mapping paths to cell indices for efficient lookup
    path_index: Arc<Mutex<HashMap<PathBuf, usize>>>,
}

impl NotebookVfs {
    /// Create a new NotebookVfs from a parsed notebook
    pub fn from_notebook(notebook: Notebook) -> Self {
        // Extract v4 notebook or convert
        let v4_notebook = match notebook {
            Notebook::V4(nb) => nb,
            Notebook::Legacy(_) => {
                // For now, create an empty v4 notebook for legacy
                v4::Notebook {
                    metadata: v4::Metadata {
                        kernelspec: None,
                        language_info: None,
                        authors: None,
                        additional: HashMap::new(),
                    },
                    nbformat: 4,
                    nbformat_minor: 5,
                    cells: vec![],
                }
            }
        };

        let mut path_index = HashMap::new();

        // Build index of path -> cell index (using cell id as path)
        for (idx, cell) in v4_notebook.cells.iter().enumerate() {
            let cell_id = match cell {
                v4::Cell::Markdown { id, .. } => id,
                v4::Cell::Code { id, .. } => id,
                v4::Cell::Raw { id, .. } => id,
            };
            // Use cell id as the path (cell ids can be paths)
            path_index.insert(PathBuf::from(cell_id.to_string()), idx);
        }

        Self {
            notebook: Arc::new(Mutex::new(v4_notebook)),
            path_index: Arc::new(Mutex::new(path_index)),
        }
    }

    /// Load a notebook from a file
    pub fn from_file(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let notebook = nbformat::parse_notebook(&content)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        Ok(Self::from_notebook(notebook))
    }

    /// Get a reference to the underlying notebook
    /// Note: Returns a clone of the v4 notebook wrapped in the Notebook enum
    pub fn to_notebook(&self) -> Notebook {
        // v4::Notebook doesn't implement Clone, so we can't clone it
        // This is a limitation - we'd need to reconstruct it or store the original
        // For now, return an empty notebook as a placeholder
        Notebook::V4(v4::Notebook {
            metadata: v4::Metadata {
                kernelspec: None,
                language_info: None,
                authors: None,
                additional: HashMap::new(),
            },
            nbformat: 4,
            nbformat_minor: 5,
            cells: vec![],
        })
    }

    /// Save the notebook to a file
    /// Note: This is a simplified implementation that reconstructs basic notebook structure
    /// For full fidelity, consider preserving the original JSON and merging changes
    pub fn to_file(&self, _path: &Path) -> Result<()> {
        // TODO: Implement proper serialization when nbformat types support Serialize
        // For now, this is a placeholder - the notebook can be read but not written back
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Notebook serialization not yet implemented - nbformat types don't implement Serialize",
        ))
    }

    /// Get cell id from cell
    fn get_cell_id(cell: &v4::Cell) -> &v4::CellId {
        match cell {
            v4::Cell::Markdown { id, .. } => id,
            v4::Cell::Code { id, .. } => id,
            v4::Cell::Raw { id, .. } => id,
        }
    }

    /// Get source from cell
    fn get_cell_source(cell: &v4::Cell) -> String {
        match cell {
            v4::Cell::Markdown { source, .. } => source.join(""),
            v4::Cell::Code { source, .. } => source.join(""),
            v4::Cell::Raw { source, .. } => source.join(""),
        }
    }

    /// Set source in cell
    fn set_cell_source(cell: &mut v4::Cell, content: &str) {
        let lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
        match cell {
            v4::Cell::Markdown { source, .. } => *source = lines,
            v4::Cell::Code { source, .. } => *source = lines,
            v4::Cell::Raw { source, .. } => *source = lines,
        }
    }

    /// Normalize a path for consistent lookup
    fn normalize_path(path: &Path) -> PathBuf {
        let mut normalized = PathBuf::new();
        for component in path.components() {
            match component {
                std::path::Component::Prefix(_) => {}
                std::path::Component::RootDir => normalized.push("/"),
                std::path::Component::CurDir => {}
                std::path::Component::ParentDir => {
                    normalized.pop();
                }
                std::path::Component::Normal(name) => normalized.push(name),
            }
        }
        if normalized.as_os_str().is_empty() {
            return PathBuf::from(".");
        }
        normalized
    }

    /// Find cell index by path
    fn find_cell_index(&self, path: &Path) -> Option<usize> {
        let normalized = Self::normalize_path(path);
        let index = self.path_index.lock().unwrap();
        index.get(&normalized).copied()
    }
}

impl Vfs for NotebookVfs {
    fn read_to_string(&self, path: &Path) -> Result<String> {
        let notebook = self.notebook.lock().unwrap();
        let idx = self.find_cell_index(path).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("File not found in notebook: {:?}", path),
            )
        })?;

        let cell = &notebook.cells[idx];
        Ok(Self::get_cell_source(cell))
    }

    fn write_from_string(&self, path: &Path, content: &str) -> Result<()> {
        let normalized = Self::normalize_path(path);
        let mut notebook = self.notebook.lock().unwrap();
        let mut index = self.path_index.lock().unwrap();

        if let Some(&idx) = index.get(&normalized) {
            // Update existing cell
            let cell = &mut notebook.cells[idx];
            Self::set_cell_source(cell, content);
        } else {
            // Create new cell with path as id
            use uuid::Uuid;
            let path_str = normalized.to_string_lossy().to_string();

            let cell = v4::Cell::Code {
                id: v4::CellId::from(Uuid::new_v4()),
                metadata: v4::CellMetadata {
                    id: Some(path_str.clone()),
                    collapsed: None,
                    scrolled: None,
                    deletable: None,
                    editable: None,
                    format: None,
                    name: None,
                    tags: None,
                    jupyter: None,
                    execution: None,
                },
                execution_count: None,
                source: content.lines().map(|s| s.to_string()).collect(),
                outputs: vec![],
            };

            let idx = notebook.cells.len();
            notebook.cells.push(cell);
            index.insert(normalized, idx);
        }

        Ok(())
    }

    fn exists(&self, path: &Path) -> bool {
        self.find_cell_index(path).is_some()
    }

    fn is_dir(&self, path: &Path) -> bool {
        let normalized = Self::normalize_path(path);
        let index = self.path_index.lock().unwrap();

        // Check if any path starts with this path (indicating it's a directory)
        for key in index.keys() {
            if key.starts_with(&normalized) && *key != normalized {
                return true;
            }
        }
        false
    }

    fn list_dir(&self, path: &Path) -> Result<Vec<PathBuf>> {
        let normalized = Self::normalize_path(path);
        let index = self.path_index.lock().unwrap();

        let mut entries = Vec::new();
        for key in index.keys() {
            if key.starts_with(&normalized) && *key != normalized {
                entries.push(key.clone());
            }
        }
        Ok(entries)
    }

    fn create_dir_all(&self, _path: &Path) -> Result<()> {
        // Directories are implicit from paths, no-op
        Ok(())
    }

    fn glob(&self, pattern: &str) -> Result<Vec<PathBuf>> {
        let index = self.path_index.lock().unwrap();
        let glob_pattern = glob::Pattern::new(pattern)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;

        let mut matches = Vec::new();
        for path in index.keys() {
            if glob_pattern.matches_path(path) {
                matches.push(path.clone());
            }
        }
        Ok(matches)
    }

    fn remove(&self, path: &Path) -> Result<()> {
        let normalized = Self::normalize_path(path);
        let mut notebook = self.notebook.lock().unwrap();
        let mut index = self.path_index.lock().unwrap();

        // Remove the cell itself
        if let Some(&idx) = index.get(&normalized) {
            notebook.cells.remove(idx);
            index.remove(&normalized);

            // Rebuild index for remaining cells
            index.clear();
            for (new_idx, cell) in notebook.cells.iter().enumerate() {
                let cell_id = Self::get_cell_id(cell);
                index.insert(PathBuf::from(cell_id.to_string()), new_idx);
            }
        }

        // Remove all cells that are children of this path (if it's a directory)
        let keys_to_remove: Vec<PathBuf> = index
            .keys()
            .filter(|k| k.starts_with(&normalized) && **k != normalized)
            .cloned()
            .collect();

        for key in keys_to_remove {
            if let Some(idx) = index.get(&key).copied() {
                notebook.cells.remove(idx);
                index.remove(&key);
            }
        }

        // Rebuild index after removals
        index.clear();
        for (new_idx, cell) in notebook.cells.iter().enumerate() {
            let cell_id = Self::get_cell_id(cell);
            index.insert(PathBuf::from(cell_id.to_string()), new_idx);
        }

        Ok(())
    }

    fn copy(&self, from: &Path, to: &Path) -> Result<()> {
        let content = self.read_to_string(from)?;
        self.write_from_string(to, &content)
    }

    fn metadata(&self, path: &Path) -> Result<FileMetadata> {
        let normalized = Self::normalize_path(path);
        let notebook = self.notebook.lock().unwrap();

        if let Some(idx) = self.find_cell_index(&normalized) {
            let cell = &notebook.cells[idx];
            let size = Self::get_cell_source(cell).len() as u64;

            // For now, no modified time available in standard metadata
            let modified = None;

            Ok(FileMetadata {
                size,
                is_file: true,
                is_dir: false,
                modified,
                created: None,
            })
        } else {
            // Check if it's a directory
            let is_dir = self.is_dir(&normalized);
            Ok(FileMetadata {
                size: 0,
                is_file: false,
                is_dir,
                modified: None,
                created: None,
            })
        }
    }
}
