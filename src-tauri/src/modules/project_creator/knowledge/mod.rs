use crate::modules::project_creator::models::*;

pub trait KnowledgeBase: Send + Sync {
    fn get_entry(&self, key: &str) -> Option<KnowledgeEntry>;
    fn search(&self, query: &str) -> Vec<KnowledgeEntry>;
}

pub struct DefaultKnowledgeBase;

impl DefaultKnowledgeBase {
    pub fn new() -> Self {
        Self
    }
}

impl KnowledgeBase for DefaultKnowledgeBase {
    fn get_entry(&self, _key: &str) -> Option<KnowledgeEntry> {
        None
    }

    fn search(&self, _query: &str) -> Vec<KnowledgeEntry> {
        Vec::new()
    }
}
