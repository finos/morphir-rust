//! Host functions exposed to extension plugins
//!
//! These functions allow extensions to interact with the daemon.

use extism::{Function, UserData, Val, ValType};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tracing::debug;

/// Workspace information provided to extensions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceInfo {
    /// Workspace root path
    pub root: String,
    /// Output directory path
    pub output_dir: String,
}

/// Host state shared with extensions
#[derive(Debug, Clone)]
pub struct MorphirHostState {
    /// Workspace root directory
    pub workspace_root: PathBuf,
    /// Output directory
    pub output_dir: PathBuf,
    /// IR cache
    pub ir_cache: Arc<RwLock<HashMap<String, serde_json::Value>>>,
}

impl Default for MorphirHostState {
    fn default() -> Self {
        Self {
            workspace_root: PathBuf::from("."),
            output_dir: PathBuf::from(".morphir-dist"),
            ir_cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

/// Host functions container
pub struct MorphirHostFunctions {
    state: Arc<MorphirHostState>,
}

impl Default for MorphirHostFunctions {
    fn default() -> Self {
        Self::new(MorphirHostState::default())
    }
}

impl MorphirHostFunctions {
    /// Create new host functions with given state
    pub fn new(state: MorphirHostState) -> Self {
        Self {
            state: Arc::new(state),
        }
    }

    /// Create host functions for a workspace
    pub fn for_workspace(workspace_root: PathBuf, output_dir: PathBuf) -> Self {
        Self::new(MorphirHostState {
            workspace_root,
            output_dir,
            ir_cache: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    /// Convert to Extism functions
    pub fn into_functions(self) -> Vec<Function> {
        let state = self.state;

        vec![
            // Get workspace info
            {
                let state = state.clone();
                Function::new(
                    "morphir_get_workspace_info",
                    [ValType::I64],
                    [ValType::I64],
                    UserData::new(state),
                    get_workspace_info_impl,
                )
            },
            // Cache IR
            {
                let state = state.clone();
                Function::new(
                    "morphir_cache_ir",
                    [ValType::I64, ValType::I64],
                    [],
                    UserData::new(state),
                    cache_ir_impl,
                )
            },
            // Get cached IR
            {
                let state = state.clone();
                Function::new(
                    "morphir_get_cached_ir",
                    [ValType::I64],
                    [ValType::I64],
                    UserData::new(state),
                    get_cached_ir_impl,
                )
            },
            // Log function
            {
                let state = state.clone();
                Function::new(
                    "morphir_log",
                    [ValType::I64, ValType::I64],
                    [],
                    UserData::new(state),
                    log_impl,
                )
            },
        ]
    }

    /// Get the shared state
    pub fn state(&self) -> &Arc<MorphirHostState> {
        &self.state
    }
}

// Host function implementations

fn get_workspace_info_impl(
    _plugin: &mut extism::CurrentPlugin,
    _inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<Arc<MorphirHostState>>,
) -> Result<(), extism::Error> {
    let state = user_data.get()?;
    let state = state.lock().unwrap();

    let info = WorkspaceInfo {
        root: state.workspace_root.to_string_lossy().to_string(),
        output_dir: state.output_dir.to_string_lossy().to_string(),
    };

    let _json = serde_json::to_vec(&info).map_err(|e| extism::Error::msg(e.to_string()))?;

    // For now, just log - actual memory management would need extism's memory APIs
    debug!("get_workspace_info called, returning: {:?}", info);

    // In a real implementation, we'd allocate memory in the plugin and return a pointer
    outputs[0] = Val::I64(0);
    Ok(())
}

fn cache_ir_impl(
    _plugin: &mut extism::CurrentPlugin,
    _inputs: &[Val],
    _outputs: &mut [Val],
    user_data: UserData<Arc<MorphirHostState>>,
) -> Result<(), extism::Error> {
    let _state = user_data.get()?;

    // In a real implementation, we'd read the key and IR from plugin memory
    debug!("cache_ir called");

    Ok(())
}

fn get_cached_ir_impl(
    _plugin: &mut extism::CurrentPlugin,
    _inputs: &[Val],
    outputs: &mut [Val],
    user_data: UserData<Arc<MorphirHostState>>,
) -> Result<(), extism::Error> {
    let _state = user_data.get()?;

    // In a real implementation, we'd read the key from plugin memory
    debug!("get_cached_ir called");

    outputs[0] = Val::I64(0);
    Ok(())
}

fn log_impl(
    _plugin: &mut extism::CurrentPlugin,
    _inputs: &[Val],
    _outputs: &mut [Val],
    _user_data: UserData<Arc<MorphirHostState>>,
) -> Result<(), extism::Error> {
    // In a real implementation, we'd read the level and message from plugin memory
    // For now, just acknowledge the call
    debug!("Extension log called");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_host_state_default() {
        let state = MorphirHostState::default();
        assert_eq!(state.workspace_root, PathBuf::from("."));
    }

    #[test]
    fn test_host_functions_creation() {
        let funcs = MorphirHostFunctions::default();
        let functions = funcs.into_functions();
        assert!(!functions.is_empty());
    }
}
