use crate::modules::project_creator::models::*;

pub trait PackRegistry: Send + Sync {
    fn list_packs(&self, kind: Option<PackKind>) -> Vec<PackInfo>;
    fn get_pack(&self, id: &str) -> Option<PackInfo>;
}

pub struct DefaultPackRegistry;

impl DefaultPackRegistry {
    pub fn new() -> Self {
        Self
    }
}

impl PackRegistry for DefaultPackRegistry {
    fn list_packs(&self, _kind: Option<PackKind>) -> Vec<PackInfo> {
        Vec::new()
    }

    fn get_pack(&self, _id: &str) -> Option<PackInfo> {
        None
    }
}
