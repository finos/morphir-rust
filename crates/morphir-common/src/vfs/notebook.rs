use super::{FileMetadata, Vfs};
use nbformat::{Notebook, Cell, CellType};
use std::collections::HashMap;
use std::io::Result;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

/// Jupyter Notebook VFS implementation
///
/// Treats Jupyter notebook cells as files in a virtual file system.
/// Uses cell metadata to store file paths and metadata.
/// Cell metadata convention: `metadata.morphir.path` for file paths.
#[derive(Clone, Debug)]
pub struct NotebookVfs {
    notebook: Arc<Mutex<Notebook>>,
    /// Index mapping paths to cell indices for efficient lookup
    path_index: Arc<Mutex<HashMap<PathBuf, usize>>>,
}

impl NotebookVfs {
    /// Create a new NotebookVfs from a parsed notebook
    pub fn from_notebook(notebook: Notebook) -> Self {
        let mut path_index = HashMap::new();
        
        // Build index of path -> cell index
        for (idx, cell) in notebook.cells.iter().enumerate() {
            if let Some(path) = Self::extract_path_from_cell(cell) {
                path_index.insert(path, idx);
            }
        }
        
        Self {
            notebook: Arc::new(Mutex::new(notebook)),
            path_index: Arc::new(Mutex::new(path_index)),
        }
    }

    /// Load a notebook from a file
    pub fn from_file(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let notebook: Notebook = serde_json::from_str(&content)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        Ok(Self::from_notebook(notebook))
    }

    /// Serialize the notebook back to a Notebook struct
    pub fn to_notebook(&self) -> Notebook {
        self.notebook.lock().unwrap().clone()
    }

    /// Save the notebook to a file
    pub fn to_file(&self, path: &Path) -> Result<()> {
        let notebook = self.notebook.lock().unwrap();
        let json = serde_json::to_string_pretty(&*notebook)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Extract file path from cell metadata
    fn extract_path_from_cell(cell: &Cell) -> Option<PathBuf> {
        cell.metadata
            .get("morphir")
            .and_then(|m| m.get("path"))
            .and_then(|p| p.as_str())
            .map(PathBuf::from)
    }

    /// Set file path in cell metadata
    fn set_path_in_cell(cell: &mut Cell, path: &Path) {
        let path_str = path.to_string_lossy().to_string();
        cell.metadata
            .entry("morphir".to_string())
            .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()))
            .as_object_mut()
            .unwrap()
            .insert("path".to_string(), serde_json::Value::String(path_str));
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
        match &cell.cell_type {
            CellType::Code => {
                // Extract source from code cell
                cell.source.as_str().ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "Cell source is not a string",
                    )
                })
            }
            CellType::Markdown => {
                // For markdown cells, return the source as-is
                cell.source.as_str().ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "Cell source is not a string",
                    )
                })
            }
            _ => Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Cell type not supported for file content",
            )),
        }
    }

    fn write_from_string(&self, path: &Path, content: &str) -> Result<()> {
        let normalized = Self::normalize_path(path);
        let mut notebook = self.notebook.lock().unwrap();
        let mut index = self.path_index.lock().unwrap();
        
        if let Some(&idx) = index.get(&normalized) {
            // Update existing cell
            let cell = &mut notebook.cells[idx];
            cell.source = nbformat::CellSource::String(content.to_string());
            Self::set_path_in_cell(cell, &normalized);
        } else {
            // Create new cell
            let mut cell = Cell {
                cell_type: CellType::Code,
                source: nbformat::CellSource::String(content.to_string()),
                metadata: serde_json::Map::new(),
                outputs: vec![],
                execution_count: None,
            };
            Self::set_path_in_cell(&mut cell, &normalized);
            
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
            if key.starts_with(&normalized) && key != &normalized {
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
            if key.starts_with(&normalized) && key != &normalized {
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
                if let Some(cell_path) = Self::extract_path_from_cell(cell) {
                    index.insert(cell_path, new_idx);
                }
            }
        }
        
        // Remove all cells that are children of this path (if it's a directory)
        let keys_to_remove: Vec<PathBuf> = index
            .keys()
            .filter(|k| k.starts_with(&normalized) && k != &normalized)
            .cloned()
            .collect();
        
        for key in keys_to_remove {
            if let Some(&idx) = index.get(&key) {
                notebook.cells.remove(idx);
                index.remove(&key);
            }
        }
        
        // Rebuild index after removals
        index.clear();
        for (new_idx, cell) in notebook.cells.iter().enumerate() {
            if let Some(cell_path) = Self::extract_path_from_cell(cell) {
                index.insert(cell_path, new_idx);
            }
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
        
        if let Some(&idx) = self.find_cell_index(&normalized) {
            let cell = &notebook.cells[idx];
            let size = match &cell.source {
                nbformat::CellSource::String(s) => s.len() as u64,
                nbformat::CellSource::Array(arr) => {
                    arr.iter().map(|s| s.len()).sum::<usize>() as u64
                }
            };
            
            // Try to extract metadata from cell metadata
            let modified = cell.metadata
                .get("morphir")
                .and_then(|m| m.get("metadata"))
                .and_then(|m| m.get("modified"))
                .and_then(|m| m.as_u64())
                .and_then(|ts| SystemTime::UNIX_EPOCH.checked_add(std::time::Duration::from_secs(ts)));
            
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
