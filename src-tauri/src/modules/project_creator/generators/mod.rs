use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use crate::modules::project_creator::models::*;

pub trait Generator: Send + Sync {
    fn id(&self) -> &str;
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn generate(
        &self,
        context: &WizardContext,
        project_path: &Path,
        config: &serde_json::Value,
    ) -> Result<GenerationReport, String>;
}

pub struct GeneratorRegistry {
    generators: HashMap<String, Arc<dyn Generator>>,
}

impl GeneratorRegistry {
    pub fn new() -> Self {
        Self {
            generators: HashMap::new(),
        }
    }

    pub fn register(&mut self, generator: Arc<dyn Generator>) {
        let id = generator.id().to_string();
        self.generators.insert(id, generator);
    }

    pub fn get(&self, id: &str) -> Option<Arc<dyn Generator>> {
        self.generators.get(id).cloned()
    }

    pub fn list_descriptors(&self) -> Vec<GeneratorDescriptor> {
        self.generators
            .values()
            .map(|g| GeneratorDescriptor {
                id: g.id().to_string(),
                name: g.name().to_string(),
                description: g.description().to_string(),
            })
            .collect()
    }
}

/// Заглушка для CLI-генератора (будет реализован в Milestone 4)
pub struct CliGenerator;

impl Generator for CliGenerator {
    fn id(&self) -> &str {
        "cli"
    }
    fn name(&self) -> &str {
        "CLI Command"
    }
    fn description(&self) -> &str {
        "Executes a CLI command to generate project scaffolding"
    }
    fn generate(
        &self,
        _context: &WizardContext,
        _project_path: &Path,
        _config: &serde_json::Value,
    ) -> Result<GenerationReport, String> {
        Err("CliGenerator not yet implemented — Milestone 4".to_string())
    }
}
