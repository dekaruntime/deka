use std::collections::HashSet;

#[derive(Debug, Clone)]
pub struct ImportSpec {
    pub imported: String,
    pub local: String,
}

#[derive(Debug, Clone)]
pub struct ImportDecl {
    pub from: String,
    pub specs: Vec<ImportSpec>,
}

#[derive(Debug, Clone)]
pub struct SourceModuleMeta {
    pub imports: Vec<ImportDecl>,
    pub exported_functions: HashSet<String>,
    pub export_specs: Vec<ImportSpec>,
}

impl SourceModuleMeta {
    pub fn empty() -> Self {
        Self {
            imports: Vec::new(),
            exported_functions: HashSet::new(),
            export_specs: Vec::new(),
        }
    }
}
