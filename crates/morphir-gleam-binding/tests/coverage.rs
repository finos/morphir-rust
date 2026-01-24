//! Feature Coverage Tracking
//!
//! Tracks feature implementation status and generates coverage reports.

#[cfg(test)]
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Feature status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FeatureStatus {
    #[serde(rename = "unimplemented")]
    Unimplemented,
    #[serde(rename = "partial")]
    Partial,
    #[serde(rename = "implemented")]
    Implemented,
}

/// Feature definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureDefinition {
    pub id: String,
    pub name: String,
    pub category: String,
    pub scenarios: Vec<String>,
    pub status: FeatureStatus,
    pub implementation_path: Option<String>,
    pub test_path: Option<String>,
}

/// Feature registry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureRegistry {
    pub features: HashMap<String, Vec<FeatureDefinition>>,
}

impl FeatureRegistry {
    /// Load from YAML file
    pub fn from_yaml(path: &PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        let content = std::fs::read_to_string(path)?;
        let registry: FeatureRegistry = serde_yaml::from_str(&content)?;
        Ok(registry)
    }

    /// Get all features
    pub fn all_features(&self) -> Vec<&FeatureDefinition> {
        self.features.values().flatten().collect()
    }

    /// Count features by status
    pub fn count_by_status(&self) -> HashMap<FeatureStatus, usize> {
        let mut counts = HashMap::new();
        counts.insert(FeatureStatus::Unimplemented, 0);
        counts.insert(FeatureStatus::Partial, 0);
        counts.insert(FeatureStatus::Implemented, 0);

        for feature in self.all_features() {
            *counts.entry(feature.status).or_insert(0) += 1;
        }

        counts
    }

    /// Calculate coverage percentage
    pub fn coverage_percentage(&self) -> f64 {
        let counts = self.count_by_status();
        let total = counts.values().sum::<usize>();
        if total == 0 {
            return 0.0;
        }

        let implemented = counts
            .get(&FeatureStatus::Implemented)
            .copied()
            .unwrap_or(0);
        let partial = counts.get(&FeatureStatus::Partial).copied().unwrap_or(0);

        // Count partial as 50% coverage
        ((implemented as f64) + (partial as f64 * 0.5)) / (total as f64) * 100.0
    }
}

/// Coverage tracker
pub struct CoverageTracker {
    registry: FeatureRegistry,
    test_results: HashMap<String, bool>, // scenario -> passed
}

impl CoverageTracker {
    pub fn new(registry: FeatureRegistry) -> Self {
        Self {
            registry,
            test_results: HashMap::new(),
        }
    }

    pub fn record_test_result(&mut self, scenario: String, passed: bool) {
        self.test_results.insert(scenario, passed);
    }

    pub fn generate_report(&self) -> CoverageReport {
        let counts = self.registry.count_by_status();
        let coverage = self.registry.coverage_percentage();

        CoverageReport {
            total_features: self.registry.all_features().len(),
            implemented: counts
                .get(&FeatureStatus::Implemented)
                .copied()
                .unwrap_or(0),
            partial: counts.get(&FeatureStatus::Partial).copied().unwrap_or(0),
            unimplemented: counts
                .get(&FeatureStatus::Unimplemented)
                .copied()
                .unwrap_or(0),
            coverage_percentage: coverage,
            features: self
                .registry
                .all_features()
                .iter()
                .map(|f| (*f).clone())
                .collect(),
        }
    }
}

/// Coverage report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverageReport {
    pub total_features: usize,
    pub implemented: usize,
    pub partial: usize,
    pub unimplemented: usize,
    pub coverage_percentage: f64,
    pub features: Vec<FeatureDefinition>,
}

impl CoverageReport {
    pub fn to_markdown(&self) -> String {
        let mut md = String::new();
        md.push_str("# Feature Coverage Report\n\n");
        md.push_str(&format!("**Total Features**: {}\n", self.total_features));
        md.push_str(&format!("**Implemented**: {}\n", self.implemented));
        md.push_str(&format!("**Partial**: {}\n", self.partial));
        md.push_str(&format!("**Unimplemented**: {}\n", self.unimplemented));
        md.push_str(&format!(
            "**Coverage**: {:.1}%\n\n",
            self.coverage_percentage
        ));

        md.push_str("## Features by Status\n\n");
        md.push_str("### Implemented\n");
        for feature in &self.features {
            if matches!(feature.status, FeatureStatus::Implemented) {
                md.push_str(&format!("- {}: {}\n", feature.id, feature.name));
            }
        }

        md.push_str("\n### Partial\n");
        for feature in &self.features {
            if matches!(feature.status, FeatureStatus::Partial) {
                md.push_str(&format!("- {}: {}\n", feature.id, feature.name));
            }
        }

        md.push_str("\n### Unimplemented\n");
        for feature in &self.features {
            if matches!(feature.status, FeatureStatus::Unimplemented) {
                md.push_str(&format!("- {}: {}\n", feature.id, feature.name));
            }
        }

        md
    }
}
