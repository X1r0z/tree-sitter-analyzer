use std::collections::HashMap;
use std::path::Path;
use std::sync::LazyLock;

#[allow(dead_code)]
pub struct LanguageInfo {
    pub name: &'static str,
    pub extensions: &'static [&'static str],
    pub function_query: &'static str,
    pub class_query: &'static str,
    pub call_query: &'static str,
    pub import_query: &'static str,
    pub field_query: &'static str,
}

static PYTHON_INFO: LanguageInfo = LanguageInfo {
    name: "python",
    extensions: &[".py", ".pyw", ".pyi"],
    function_query: r#"[(function_definition name: (identifier) @name) @function (decorated_definition definition: (function_definition name: (identifier) @name)) @function]"#,
    class_query: r#"[(class_definition name: (identifier) @name) @class (decorated_definition definition: (class_definition name: (identifier) @name)) @class]"#,
    call_query: r#"[(call function: (identifier) @callee) (call function: (attribute object: (_) @object attribute: (identifier) @method))] @call"#,
    import_query: r#"[(import_statement name: (dotted_name) @module) (import_from_statement module_name: (dotted_name) @module) (import_from_statement module_name: (relative_import) @module)] @import"#,
    field_query: r#"(class_definition body: (block (expression_statement (assignment left: (identifier) @name type: (type)? @type) @field)))"#,
};

static JAVASCRIPT_INFO: LanguageInfo = LanguageInfo {
    name: "javascript",
    extensions: &[".js", ".mjs", ".cjs", ".jsx"],
    function_query: r#"[(function_declaration name: (identifier) @name) (generator_function_declaration name: (identifier) @name) (method_definition name: [(property_identifier) (private_property_identifier)] @name) (function_expression name: (identifier) @name) (variable_declarator name: (identifier) @name value: [(arrow_function) (function_expression)])] @function"#,
    class_query: r#"[(class_declaration name: (identifier) @name) (class name: (identifier) @name) (variable_declarator name: (identifier) @name value: (class))] @class"#,
    call_query: r#"[(call_expression function: (identifier) @callee) (call_expression function: (member_expression object: (_) @object property: [(property_identifier) (private_property_identifier)] @method)) (new_expression constructor: (identifier) @callee) (new_expression constructor: (member_expression object: (_) @object property: [(property_identifier) (private_property_identifier)] @method))] @call"#,
    import_query: r#"[(import_statement source: (string) @module) (export_statement source: (string) @module) ((call_expression function: (identifier) @_callee arguments: (arguments (string) @module)) @import (#match? @_callee "^(require|import)$")) ((call_expression function: (import) arguments: (arguments (string) @module)) @import)]"#,
    field_query: r#"(field_definition property: [(property_identifier) (private_property_identifier)] @name) @field"#,
};

static TYPESCRIPT_INFO: LanguageInfo = LanguageInfo {
    name: "typescript",
    extensions: &[".ts"],
    function_query: r#"[(function_declaration name: (identifier) @name) (generator_function_declaration name: (identifier) @name) (method_definition name: [(property_identifier) (private_property_identifier)] @name) (function_expression name: (identifier) @name) (variable_declarator name: (identifier) @name value: [(arrow_function) (function_expression)])] @function"#,
    class_query: r#"[(class_declaration name: (type_identifier) @name) (abstract_class_declaration name: (type_identifier) @name) (interface_declaration name: (type_identifier) @name) (type_alias_declaration name: (type_identifier) @name) (enum_declaration name: (identifier) @name) (variable_declarator name: (identifier) @name value: (class))] @class"#,
    call_query: r#"[(call_expression function: (identifier) @callee) (call_expression function: (member_expression object: (_) @object property: [(property_identifier) (private_property_identifier)] @method)) (new_expression constructor: (identifier) @callee) (new_expression constructor: (member_expression object: (_) @object property: [(property_identifier) (private_property_identifier)] @method))] @call"#,
    import_query: r#"[(import_statement source: (string) @module) (import_statement (import_require_clause source: (string) @module)) (export_statement source: (string) @module) ((call_expression function: (identifier) @_callee arguments: (arguments (string) @module)) @import (#match? @_callee "^(require|import)$")) ((call_expression function: (import) arguments: (arguments (string) @module)) @import)]"#,
    field_query: r#"[(public_field_definition name: [(property_identifier) (private_property_identifier)] @name type: (type_annotation)? @type) (property_signature name: (property_identifier) @name type: (type_annotation)? @type)] @field"#,
};

static TSX_INFO: LanguageInfo = LanguageInfo {
    name: "tsx",
    extensions: &[".tsx"],
    function_query: r#"[(function_declaration name: (identifier) @name) (method_definition name: [(property_identifier) (private_property_identifier)] @name) (function_expression name: (identifier) @name) (variable_declarator name: (identifier) @name value: [(arrow_function) (function_expression)])] @function"#,
    class_query: r#"[(class_declaration name: (type_identifier) @name) (interface_declaration name: (type_identifier) @name) (type_alias_declaration name: (type_identifier) @name) (enum_declaration name: (identifier) @name) (variable_declarator name: (identifier) @name value: (class))] @class"#,
    call_query: r#"[(call_expression function: (identifier) @callee) (call_expression function: (member_expression object: (_) @object property: [(property_identifier) (private_property_identifier)] @method)) (new_expression constructor: (identifier) @callee) (new_expression constructor: (member_expression object: (_) @object property: [(property_identifier) (private_property_identifier)] @method))] @call"#,
    import_query: r#"[(import_statement source: (string) @module) (import_statement (import_require_clause source: (string) @module)) (export_statement source: (string) @module) ((call_expression function: (identifier) @_callee arguments: (arguments (string) @module)) @import (#match? @_callee "^(require|import)$")) ((call_expression function: (import) arguments: (arguments (string) @module)) @import)]"#,
    field_query: r#"[(public_field_definition name: [(property_identifier) (private_property_identifier)] @name type: (type_annotation)? @type) (property_signature name: (property_identifier) @name type: (type_annotation)? @type)] @field"#,
};

static JAVA_INFO: LanguageInfo = LanguageInfo {
    name: "java",
    extensions: &[".java"],
    function_query: r#"[(method_declaration name: (identifier) @name) (constructor_declaration name: (identifier) @name)] @function"#,
    class_query: r#"[(class_declaration name: (identifier) @name) (interface_declaration name: (identifier) @name) (enum_declaration name: (identifier) @name) (record_declaration name: (identifier) @name) (annotation_type_declaration name: (identifier) @name)] @class"#,
    call_query: r#"[(method_invocation name: (identifier) @callee) (method_invocation object: (_) @object name: (identifier) @method) (object_creation_expression type: (_) @callee) (explicit_constructor_invocation)] @call"#,
    import_query: r#"[(import_declaration (scoped_identifier) @module) (import_declaration (identifier) @module)] @import"#,
    field_query: r#"(field_declaration type: (_) @type (variable_declarator name: (identifier) @name)) @field"#,
};

static GO_INFO: LanguageInfo = LanguageInfo {
    name: "go",
    extensions: &[".go"],
    function_query: r#"[(function_declaration name: (identifier) @name) (method_declaration name: (field_identifier) @name)] @function"#,
    class_query: r#"(type_declaration (type_spec name: (type_identifier) @name type: [(struct_type) (interface_type)])) @class"#,
    call_query: r#"[(call_expression function: (identifier) @callee) (call_expression function: (selector_expression operand: (_) @object field: (field_identifier) @method))] @call"#,
    import_query: r#"(import_spec path: [(interpreted_string_literal) (raw_string_literal)] @module) @import"#,
    field_query: r#"(field_declaration name: (field_identifier) @name type: (_) @type) @field"#,
};

static LANGUAGE_INFOS: &[&LanguageInfo] = &[
    &PYTHON_INFO,
    &JAVASCRIPT_INFO,
    &TYPESCRIPT_INFO,
    &TSX_INFO,
    &JAVA_INFO,
    &GO_INFO,
];

static FILE_EXTENSION_MAP: LazyLock<HashMap<&'static str, &'static str>> = LazyLock::new(|| {
    let mut m = HashMap::new();
    m.insert(".py", "python");
    m.insert(".pyw", "python");
    m.insert(".pyi", "python");
    m.insert(".js", "javascript");
    m.insert(".mjs", "javascript");
    m.insert(".cjs", "javascript");
    m.insert(".jsx", "javascript");
    m.insert(".ts", "typescript");
    m.insert(".tsx", "tsx");
    m.insert(".java", "java");
    m.insert(".go", "go");
    m
});

static LANGUAGE_INFO_MAP: LazyLock<HashMap<&'static str, &'static LanguageInfo>> =
    LazyLock::new(|| {
        let mut m = HashMap::new();
        for info in LANGUAGE_INFOS {
            m.insert(info.name, *info);
        }
        m
    });

static SUPPORTED_EXTENSIONS: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    let mut exts: Vec<&'static str> = FILE_EXTENSION_MAP.keys().copied().collect();
    exts.sort();
    exts
});

pub fn detect_language(file_path: &Path) -> Option<&'static str> {
    let ext = file_path.extension()?.to_str()?;
    let dotted = format!(".{ext}");
    FILE_EXTENSION_MAP.get(dotted.as_str()).copied()
}

pub fn get_language(name: &str) -> Option<tree_sitter::Language> {
    match name {
        "python" => Some(tree_sitter_python::LANGUAGE.into()),
        "javascript" => Some(tree_sitter_javascript::LANGUAGE.into()),
        "typescript" => Some(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
        "tsx" => Some(tree_sitter_typescript::LANGUAGE_TSX.into()),
        "java" => Some(tree_sitter_java::LANGUAGE.into()),
        "go" => Some(tree_sitter_go::LANGUAGE.into()),
        _ => None,
    }
}

pub fn get_language_info(name: &str) -> Option<&'static LanguageInfo> {
    LANGUAGE_INFO_MAP.get(name).copied()
}

pub fn get_supported_extensions() -> &'static [&'static str] {
    &SUPPORTED_EXTENSIONS
}
