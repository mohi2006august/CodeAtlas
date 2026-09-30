//! Declarative per-language extraction rules (FR-01).
//!
//! Each language is described by node kinds rather than bespoke code, so the
//! one generic walker in `extract.rs` serves every grammar. Rules are checked
//! in order and the first one whose kind, value shape and guard match wins.

use std::sync::LazyLock;
use tree_sitter::{Language, Node};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sym {
    Function,
    Class,
    /// Adds to the qualified name but emits no symbol (namespaces, modules).
    Namespace,
    /// Like `Namespace`, but functions inside are methods (impl blocks, extensions).
    ClassScope,
}

pub type NameFn = for<'a, 'b> fn(Node<'a>, &'b [u8]) -> Option<String>;
pub type GuardFn = for<'a, 'b> fn(Node<'a>, &'b [u8]) -> bool;
pub type CustomDefFn = for<'a, 'b> fn(Node<'a>, &'b [u8]) -> Option<(Sym, String)>;

pub enum NameBy {
    Field(&'static str),
    /// C-style declarator chain down to the identifier.
    Declarator,
    /// Go method: `Receiver.name`.
    GoReceiver,
    FirstKind(&'static [&'static str]),
    Const(&'static str),
    Custom(NameFn),
}

pub struct DefRule {
    pub kind: &'static str,
    pub sym: Sym,
    pub name: NameBy,
    /// Steps from the definition node to the node holding the body: a field
    /// name, or `#kind` for the first named child of that kind.
    pub value_path: &'static [&'static str],
    /// When non-empty, the node reached via `value_path` must have one of these kinds.
    pub value_kinds: &'static [&'static str],
    pub guard: Option<GuardFn>,
    /// Nested definitions are not scoped under this one.
    pub leaf: bool,
    pub test: bool,
}

impl DefRule {
    fn new(kind: &'static str, sym: Sym) -> Self {
        DefRule {
            kind,
            sym,
            name: NameBy::Field("name"),
            value_path: &[],
            value_kinds: &[],
            guard: None,
            leaf: false,
            test: false,
        }
    }
    fn name(mut self, name: NameBy) -> Self {
        self.name = name;
        self
    }
    fn value(mut self, path: &'static [&'static str], kinds: &'static [&'static str]) -> Self {
        self.value_path = path;
        self.value_kinds = kinds;
        self
    }
    fn guard(mut self, guard: GuardFn) -> Self {
        self.guard = Some(guard);
        self
    }
    fn leaf(mut self) -> Self {
        self.leaf = true;
        self
    }
    fn test(mut self) -> Self {
        self.test = true;
        self
    }
}

fn func(kind: &'static str) -> DefRule {
    DefRule::new(kind, Sym::Function)
}
fn class(kind: &'static str) -> DefRule {
    DefRule::new(kind, Sym::Class)
}
fn namespace(kind: &'static str) -> DefRule {
    DefRule::new(kind, Sym::Namespace)
}
fn class_scope(kind: &'static str) -> DefRule {
    DefRule::new(kind, Sym::ClassScope)
}

pub struct CallRule {
    pub kind: &'static str,
    /// Field holding the callee; `""` for the first named child, `#kind` for
    /// the first child of that kind.
    pub callee: &'static str,
    /// When set, `callee` is a bare name and this field holds the receiver.
    pub object: &'static str,
}

fn call(kind: &'static str, callee: &'static str) -> CallRule {
    CallRule { kind, callee, object: "" }
}
fn method_call(kind: &'static str, callee: &'static str, object: &'static str) -> CallRule {
    CallRule { kind, callee, object }
}

pub struct LangSpec {
    pub name: &'static str,
    /// Symbol id prefix, e.g. `py`.
    pub prefix: &'static str,
    pub exts: &'static [&'static str],
    pub language: fn() -> Language,
    pub defs: Vec<DefRule>,
    pub calls: Vec<CallRule>,
    pub import_kinds: &'static [&'static str],
    /// Call names that are really imports (`require`, `source`, `@import`).
    pub import_calls: &'static [&'static str],
    pub ignore_calls: &'static [&'static str],
    /// Field names or child kinds on a class node that list its bases.
    pub inherit: &'static [&'static str],
    pub wrappers: &'static [&'static str],
    /// Wrappers that only count when the definition is their sole named child.
    pub sole_wrappers: &'static [&'static str],
    pub body_kinds: &'static [&'static str],
    pub comment_kinds: &'static [&'static str],
    /// Non-comment siblings that belong to the following definition (attributes, type signatures).
    pub leading_kinds: &'static [&'static str],
    pub type_ref_kinds: &'static [&'static str],
    pub docstring: bool,
    /// Several clauses with one name form a single symbol (Elixir, Haskell).
    pub merge_clauses: bool,
    pub custom_def: Option<CustomDefFn>,
}

impl LangSpec {
    fn base(
        name: &'static str,
        prefix: &'static str,
        exts: &'static [&'static str],
        language: fn() -> Language,
    ) -> Self {
        LangSpec {
            name,
            prefix,
            exts,
            language,
            defs: Vec::new(),
            calls: Vec::new(),
            import_kinds: &[],
            import_calls: &[],
            ignore_calls: &[],
            inherit: &[],
            wrappers: &[],
            sole_wrappers: &[],
            body_kinds: &[],
            comment_kinds: &["comment"],
            leading_kinds: &[],
            type_ref_kinds: &[],
            docstring: false,
            merge_clauses: false,
            custom_def: None,
        }
    }
}

pub fn text<'a>(node: Node, src: &'a [u8]) -> &'a str {
    node.utf8_text(src).unwrap_or("")
}

fn first_child_of_kind<'a>(node: Node<'a>, kinds: &[&str]) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    let found = node.named_children(&mut cursor).find(|c| kinds.contains(&c.kind()));
    found
}

// ---- name helpers -----------------------------------------------------------

/// Drop balanced `<...>` generic arguments: `Foo<T>::bar` -> `Foo::bar`.
pub fn strip_generics(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut depth = 0usize;
    for c in s.chars() {
        match c {
            '<' => depth += 1,
            '>' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

pub fn name_declarator(node: Node, src: &[u8]) -> Option<String> {
    let mut d = node.child_by_field_name("declarator")?;
    for _ in 0..12 {
        if matches!(
            d.kind(),
            "identifier"
                | "field_identifier"
                | "type_identifier"
                | "destructor_name"
                | "operator_name"
                | "qualified_identifier"
                | "primitive_type"
        ) {
            break;
        }
        if let Some(next) = d.child_by_field_name("declarator") {
            d = next;
        } else if d.kind().ends_with("declarator") && d.named_child_count() > 0 {
            d = d.named_child(d.named_child_count() - 1)?;
        } else {
            break;
        }
    }
    let name = strip_generics(text(d, src)).replace("::", ".").replace(char::is_whitespace, "");
    if name.is_empty() || name.contains('(') {
        return None;
    }
    Some(name)
}

pub fn name_go_receiver(node: Node, src: &[u8]) -> Option<String> {
    let name = text(node.child_by_field_name("name")?, src);
    let receiver = text(node.child_by_field_name("receiver")?, src);
    let inner = receiver.trim_matches(|c| c == '(' || c == ')');
    let ty = inner.split_whitespace().last().unwrap_or("");
    let ty = ty.trim_start_matches('*');
    let ty = ty.split('[').next().unwrap_or(ty);
    if ty.is_empty() {
        Some(name.to_string())
    } else {
        Some(format!("{ty}.{name}"))
    }
}

fn js_assign_name(node: Node, src: &[u8]) -> Option<String> {
    let left = node.child_by_field_name("left")?;
    match left.kind() {
        "identifier" => Some(text(left, src).to_string()),
        "member_expression" => Some(text(left.child_by_field_name("property")?, src).to_string()),
        _ => None,
    }
}

fn js_name_is_identifier(node: Node, _src: &[u8]) -> bool {
    node.child_by_field_name("name")
        .map(|n| matches!(n.kind(), "identifier" | "type_identifier" | "property_identifier"))
        .unwrap_or(false)
}

fn rust_impl_name(node: Node, src: &[u8]) -> Option<String> {
    let ty = strip_generics(text(node.child_by_field_name("type")?, src));
    let ty = ty.trim_start_matches(['&', '*']).trim();
    let ty = ty.strip_prefix("mut ").unwrap_or(ty).trim_start_matches("dyn ").trim();
    let last = ty.rsplit("::").next().unwrap_or(ty).trim();
    (!last.is_empty()).then(|| last.to_string())
}

fn has_body(node: Node, _src: &[u8]) -> bool {
    node.child_by_field_name("body").is_some()
}

fn swift_is_extension(node: Node, _src: &[u8]) -> bool {
    let mut cursor = node.walk();
    let found = node.children(&mut cursor).any(|c| c.kind() == "extension");
    found
}

fn lua_assign_name(node: Node, src: &[u8]) -> Option<String> {
    let vars = first_child_of_kind(node, &["variable_list"])?;
    Some(text(vars.child_by_field_name("name")?, src).replace(':', "."))
}

fn lua_fn_name(node: Node, src: &[u8]) -> Option<String> {
    Some(text(node.child_by_field_name("name")?, src).replace(':', "."))
}

fn ocaml_is_function(node: Node, _src: &[u8]) -> bool {
    let named_value = node
        .child_by_field_name("pattern")
        .map(|p| p.kind() == "value_name")
        .unwrap_or(false);
    if !named_value {
        return false;
    }
    let has_param = first_child_of_kind(node, &["parameter"]).is_some();
    let fun_body = node
        .child_by_field_name("body")
        .map(|b| matches!(b.kind(), "fun_expression" | "function_expression"))
        .unwrap_or(false);
    has_param || fun_body
}

const ZIG_CONTAINERS: &[&str] = &[
    "struct_declaration",
    "enum_declaration",
    "union_declaration",
    "opaque_declaration",
    "error_set_declaration",
];

fn zig_is_container(node: Node, _src: &[u8]) -> bool {
    first_child_of_kind(node, ZIG_CONTAINERS).is_some()
}

fn zig_test_name(node: Node, src: &[u8]) -> Option<String> {
    let label = first_child_of_kind(node, &["string", "identifier"])
        .map(|n| text(n, src).trim_matches('"').to_string())
        .unwrap_or_default();
    let slug: String = label
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    let slug = slug.trim_matches('_');
    Some(if slug.is_empty() { "test".to_string() } else { format!("test_{slug}") })
}

fn julia_callable_name(node: Node, src: &[u8]) -> Option<String> {
    let mut n = node.named_child(0)?;
    for _ in 0..8 {
        match n.kind() {
            "signature" | "where_expression" | "typed_expression" | "parenthesized_expression" => {
                n = n.named_child(0)?;
            }
            "call_expression" => return Some(text(n.named_child(0)?, src).to_string()),
            "identifier" | "field_expression" | "operator" => return Some(text(n, src).to_string()),
            _ => return None,
        }
    }
    None
}

fn julia_is_short_function(node: Node, _src: &[u8]) -> bool {
    node.named_child(0)
        .map(|n| matches!(n.kind(), "call_expression" | "where_expression"))
        .unwrap_or(false)
}

fn julia_type_name(node: Node, src: &[u8]) -> Option<String> {
    let mut n = first_child_of_kind(node, &["type_head"])?;
    for _ in 0..8 {
        if n.kind() == "identifier" {
            return Some(text(n, src).to_string());
        }
        n = n.named_child(0)?;
    }
    None
}

fn dart_signature_name(node: Node, src: &[u8]) -> Option<String> {
    let mut n = node.child_by_field_name("signature")?;
    for _ in 0..4 {
        if let Some(name) = n.child_by_field_name("name") {
            return Some(text(name, src).to_string());
        }
        n = n.named_child(0)?;
    }
    None
}

fn dart_is_abstract_member(node: Node, _src: &[u8]) -> bool {
    node.parent().map(|p| p.kind() == "declaration").unwrap_or(false)
}

fn elixir_def(node: Node, src: &[u8]) -> Option<(Sym, String)> {
    if node.kind() != "call" {
        return None;
    }
    let target = node.child_by_field_name("target")?;
    if target.kind() != "identifier" {
        return None;
    }
    let args = first_child_of_kind(node, &["arguments"])?;
    let first = args.named_child(0)?;
    match text(target, src) {
        "defmodule" | "defprotocol" | "defimpl" => Some((Sym::Class, text(first, src).to_string())),
        "def" | "defp" | "defmacro" | "defmacrop" | "defguard" | "defguardp" | "defdelegate" => {
            let head = if first.kind() == "binary_operator" {
                first.child_by_field_name("left")?
            } else {
                first
            };
            let name = match head.kind() {
                "call" => text(head.child_by_field_name("target")?, src),
                "identifier" => text(head, src),
                _ => return None,
            };
            Some((Sym::Function, name.to_string()))
        }
        _ => None,
    }
}

// ---- the table --------------------------------------------------------------

const JS_FN_VALUES: &[&str] =
    &["arrow_function", "function_expression", "generator_function", "function"];

fn js_defs() -> Vec<DefRule> {
    vec![
        func("function_declaration"),
        func("generator_function_declaration"),
        class("class_declaration"),
        func("method_definition"),
        func("variable_declarator").value(&["value"], JS_FN_VALUES).guard(js_name_is_identifier),
        class("variable_declarator").value(&["value"], &["class"]).guard(js_name_is_identifier),
        func("field_definition").name(NameBy::Field("property")).value(&["value"], JS_FN_VALUES),
        func("public_field_definition").value(&["value"], JS_FN_VALUES),
        func("assignment_expression")
            .name(NameBy::Custom(js_assign_name))
            .value(&["right"], JS_FN_VALUES),
        func("pair").name(NameBy::Field("key")).value(&["value"], JS_FN_VALUES),
    ]
}

fn ts_defs() -> Vec<DefRule> {
    let mut defs = js_defs();
    defs.extend([
        class("abstract_class_declaration"),
        class("interface_declaration"),
        class("enum_declaration"),
        class("type_alias_declaration").leaf(),
        func("method_signature"),
        func("abstract_method_signature"),
        func("function_signature"),
        namespace("internal_module"),
        namespace("module"),
    ]);
    defs
}

fn js_calls() -> Vec<CallRule> {
    vec![call("call_expression", "function"), call("new_expression", "constructor")]
}

fn c_defs() -> Vec<DefRule> {
    vec![
        func("function_definition").name(NameBy::Declarator),
        func("preproc_function_def"),
        class("struct_specifier").guard(has_body),
        class("union_specifier").guard(has_body),
        class("enum_specifier").guard(has_body),
        class("type_definition").name(NameBy::Declarator).leaf(),
    ]
}

fn js_like(mut spec: LangSpec, typescript: bool) -> LangSpec {
    spec.defs = if typescript { ts_defs() } else { js_defs() };
    spec.calls = js_calls();
    spec.import_kinds = &["import_statement"];
    spec.import_calls = &["require", "import"];
    spec.inherit = &["class_heritage", "extends_type_clause"];
    spec.wrappers = &["export_statement", "ambient_declaration"];
    spec.sole_wrappers = &["lexical_declaration", "variable_declaration", "expression_statement"];
    spec.body_kinds = &["statement_block", "class_body"];
    if typescript {
        spec.type_ref_kinds = &["type_identifier"];
    }
    spec
}

fn build_specs() -> Vec<LangSpec> {
    let mut specs = Vec::new();

    let mut python =
        LangSpec::base("python", "py", &["py", "pyi"], || tree_sitter_python::LANGUAGE.into());
    python.defs = vec![func("function_definition"), class("class_definition")];
    python.calls = vec![call("call", "function")];
    python.import_kinds = &["import_statement", "import_from_statement"];
    python.inherit = &["superclasses"];
    python.wrappers = &["decorated_definition"];
    python.body_kinds = &["block"];
    python.type_ref_kinds = &["type"];
    python.docstring = true;
    specs.push(python);

    specs.push(js_like(
        LangSpec::base("javascript", "js", &["js", "jsx", "mjs", "cjs"], || {
            tree_sitter_javascript::LANGUAGE.into()
        }),
        false,
    ));
    specs.push(js_like(
        LangSpec::base("typescript", "ts", &["ts", "mts", "cts"], || {
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
        }),
        true,
    ));
    specs.push(js_like(
        LangSpec::base("tsx", "ts", &["tsx"], || tree_sitter_typescript::LANGUAGE_TSX.into()),
        true,
    ));

    let mut rust = LangSpec::base("rust", "rs", &["rs"], || tree_sitter_rust::LANGUAGE.into());
    rust.defs = vec![
        func("function_item"),
        func("function_signature_item"),
        class("struct_item"),
        class("enum_item"),
        class("union_item"),
        class("trait_item"),
        class("type_item").leaf(),
        func("macro_definition"),
        class_scope("impl_item").name(NameBy::Custom(rust_impl_name)),
        namespace("mod_item").guard(has_body),
    ];
    rust.calls = vec![
        call("call_expression", "function"),
        call("macro_invocation", "macro"),
        call("struct_expression", "name"),
    ];
    rust.import_kinds = &["use_declaration"];
    rust.inherit = &["bounds"];
    rust.body_kinds = &["block", "declaration_list", "field_declaration_list", "enum_variant_list"];
    rust.comment_kinds = &["line_comment", "block_comment"];
    rust.leading_kinds = &["attribute_item"];
    rust.type_ref_kinds = &["type_identifier"];
    specs.push(rust);

    let mut go = LangSpec::base("go", "go", &["go"], || tree_sitter_go::LANGUAGE.into());
    go.defs = vec![
        func("function_declaration"),
        func("method_declaration").name(NameBy::GoReceiver),
        class("type_spec"),
        class("type_alias"),
    ];
    go.calls = vec![call("call_expression", "function"), call("composite_literal", "type")];
    go.import_kinds = &["import_spec"];
    go.sole_wrappers = &["type_declaration"];
    go.body_kinds = &["block"];
    go.type_ref_kinds = &["type_identifier"];
    specs.push(go);

    let mut java = LangSpec::base("java", "java", &["java"], || tree_sitter_java::LANGUAGE.into());
    java.defs = vec![
        class("class_declaration"),
        class("interface_declaration"),
        class("enum_declaration"),
        class("record_declaration"),
        class("annotation_type_declaration"),
        func("method_declaration"),
        func("constructor_declaration"),
    ];
    java.calls = vec![
        method_call("method_invocation", "name", "object"),
        call("object_creation_expression", "type"),
    ];
    java.import_kinds = &["import_declaration"];
    java.inherit = &["superclass", "interfaces", "extends_interfaces"];
    java.body_kinds = &["block", "class_body", "interface_body", "enum_body"];
    java.comment_kinds = &["line_comment", "block_comment"];
    java.type_ref_kinds = &["type_identifier"];
    specs.push(java);

    let mut c = LangSpec::base("c", "c", &["c"], || tree_sitter_c::LANGUAGE.into());
    c.defs = c_defs();
    c.calls = vec![call("call_expression", "function")];
    c.import_kinds = &["preproc_include"];
    c.body_kinds = &["compound_statement", "field_declaration_list", "enumerator_list"];
    c.type_ref_kinds = &["type_identifier"];
    specs.push(c);

    let mut cpp = LangSpec::base("cpp", "cpp", &["cc", "cpp", "cxx", "hpp", "hh", "hxx"], || {
        tree_sitter_cpp::LANGUAGE.into()
    });
    cpp.defs = c_defs();
    cpp.defs.extend([class("class_specifier").guard(has_body), namespace("namespace_definition")]);
    cpp.calls = vec![call("call_expression", "function"), call("new_expression", "type")];
    cpp.import_kinds = &["preproc_include"];
    cpp.inherit = &["base_class_clause"];
    cpp.wrappers = &["template_declaration"];
    cpp.body_kinds = &["compound_statement", "field_declaration_list", "enumerator_list"];
    cpp.type_ref_kinds = &["type_identifier"];
    specs.push(cpp);

    let mut csharp =
        LangSpec::base("csharp", "cs", &["cs"], || tree_sitter_c_sharp::LANGUAGE.into());
    csharp.defs = vec![
        class("class_declaration"),
        class("interface_declaration"),
        class("struct_declaration"),
        class("enum_declaration"),
        class("record_declaration"),
        class("record_struct_declaration"),
        func("method_declaration"),
        func("constructor_declaration"),
        func("destructor_declaration"),
        func("local_function_statement"),
        namespace("namespace_declaration"),
    ];
    csharp.calls = vec![
        call("invocation_expression", "function"),
        call("object_creation_expression", "type"),
    ];
    csharp.import_kinds = &["using_directive"];
    csharp.inherit = &["base_list"];
    csharp.body_kinds = &["block", "declaration_list", "arrow_expression_clause"];
    specs.push(csharp);

    let mut ruby = LangSpec::base("ruby", "rb", &["rb", "rake"], || tree_sitter_ruby::LANGUAGE.into());
    ruby.defs = vec![
        func("method"),
        func("singleton_method"),
        class("class"),
        class("module"),
    ];
    ruby.calls = vec![method_call("call", "method", "receiver")];
    ruby.import_calls = &["require", "require_relative", "load"];
    ruby.inherit = &["superclass"];
    ruby.body_kinds = &["body_statement"];
    specs.push(ruby);

    let mut php = LangSpec::base("php", "php", &["php"], || tree_sitter_php::LANGUAGE_PHP.into());
    php.defs = vec![
        func("function_definition"),
        func("method_declaration"),
        class("class_declaration"),
        class("interface_declaration"),
        class("trait_declaration"),
        class("enum_declaration"),
        namespace("namespace_definition").guard(has_body),
    ];
    php.calls = vec![
        call("function_call_expression", "function"),
        method_call("member_call_expression", "name", "object"),
        method_call("nullsafe_member_call_expression", "name", "object"),
        method_call("scoped_call_expression", "name", "scope"),
        call("object_creation_expression", ""),
    ];
    php.import_kinds = &[
        "namespace_use_declaration",
        "require_expression",
        "require_once_expression",
        "include_expression",
        "include_once_expression",
    ];
    php.inherit = &["base_clause", "class_interface_clause"];
    php.body_kinds = &["compound_statement", "declaration_list"];
    php.type_ref_kinds = &["named_type"];
    specs.push(php);

    let mut swift =
        LangSpec::base("swift", "swift", &["swift"], || tree_sitter_swift::LANGUAGE.into());
    swift.defs = vec![
        func("function_declaration"),
        func("protocol_function_declaration"),
        func("init_declaration").name(NameBy::Const("init")),
        func("deinit_declaration").name(NameBy::Const("deinit")),
        class_scope("class_declaration").guard(swift_is_extension),
        class("class_declaration"),
        class("protocol_declaration"),
    ];
    swift.calls = vec![call("call_expression", "")];
    swift.import_kinds = &["import_declaration"];
    swift.inherit = &["inheritance_specifier"];
    swift.body_kinds = &["function_body", "class_body", "enum_class_body", "protocol_body"];
    swift.comment_kinds = &["comment", "multiline_comment"];
    swift.type_ref_kinds = &["type_identifier"];
    specs.push(swift);

    let mut kotlin =
        LangSpec::base("kotlin", "kt", &["kt", "kts"], || tree_sitter_kotlin_ng::LANGUAGE.into());
    kotlin.defs = vec![
        func("function_declaration"),
        func("secondary_constructor").name(NameBy::Const("constructor")),
        class("class_declaration"),
        class("object_declaration"),
    ];
    kotlin.calls = vec![call("call_expression", "")];
    kotlin.import_kinds = &["import"];
    kotlin.inherit = &["delegation_specifiers"];
    kotlin.body_kinds = &["function_body", "class_body", "enum_class_body"];
    kotlin.comment_kinds = &["line_comment", "block_comment"];
    kotlin.type_ref_kinds = &["user_type"];
    specs.push(kotlin);

    let mut scala =
        LangSpec::base("scala", "scala", &["scala", "sc"], || tree_sitter_scala::LANGUAGE.into());
    scala.defs = vec![
        func("function_definition"),
        func("function_declaration"),
        class("class_definition"),
        class("object_definition"),
        class("trait_definition"),
        class("enum_definition"),
    ];
    scala.calls = vec![call("call_expression", "function"), call("instance_expression", "")];
    scala.import_kinds = &["import_declaration"];
    scala.inherit = &["extend"];
    scala.body_kinds = &["template_body", "block", "indented_block"];
    scala.comment_kinds = &["comment", "block_comment"];
    scala.type_ref_kinds = &["type_identifier"];
    specs.push(scala);

    let mut bash = LangSpec::base("bash", "sh", &["sh", "bash"], || tree_sitter_bash::LANGUAGE.into());
    bash.defs = vec![func("function_definition")];
    bash.calls = vec![call("command", "name")];
    bash.import_calls = &["source", "."];
    bash.body_kinds = &["compound_statement"];
    specs.push(bash);

    let mut lua = LangSpec::base("lua", "lua", &["lua"], || tree_sitter_lua::LANGUAGE.into());
    lua.defs = vec![
        func("function_declaration").name(NameBy::Custom(lua_fn_name)),
        func("assignment_statement")
            .name(NameBy::Custom(lua_assign_name))
            .value(&["#expression_list", "value"], &["function_definition"]),
    ];
    lua.calls = vec![call("function_call", "name")];
    lua.import_calls = &["require", "dofile"];
    lua.sole_wrappers = &["variable_declaration"];
    lua.body_kinds = &["block"];
    specs.push(lua);

    let mut elixir =
        LangSpec::base("elixir", "ex", &["ex", "exs"], || tree_sitter_elixir::LANGUAGE.into());
    elixir.calls = vec![call("call", "target")];
    elixir.import_calls = &["alias", "import", "use", "require"];
    elixir.ignore_calls = &[
        "def", "defp", "defmacro", "defmacrop", "defmodule", "defprotocol", "defimpl", "defguard",
        "defguardp", "defdelegate", "defstruct", "quote", "unquote",
    ];
    elixir.body_kinds = &["do_block"];
    elixir.leading_kinds = &["unary_operator"];
    elixir.merge_clauses = true;
    elixir.custom_def = Some(elixir_def);
    specs.push(elixir);

    let mut haskell =
        LangSpec::base("haskell", "hs", &["hs"], || tree_sitter_haskell::LANGUAGE.into());
    haskell.defs = vec![
        func("function"),
        func("bind"),
        class("data_type"),
        class("newtype"),
        class("class"),
        class("type_synomym").leaf(),
        class("type_family").leaf(),
        class_scope("instance"),
    ];
    haskell.calls = vec![call("apply", "function")];
    haskell.import_kinds = &["import"];
    haskell.body_kinds = &["match", "class_declarations", "instance_declarations", "data_constructors"];
    haskell.comment_kinds = &["comment", "haddock"];
    haskell.leading_kinds = &["signature"];
    haskell.merge_clauses = true;
    specs.push(haskell);

    let mut ocaml =
        LangSpec::base("ocaml", "ml", &["ml"], || tree_sitter_ocaml::LANGUAGE_OCAML.into());
    ocaml.defs = vec![
        func("let_binding").name(NameBy::Field("pattern")).guard(ocaml_is_function),
        func("external").name(NameBy::FirstKind(&["value_name"])),
        class("module_binding").name(NameBy::FirstKind(&["module_name"])),
        class("type_binding").leaf(),
        class("class_binding").name(NameBy::FirstKind(&["class_name"])),
    ];
    ocaml.calls = vec![call("application_expression", "function")];
    ocaml.import_kinds = &["open_module"];
    ocaml.sole_wrappers = &["value_definition", "module_definition", "type_definition", "class_definition"];
    specs.push(ocaml);

    let mut zig = LangSpec::base("zig", "zig", &["zig"], || tree_sitter_zig::LANGUAGE.into());
    zig.defs = vec![
        func("function_declaration"),
        class("variable_declaration")
            .name(NameBy::FirstKind(&["identifier"]))
            .guard(zig_is_container),
        func("test_declaration").name(NameBy::Custom(zig_test_name)).test(),
    ];
    zig.calls = vec![call("call_expression", "function"), call("builtin_function", "#builtin_identifier")];
    zig.import_calls = &["@import"];
    zig.body_kinds = &["block", "struct_declaration", "enum_declaration", "union_declaration"];
    specs.push(zig);

    let mut dart = LangSpec::base("dart", "dart", &["dart"], || tree_sitter_dart::LANGUAGE.into());
    dart.defs = vec![
        func("function_declaration").name(NameBy::Custom(dart_signature_name)),
        func("method_declaration").name(NameBy::Custom(dart_signature_name)),
        func("function_signature").guard(dart_is_abstract_member),
        class("class_declaration"),
        class("enum_declaration"),
        class("mixin_declaration"),
        class("extension_declaration"),
        class("extension_type_declaration"),
    ];
    dart.calls = vec![call("call_expression", "function")];
    dart.import_kinds = &["import_specification"];
    dart.inherit = &["superclass", "interfaces", "mixins"];
    dart.sole_wrappers = &["class_member", "declaration"];
    dart.body_kinds = &["function_body", "class_body", "enum_body"];
    dart.comment_kinds = &["comment", "documentation_comment"];
    dart.type_ref_kinds = &["type_identifier"];
    specs.push(dart);

    let mut r = LangSpec::base("r", "r", &["r"], || tree_sitter_r::LANGUAGE.into());
    r.defs = vec![func("binary_operator")
        .name(NameBy::Field("lhs"))
        .value(&["rhs"], &["function_definition"])];
    r.calls = vec![call("call", "function")];
    r.import_calls = &["library", "require", "source", "requireNamespace", "loadNamespace"];
    r.body_kinds = &["braced_expression"];
    specs.push(r);

    let mut julia = LangSpec::base("julia", "jl", &["jl"], || tree_sitter_julia::LANGUAGE.into());
    julia.defs = vec![
        func("function_definition").name(NameBy::Custom(julia_callable_name)),
        func("macro_definition").name(NameBy::Custom(julia_callable_name)),
        func("assignment").name(NameBy::Custom(julia_callable_name)).guard(julia_is_short_function),
        class("struct_definition").name(NameBy::Custom(julia_type_name)),
        class("abstract_definition").name(NameBy::Custom(julia_type_name)).leaf(),
        namespace("module_definition"),
    ];
    julia.calls = vec![call("call_expression", "")];
    julia.import_kinds = &["using_statement", "import_statement"];
    julia.import_calls = &["include"];
    julia.inherit = &["type_head"];
    julia.comment_kinds = &["line_comment", "block_comment"];
    julia.leading_kinds = &["string_literal"];
    specs.push(julia);

    let mut objc = LangSpec::base("objc", "objc", &["m", "mm"], || tree_sitter_objc::LANGUAGE.into());
    objc.defs = c_defs();
    objc.defs.extend([
        class("class_interface").name(NameBy::FirstKind(&["identifier"])),
        class("protocol_declaration").name(NameBy::FirstKind(&["identifier"])),
        class_scope("class_implementation").name(NameBy::FirstKind(&["identifier"])),
        func("method_definition").name(NameBy::FirstKind(&["identifier"])),
    ]);
    objc.calls = vec![
        method_call("message_expression", "method", "receiver"),
        call("call_expression", "function"),
    ];
    objc.import_kinds = &["preproc_include", "preproc_import"];
    objc.inherit = &["superclass"];
    objc.body_kinds = &["compound_statement", "field_declaration_list", "enumerator_list"];
    objc.type_ref_kinds = &["type_identifier"];
    specs.push(objc);

    specs
}

static SPECS: LazyLock<Vec<LangSpec>> = LazyLock::new(build_specs);

pub fn specs() -> &'static [LangSpec] {
    &SPECS
}

pub fn spec_by_name(name: &str) -> Option<&'static LangSpec> {
    specs().iter().find(|s| s.name == name)
}

/// Code-like files we have no grammar for; indexed as line-window chunks.
pub const FALLBACK_EXTS: &[&str] = &[
    "sql", "proto", "graphql", "gql", "vue", "svelte", "tf", "ps1", "pl", "pm", "erl", "clj",
    "cljs", "groovy", "gradle", "f90", "vb", "fs", "sol", "nim", "cr", "mli",
];

pub enum Detected {
    Spec(&'static LangSpec),
    /// No grammar: line-window chunks with this id prefix.
    Fallback(String),
}

/// Pick a language from the file extension. `.h` is sniffed because it may be
/// C, C++ or Objective-C.
pub fn detect(path: &str, content: &[u8]) -> Option<Detected> {
    let file = path.rsplit('/').next().unwrap_or(path);
    let ext = file.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase())?;
    if ext == "h" {
        let head = String::from_utf8_lossy(&content[..content.len().min(16 * 1024)]);
        let name = if head.contains("@interface") || head.contains("@protocol") {
            "objc"
        } else if ["class ", "namespace ", "template<", "template <"].iter().any(|k| head.contains(k)) {
            "cpp"
        } else {
            "c"
        };
        return spec_by_name(name).map(Detected::Spec);
    }
    if let Some(spec) = specs().iter().find(|s| s.exts.contains(&ext.as_str())) {
        return Some(Detected::Spec(spec));
    }
    FALLBACK_EXTS.contains(&ext.as_str()).then(|| Detected::Fallback(ext))
}

/// True when the path has an extension CodeAtlas indexes.
pub fn is_indexable_path(path: &str) -> bool {
    let file = path.rsplit('/').next().unwrap_or(path);
    match file.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()) {
        Some(ext) => {
            ext == "h"
                || specs().iter().any(|s| s.exts.contains(&ext.as_str()))
                || FALLBACK_EXTS.contains(&ext.as_str())
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supports_at_least_twenty_languages() {
        let mut names: Vec<_> = specs().iter().map(|s| s.name).collect();
        names.sort_unstable();
        names.dedup();
        assert!(names.len() >= 20, "only {} languages", names.len());
    }

    #[test]
    fn every_grammar_loads() {
        for spec in specs() {
            let mut parser = tree_sitter::Parser::new();
            parser
                .set_language(&(spec.language)())
                .unwrap_or_else(|e| panic!("{}: {e}", spec.name));
        }
    }

    #[test]
    fn header_files_are_sniffed() {
        let is = |content: &str, want: &str| match detect("a/b.h", content.as_bytes()) {
            Some(Detected::Spec(s)) => assert_eq!(s.name, want),
            _ => panic!("no spec"),
        };
        is("int f(void);", "c");
        is("namespace x { class Y {}; }", "cpp");
        is("@interface Foo : NSObject\n@end", "objc");
    }

    #[test]
    fn strips_generic_arguments() {
        assert_eq!(strip_generics("Foo<T, Vec<U>>::bar"), "Foo::bar");
    }
}
