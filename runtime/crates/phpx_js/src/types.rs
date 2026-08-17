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
    /// Line indices (0-based, inclusive) of the frontmatter delimiters when the
    /// file uses `---` frontmatter. `template_start_line` is the first line
    /// after the closing delimiter; the rest of the file is the template body.
    pub frontmatter_start_line: Option<usize>,
    pub frontmatter_end_line: Option<usize>,
    pub template_start_line: Option<usize>,
    /// True when the source is being compiled as DekaScript (.ds). The JS
    /// emitter uses this to inject the DekaScript runtime prelude (safe
    /// globals, deka.unsafe, etc.) as required by RFD 21.
    pub is_ds: bool,
}

impl SourceModuleMeta {
    pub fn empty() -> Self {
        Self {
            imports: Vec::new(),
            exported_functions: HashSet::new(),
            export_specs: Vec::new(),
            frontmatter_start_line: None,
            frontmatter_end_line: None,
            template_start_line: None,
            is_ds: false,
        }
    }
}
