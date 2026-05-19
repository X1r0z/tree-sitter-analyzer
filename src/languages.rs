use std::collections::HashMap;
use std::path::Path;
use std::sync::LazyLock;

use tree_sitter::{Node, Query};

use crate::models::{AnnotationInfo, FieldInfo, FunctionParamInfo};
use crate::parser::capture::CallCaptureMatch;
use crate::parser::languages::{go, java, javascript, python};
use crate::parser::{EnclosingContext, ParseContext};

pub struct LanguageInfo {
    pub name: &'static str,
    pub extensions: &'static [&'static str],
    pub(crate) engine: &'static dyn LanguageEngine,
    pub(crate) ts_language: fn() -> tree_sitter::Language,
    pub function_query: &'static str,
    pub class_query: &'static str,
    pub call_query: &'static str,
    pub import_query: &'static str,
}

impl LanguageInfo {
    pub(crate) fn engine(&self) -> &'static dyn LanguageEngine {
        self.engine
    }

    pub(crate) fn ts_language(&self) -> tree_sitter::Language {
        (self.ts_language)()
    }

    pub(crate) fn query(&self, kind: QueryKind) -> &'static Query {
        COMPILED_QUERY_MAP
            .get(self.name)
            .expect("compiled queries for supported language")
            .get(kind)
    }
}

pub struct ResolvedCall<'a> {
    pub callee: String,
    pub is_method: bool,
    pub object_name: Option<String>,
    pub callee_function_node: Option<Node<'a>>,
}

pub trait LanguageEngine: Sync {
    fn id(&self) -> &'static str;

    fn language_info(&self) -> &'static LanguageInfo {
        language_for_name(self.id()).expect("supported language")
    }

    fn ts_language(&self) -> tree_sitter::Language {
        language_for_name(self.language_info().name)
            .expect("supported language")
            .ts_language()
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
        enclosing: &EnclosingContext<'a>,
    ) -> ResolvedCall<'a>;

    fn resolve_call_targets(
        &self,
        ctx: &ParseContext,
        function_node: Node<'_>,
        call_node: Node<'_>,
        identifier_name: &str,
    ) -> Vec<String> {
        let _ = (ctx, function_node, call_node, identifier_name);
        Vec::new()
    }

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
    engine: &python::PYTHON_ENGINE,
    ts_language: || tree_sitter_python::LANGUAGE.into(),
    function_query: r"[(function_definition name: (identifier) @name) @function (decorated_definition definition: (function_definition name: (identifier) @name)) @function]",
    class_query: r"[(class_definition name: (identifier) @name) @class (decorated_definition definition: (class_definition name: (identifier) @name)) @class]",
    call_query: r"[(call function: (identifier) @callee) (call function: (attribute object: (_) @object attribute: (identifier) @method))] @call",
    import_query: r"[(import_statement name: (dotted_name) @module) (import_from_statement module_name: (dotted_name) @module) (import_from_statement module_name: (relative_import) @module)] @import",
};

static JAVASCRIPT_INFO: LanguageInfo = LanguageInfo {
    name: "javascript",
    extensions: &[".js", ".mjs", ".cjs", ".jsx"],
    engine: &javascript::JAVASCRIPT_ENGINE,
    ts_language: || tree_sitter_javascript::LANGUAGE.into(),
    function_query: r"[(function_declaration name: (identifier) @name) (generator_function_declaration name: (identifier) @name) (method_definition name: [(property_identifier) (private_property_identifier)] @name) (function_expression name: (identifier) @name) (variable_declarator name: (identifier) @name value: [(arrow_function) (function_expression)])] @function",
    class_query: r"[(class_declaration name: (identifier) @name) (class name: (identifier) @name) (variable_declarator name: (identifier) @name value: (class))] @class",
    call_query: r"[(call_expression function: (identifier) @callee) (call_expression function: (super)) @call (call_expression function: (member_expression object: (_) @object property: [(property_identifier) (private_property_identifier)] @method)) (new_expression constructor: (identifier) @callee) (new_expression constructor: (member_expression object: (_) @object property: [(property_identifier) (private_property_identifier)] @method))] @call",
    import_query: r#"[(import_statement source: (string) @module) (export_statement source: (string) @module) ((call_expression function: (identifier) @_callee arguments: (arguments (string) @module)) @import (#match? @_callee "^(require|import)$")) ((call_expression function: (import) arguments: (arguments (string) @module)) @import)]"#,
};

static TYPESCRIPT_INFO: LanguageInfo = LanguageInfo {
    name: "typescript",
    extensions: &[".ts"],
    engine: &javascript::TYPESCRIPT_ENGINE,
    ts_language: || tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
    function_query: r"[(function_declaration name: (identifier) @name) (generator_function_declaration name: (identifier) @name) (method_definition name: [(property_identifier) (private_property_identifier)] @name) (function_expression name: (identifier) @name) (variable_declarator name: (identifier) @name value: [(arrow_function) (function_expression)])] @function",
    class_query: r"[(class_declaration name: (type_identifier) @name) (abstract_class_declaration name: (type_identifier) @name) (interface_declaration name: (type_identifier) @name) (type_alias_declaration name: (type_identifier) @name) (enum_declaration name: (identifier) @name) (variable_declarator name: (identifier) @name value: (class))] @class",
    call_query: r"[(call_expression function: (identifier) @callee) (call_expression function: (super)) @call (call_expression function: (member_expression object: (_) @object property: [(property_identifier) (private_property_identifier)] @method)) (new_expression constructor: (identifier) @callee) (new_expression constructor: (member_expression object: (_) @object property: [(property_identifier) (private_property_identifier)] @method))] @call",
    import_query: r#"[(import_statement source: (string) @module) (import_statement (import_require_clause source: (string) @module)) (export_statement source: (string) @module) ((call_expression function: (identifier) @_callee arguments: (arguments (string) @module)) @import (#match? @_callee "^(require|import)$")) ((call_expression function: (import) arguments: (arguments (string) @module)) @import)]"#,
};

static TSX_INFO: LanguageInfo = LanguageInfo {
    name: "tsx",
    extensions: &[".tsx"],
    engine: &javascript::TSX_ENGINE,
    ts_language: || tree_sitter_typescript::LANGUAGE_TSX.into(),
    function_query: r"[(function_declaration name: (identifier) @name) (method_definition name: [(property_identifier) (private_property_identifier)] @name) (function_expression name: (identifier) @name) (variable_declarator name: (identifier) @name value: [(arrow_function) (function_expression)])] @function",
    class_query: r"[(class_declaration name: (type_identifier) @name) (abstract_class_declaration name: (type_identifier) @name) (interface_declaration name: (type_identifier) @name) (type_alias_declaration name: (type_identifier) @name) (enum_declaration name: (identifier) @name) (variable_declarator name: (identifier) @name value: (class))] @class",
    call_query: r"[(call_expression function: (identifier) @callee) (call_expression function: (super)) @call (call_expression function: (member_expression object: (_) @object property: [(property_identifier) (private_property_identifier)] @method)) (new_expression constructor: (identifier) @callee) (new_expression constructor: (member_expression object: (_) @object property: [(property_identifier) (private_property_identifier)] @method))] @call",
    import_query: r#"[(import_statement source: (string) @module) (import_statement (import_require_clause source: (string) @module)) (export_statement source: (string) @module) ((call_expression function: (identifier) @_callee arguments: (arguments (string) @module)) @import (#match? @_callee "^(require|import)$")) ((call_expression function: (import) arguments: (arguments (string) @module)) @import)]"#,
};

static JAVA_INFO: LanguageInfo = LanguageInfo {
    name: "java",
    extensions: &[".java"],
    engine: &java::JAVA_ENGINE,
    ts_language: || tree_sitter_java::LANGUAGE.into(),
    function_query: r"[(method_declaration name: (identifier) @name) (constructor_declaration name: (identifier) @name)] @function",
    class_query: r"[(class_declaration name: (identifier) @name) (interface_declaration name: (identifier) @name) (enum_declaration name: (identifier) @name) (record_declaration name: (identifier) @name) (annotation_type_declaration name: (identifier) @name)] @class",
    call_query: r"[(method_invocation name: (identifier) @callee) (method_invocation object: (_) @object name: (identifier) @method) (object_creation_expression type: (_) @callee) (explicit_constructor_invocation)] @call",
    import_query: r"[(import_declaration (scoped_identifier) @module) (import_declaration (identifier) @module)] @import",
};

static GO_INFO: LanguageInfo = LanguageInfo {
    name: "go",
    extensions: &[".go"],
    engine: &go::GO_ENGINE,
    ts_language: || tree_sitter_go::LANGUAGE.into(),
    function_query: r"[(function_declaration name: (identifier) @name) (method_declaration name: (field_identifier) @name)] @function",
    class_query: r"(type_declaration (type_spec name: (type_identifier) @name type: [(struct_type) (interface_type)])) @class",
    call_query: r"[(call_expression function: (identifier) @callee) (call_expression function: (selector_expression operand: (_) @object field: (field_identifier) @method))] @call",
    import_query: r"(import_spec path: [(interpreted_string_literal) (raw_string_literal)] @module) @import",
};

static LANGUAGE_REGISTRY: &[&LanguageInfo] = &[
    &PYTHON_INFO,
    &JAVASCRIPT_INFO,
    &TYPESCRIPT_INFO,
    &TSX_INFO,
    &JAVA_INFO,
    &GO_INFO,
];

static COMPILED_QUERY_MAP: LazyLock<HashMap<&'static str, CompiledQueries>> = LazyLock::new(|| {
    LANGUAGE_REGISTRY
        .iter()
        .map(|info| {
            let language = info.ts_language();
            (
                info.name,
                CompiledQueries {
                    function: Query::new(&language, info.function_query)
                        .expect("valid function query"),
                    class: Query::new(&language, info.class_query).expect("valid class query"),
                    call: Query::new(&language, info.call_query).expect("valid call query"),
                    import: Query::new(&language, info.import_query).expect("valid import query"),
                },
            )
        })
        .collect()
});

struct CompiledQueries {
    function: Query,
    class: Query,
    call: Query,
    import: Query,
}

impl CompiledQueries {
    fn get(&self, kind: QueryKind) -> &Query {
        match kind {
            QueryKind::Function => &self.function,
            QueryKind::Class => &self.class,
            QueryKind::Call => &self.call,
            QueryKind::Import => &self.import,
        }
    }
}

pub fn detect_language(file_path: &Path) -> Option<&'static LanguageInfo> {
    let ext = file_path.extension()?.to_str()?;
    LANGUAGE_REGISTRY.iter().copied().find(|info| {
        info.extensions
            .iter()
            .any(|candidate| candidate.strip_prefix('.') == Some(ext))
    })
}

pub(crate) fn language_for_name(name: &str) -> Option<&'static LanguageInfo> {
    LANGUAGE_REGISTRY
        .iter()
        .copied()
        .find(|info| info.name == name)
}

pub fn supported_language_names() -> Vec<&'static str> {
    let mut names = LANGUAGE_REGISTRY
        .iter()
        .map(|info| info.name)
        .collect::<Vec<_>>();
    names.sort_unstable();
    names
}
