use std::collections::HashMap;
use std::path::Path;
use std::sync::LazyLock;

use tree_sitter::{Node, Query};

use crate::models::{AnnotationInfo, FieldInfo, FunctionParamInfo};
use crate::parser::capture::CallCaptureMatch;
use crate::parser::languages::{go, java, javascript, python};
use crate::parser::ParseContext;

pub struct LanguageInfo {
    pub name: &'static str,
    pub extensions: &'static [&'static str],
    pub function_query: &'static str,
    pub class_query: &'static str,
    pub call_query: &'static str,
    pub import_query: &'static str,
}

pub struct LanguageQueries {
    pub function_query: &'static str,
    pub class_query: &'static str,
    pub call_query: &'static str,
    pub import_query: &'static str,
}

pub struct ResolvedCall<'a> {
    pub callee: String,
    pub is_method: bool,
    pub object_name: Option<String>,
    pub callee_function_node: Option<Node<'a>>,
}

pub trait LanguageEngine: Sync {
    fn language_info(&self) -> &'static LanguageInfo;

    fn id(&self) -> &'static str {
        self.language_info().name
    }

    fn ts_language(&self) -> tree_sitter::Language {
        find_language(self.id()).expect("supported language")
    }

    fn queries(&self) -> LanguageQueries {
        let info = self.language_info();
        LanguageQueries {
            function_query: info.function_query,
            class_query: info.class_query,
            call_query: info.call_query,
            import_query: info.import_query,
        }
    }

    fn normalize_function_node<'a>(&self, _ctx: &ParseContext, node: Node<'a>) -> Node<'a> {
        node
    }

    fn normalize_class_node<'a>(&self, _ctx: &ParseContext, node: Node<'a>) -> Node<'a> {
        node
    }

    fn function_name(&self, ctx: &ParseContext, node: Node<'_>) -> Option<String>;

    fn function_params(
        &self,
        ctx: &ParseContext,
        function_node: Node<'_>,
    ) -> Vec<FunctionParamInfo>;

    fn resolve_call<'a>(
        &self,
        ctx: &ParseContext,
        matched: &CallCaptureMatch<'a>,
    ) -> ResolvedCall<'a>;

    fn is_ref_node(&self, node: Node<'_>) -> bool;

    fn class_fields(
        &self,
        ctx: &ParseContext,
        class_node: Node<'_>,
        class_name: &str,
    ) -> Vec<FieldInfo>;

    fn super_types(&self, ctx: &ParseContext, class_node: Node<'_>) -> Vec<String>;

    fn annotations(&self, _ctx: &ParseContext) -> Vec<AnnotationInfo> {
        Vec::new()
    }

    fn include_call(
        &self,
        _ctx: &ParseContext,
        _call_node: Node<'_>,
        _callee: &str,
        _object_name: Option<&str>,
    ) -> bool {
        true
    }

    fn ref_filter(&self, _ctx: &ParseContext, _node: Node<'_>) -> bool {
        true
    }

    fn enclosing_class_name(&self, _ctx: &ParseContext, _node: Node<'_>) -> Option<String> {
        None
    }
}

#[derive(Clone, Copy)]
pub enum QueryKind {
    Function,
    Class,
    Call,
    Import,
}

static PYTHON_INFO: LanguageInfo = LanguageInfo {
    name: "python",
    extensions: &[".py", ".pyw", ".pyi"],
    function_query: r#"[(function_definition name: (identifier) @name) @function (decorated_definition definition: (function_definition name: (identifier) @name)) @function]"#,
    class_query: r#"[(class_definition name: (identifier) @name) @class (decorated_definition definition: (class_definition name: (identifier) @name)) @class]"#,
    call_query: r#"[(call function: (identifier) @callee) (call function: (attribute object: (_) @object attribute: (identifier) @method))] @call"#,
    import_query: r#"[(import_statement name: (dotted_name) @module) (import_from_statement module_name: (dotted_name) @module) (import_from_statement module_name: (relative_import) @module)] @import"#,
};

static JAVASCRIPT_INFO: LanguageInfo = LanguageInfo {
    name: "javascript",
    extensions: &[".js", ".mjs", ".cjs", ".jsx"],
    function_query: r#"[(function_declaration name: (identifier) @name) (generator_function_declaration name: (identifier) @name) (method_definition name: [(property_identifier) (private_property_identifier)] @name) (function_expression name: (identifier) @name) (variable_declarator name: (identifier) @name value: [(arrow_function) (function_expression)])] @function"#,
    class_query: r#"[(class_declaration name: (identifier) @name) (class name: (identifier) @name) (variable_declarator name: (identifier) @name value: (class))] @class"#,
    call_query: r#"[(call_expression function: (identifier) @callee) (call_expression function: (member_expression object: (_) @object property: [(property_identifier) (private_property_identifier)] @method)) (new_expression constructor: (identifier) @callee) (new_expression constructor: (member_expression object: (_) @object property: [(property_identifier) (private_property_identifier)] @method))] @call"#,
    import_query: r#"[(import_statement source: (string) @module) (export_statement source: (string) @module) ((call_expression function: (identifier) @_callee arguments: (arguments (string) @module)) @import (#match? @_callee "^(require|import)$")) ((call_expression function: (import) arguments: (arguments (string) @module)) @import)]"#,
};

static TYPESCRIPT_INFO: LanguageInfo = LanguageInfo {
    name: "typescript",
    extensions: &[".ts"],
    function_query: r#"[(function_declaration name: (identifier) @name) (generator_function_declaration name: (identifier) @name) (method_definition name: [(property_identifier) (private_property_identifier)] @name) (function_expression name: (identifier) @name) (variable_declarator name: (identifier) @name value: [(arrow_function) (function_expression)])] @function"#,
    class_query: r#"[(class_declaration name: (type_identifier) @name) (abstract_class_declaration name: (type_identifier) @name) (interface_declaration name: (type_identifier) @name) (type_alias_declaration name: (type_identifier) @name) (enum_declaration name: (identifier) @name) (variable_declarator name: (identifier) @name value: (class))] @class"#,
    call_query: r#"[(call_expression function: (identifier) @callee) (call_expression function: (member_expression object: (_) @object property: [(property_identifier) (private_property_identifier)] @method)) (new_expression constructor: (identifier) @callee) (new_expression constructor: (member_expression object: (_) @object property: [(property_identifier) (private_property_identifier)] @method))] @call"#,
    import_query: r#"[(import_statement source: (string) @module) (import_statement (import_require_clause source: (string) @module)) (export_statement source: (string) @module) ((call_expression function: (identifier) @_callee arguments: (arguments (string) @module)) @import (#match? @_callee "^(require|import)$")) ((call_expression function: (import) arguments: (arguments (string) @module)) @import)]"#,
};

static TSX_INFO: LanguageInfo = LanguageInfo {
    name: "tsx",
    extensions: &[".tsx"],
    function_query: r#"[(function_declaration name: (identifier) @name) (method_definition name: [(property_identifier) (private_property_identifier)] @name) (function_expression name: (identifier) @name) (variable_declarator name: (identifier) @name value: [(arrow_function) (function_expression)])] @function"#,
    class_query: r#"[(class_declaration name: (type_identifier) @name) (abstract_class_declaration name: (type_identifier) @name) (interface_declaration name: (type_identifier) @name) (type_alias_declaration name: (type_identifier) @name) (enum_declaration name: (identifier) @name) (variable_declarator name: (identifier) @name value: (class))] @class"#,
    call_query: r#"[(call_expression function: (identifier) @callee) (call_expression function: (member_expression object: (_) @object property: [(property_identifier) (private_property_identifier)] @method)) (new_expression constructor: (identifier) @callee) (new_expression constructor: (member_expression object: (_) @object property: [(property_identifier) (private_property_identifier)] @method))] @call"#,
    import_query: r#"[(import_statement source: (string) @module) (import_statement (import_require_clause source: (string) @module)) (export_statement source: (string) @module) ((call_expression function: (identifier) @_callee arguments: (arguments (string) @module)) @import (#match? @_callee "^(require|import)$")) ((call_expression function: (import) arguments: (arguments (string) @module)) @import)]"#,
};

static JAVA_INFO: LanguageInfo = LanguageInfo {
    name: "java",
    extensions: &[".java"],
    function_query: r#"[(method_declaration name: (identifier) @name) (constructor_declaration name: (identifier) @name)] @function"#,
    class_query: r#"[(class_declaration name: (identifier) @name) (interface_declaration name: (identifier) @name) (enum_declaration name: (identifier) @name) (record_declaration name: (identifier) @name) (annotation_type_declaration name: (identifier) @name)] @class"#,
    call_query: r#"[(method_invocation name: (identifier) @callee) (method_invocation object: (_) @object name: (identifier) @method) (object_creation_expression type: (_) @callee) (explicit_constructor_invocation)] @call"#,
    import_query: r#"[(import_declaration (scoped_identifier) @module) (import_declaration (identifier) @module)] @import"#,
};

static GO_INFO: LanguageInfo = LanguageInfo {
    name: "go",
    extensions: &[".go"],
    function_query: r#"[(function_declaration name: (identifier) @name) (method_declaration name: (field_identifier) @name)] @function"#,
    class_query: r#"(type_declaration (type_spec name: (type_identifier) @name type: [(struct_type) (interface_type)])) @class"#,
    call_query: r#"[(call_expression function: (identifier) @callee) (call_expression function: (selector_expression operand: (_) @object field: (field_identifier) @method))] @call"#,
    import_query: r#"(import_spec path: [(interpreted_string_literal) (raw_string_literal)] @module) @import"#,
};

struct CompiledQueries {
    function_query: Query,
    class_query: Query,
    call_query: Query,
    import_query: Query,
}

impl CompiledQueries {
    fn get(&self, kind: QueryKind) -> &Query {
        match kind {
            QueryKind::Function => &self.function_query,
            QueryKind::Class => &self.class_query,
            QueryKind::Call => &self.call_query,
            QueryKind::Import => &self.import_query,
        }
    }
}

fn compile_queries(engine: &'static dyn LanguageEngine) -> CompiledQueries {
    let queries = engine.queries();
    let language = engine.ts_language();
    CompiledQueries {
        function_query: Query::new(&language, queries.function_query)
            .expect("valid function query"),
        class_query: Query::new(&language, queries.class_query).expect("valid class query"),
        call_query: Query::new(&language, queries.call_query).expect("valid call query"),
        import_query: Query::new(&language, queries.import_query).expect("valid import query"),
    }
}

static PYTHON_QUERIES: LazyLock<CompiledQueries> =
    LazyLock::new(|| compile_queries(&python::PYTHON_ENGINE));
static JAVASCRIPT_QUERIES: LazyLock<CompiledQueries> =
    LazyLock::new(|| compile_queries(&javascript::JAVASCRIPT_ENGINE));
static TYPESCRIPT_QUERIES: LazyLock<CompiledQueries> =
    LazyLock::new(|| compile_queries(&javascript::TYPESCRIPT_ENGINE));
static TSX_QUERIES: LazyLock<CompiledQueries> =
    LazyLock::new(|| compile_queries(&javascript::TSX_ENGINE));
static JAVA_QUERIES: LazyLock<CompiledQueries> =
    LazyLock::new(|| compile_queries(&java::JAVA_ENGINE));
static GO_QUERIES: LazyLock<CompiledQueries> =
    LazyLock::new(|| compile_queries(&go::GO_ENGINE));

struct LanguageRegistryEntry {
    info: &'static LanguageInfo,
    engine: &'static dyn LanguageEngine,
    ts_language: fn() -> tree_sitter::Language,
    compiled_queries: &'static LazyLock<CompiledQueries>,
}

static LANGUAGE_REGISTRY: &[LanguageRegistryEntry] = &[
    LanguageRegistryEntry {
        info: &PYTHON_INFO,
        engine: &python::PYTHON_ENGINE,
        ts_language: || tree_sitter_python::LANGUAGE.into(),
        compiled_queries: &PYTHON_QUERIES,
    },
    LanguageRegistryEntry {
        info: &JAVASCRIPT_INFO,
        engine: &javascript::JAVASCRIPT_ENGINE,
        ts_language: || tree_sitter_javascript::LANGUAGE.into(),
        compiled_queries: &JAVASCRIPT_QUERIES,
    },
    LanguageRegistryEntry {
        info: &TYPESCRIPT_INFO,
        engine: &javascript::TYPESCRIPT_ENGINE,
        ts_language: || tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        compiled_queries: &TYPESCRIPT_QUERIES,
    },
    LanguageRegistryEntry {
        info: &TSX_INFO,
        engine: &javascript::TSX_ENGINE,
        ts_language: || tree_sitter_typescript::LANGUAGE_TSX.into(),
        compiled_queries: &TSX_QUERIES,
    },
    LanguageRegistryEntry {
        info: &JAVA_INFO,
        engine: &java::JAVA_ENGINE,
        ts_language: || tree_sitter_java::LANGUAGE.into(),
        compiled_queries: &JAVA_QUERIES,
    },
    LanguageRegistryEntry {
        info: &GO_INFO,
        engine: &go::GO_ENGINE,
        ts_language: || tree_sitter_go::LANGUAGE.into(),
        compiled_queries: &GO_QUERIES,
    },
];

fn find_registry_entry(name: &str) -> Option<&'static LanguageRegistryEntry> {
    LANGUAGE_REGISTRY.iter().find(|entry| entry.info.name == name)
}

fn find_registry_entry_by_extension(file_path: &Path) -> Option<&'static LanguageRegistryEntry> {
    let ext = file_path.extension()?.to_str()?;
    let dotted = format!(".{ext}");
    FILE_EXTENSION_MAP.get(dotted.as_str()).copied()
}

static FILE_EXTENSION_MAP: LazyLock<HashMap<&'static str, &'static LanguageRegistryEntry>> =
    LazyLock::new(|| {
    let mut map = HashMap::new();
    for entry in LANGUAGE_REGISTRY {
        for extension in entry.info.extensions {
            map.insert(*extension, entry);
        }
    }
    map
});

static SUPPORTED_EXTENSIONS: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    let mut extensions = FILE_EXTENSION_MAP.keys().copied().collect::<Vec<_>>();
    extensions.sort();
    extensions
});

pub fn detect_language(file_path: &Path) -> Option<&'static str> {
    find_registry_entry_by_extension(file_path).map(|entry| entry.info.name)
}

pub fn find_language(name: &str) -> Option<tree_sitter::Language> {
    find_registry_entry(name).map(|entry| (entry.ts_language)())
}

pub fn find_language_info(name: &str) -> Option<&'static LanguageInfo> {
    find_registry_entry(name).map(|entry| entry.info)
}

pub fn detect_language_engine(path: &Path) -> Option<&'static dyn LanguageEngine> {
    find_registry_entry_by_extension(path).map(|entry| entry.engine)
}

pub fn supported_extensions() -> &'static [&'static str] {
    &SUPPORTED_EXTENSIONS
}

pub fn language_extensions(name: &str) -> Option<&'static [&'static str]> {
    find_registry_entry(name).map(|entry| entry.info.extensions)
}

pub fn compiled_query(language: &str, kind: QueryKind) -> Option<&'static Query> {
    find_registry_entry(language).map(|entry| entry.compiled_queries.get(kind))
}
