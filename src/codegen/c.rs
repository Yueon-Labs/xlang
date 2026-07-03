use crate::ast::*;
use crate::error::{XError, XResult};
use crate::source::Spanned;
use std::cell::Cell;
use std::collections::{BTreeMap, HashMap, HashSet};

/// Build the `i32` type node (used e.g. for numeric range-loop iterators).
fn i32_type() -> TypeNode {
    TypeNode::TypeExpr {
        name: "i32".to_string(),
        args: vec![],
    }
}

/// Mangled C name for an `impl Type` method: `__xlang_method_<Type>_<name>`.
/// Unique per (type, method), and the `__xlang_method_` prefix can't collide
/// with user identifiers (which can't start with two underscores... they can,
/// but the convention is reserved).
fn method_fn_name(type_name: &str, method_name: &str) -> String {
    format!("__xlang_method_{type_name}_{method_name}")
}

#[derive(Default)]
pub struct CGen {
    lines: Vec<String>,
    indent: usize,
    scopes: Vec<HashMap<String, TypeNode>>,
    temp_counter: usize,
    /// Return type of the function currently being generated (for constructing
    /// Some/None/Ok/Err in `return` position).
    fn_return: Option<TypeNode>,
    /// User-defined struct names (so `c_type` recognises them as value types).
    struct_names: HashSet<String>,
    /// Struct field declarations: struct name → Vec<(field_name, field_type)>.
    /// Used to look up a field's declared type in struct-literal context (e.g.
    /// so `Bag { items: vec_new() }` can lower vec_new with the Vec<T> type).
    struct_fields: HashMap<String, Vec<(String, TypeNode)>>,
    /// Unit-variant enum names (so `c_type` lowers them to int32_t) and the
    /// integer value of each variant (so `Red` → its index).
    enum_names: HashSet<String>,
    enum_values: HashMap<String, i32>,
    /// Per-variant payload type (by index) for each enum; `None` for a unit
    /// variant. An enum is a tagged struct iff any entry is `Some`.
    enum_payloads: HashMap<String, Vec<Option<TypeNode>>>,
    /// variant name → the enum it belongs to (for construction/dispatch).
    variant_enum: HashMap<String, String>,
    /// Inferred type of each expression (from typecheck), keyed by node address.
    /// Lets us lower `+` as string concat vs numeric add by inspecting operand
    /// types. Empty for the test path (`CGen::new()`), where `+` is always
    /// numeric.
    types: crate::typecheck::TypeMap,
    /// Methods declared in `impl` blocks: (type_name, method_name) → mangled C
    /// function name (`__xlang_method_<Type>_<method>`). Used to dispatch
    /// `obj.method(args)` calls.
    methods: HashMap<(String, String), String>,
    /// Methods declared with `mut self` — these take `self` by pointer so
    /// mutations persist (the caller passes `&receiver`). (type, method).
    mut_self: HashSet<(String, String)>,
    /// True while generating the body of a `mut self` method (so `self` →
    /// `(*self)` in expressions).
    in_mut_self: bool,
    /// Set when any `tls_*` builtin is called, so the (OpenSSL) TLS preamble
    /// section + `#define __XLANG_TLS__` are emitted only for programs that
    /// actually use TLS — keeps non-TLS servers free of an OpenSSL dependency.
    uses_tls: Cell<bool>,
}

impl CGen {
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct with an expression type map (from `check_program_typed`) so
    /// codegen can make type-dependent lowering decisions (string `+`).
    pub fn with_types(types: crate::typecheck::TypeMap) -> Self {
        Self {
            types,
            ..Self::default()
        }
    }

    pub fn generate(mut self, program: &Program) -> XResult<String> {
        for item in &program.items {
            if let Item::StructDecl { name, fields } = &item.node {
                self.struct_names.insert(name.clone());
                self.struct_fields.insert(
                    name.clone(),
                    fields
                        .iter()
                        .map(|f| (f.name.clone(), f.ty.clone()))
                        .collect(),
                );
            }
            if let Item::EnumDecl { name, variants } = &item.node {
                self.enum_names.insert(name.clone());
                let mut payloads = Vec::new();
                for (idx, variant) in variants.iter().enumerate() {
                    self.enum_values.insert(variant.name.clone(), idx as i32);
                    self.variant_enum.insert(variant.name.clone(), name.clone());
                    payloads.push(variant.payload.clone());
                }
                self.enum_payloads.insert(name.clone(), payloads);
            }
        }
        // Register impl-block methods: (type, method) → mangled free-function
        // name. Done before generating any code so dispatch works regardless of
        // source order.
        for item in &program.items {
            if let Item::ImplDecl {
                type_name, methods, ..
            } = &item.node
            {
                for method in methods {
                    if let Item::FnDecl { name, params, .. } = &method.node {
                        let mangled = method_fn_name(type_name, name);
                        self.methods
                            .insert((type_name.clone(), name.clone()), mangled);
                        // Track `mut self` methods (take self by pointer).
                        if !params.is_empty() && params[0].mutable && params[0].name == "self" {
                            self.mut_self.insert((type_name.clone(), name.clone()));
                        }
                    }
                }
            }
        }
        self.emit("#include <stdint.h>");
        self.emit("#include <stdbool.h>");
        self.emit("#include <stddef.h>");
        self.emit("#include <stdio.h>");
        self.emit("#include <string.h>");
        self.emit("#include <stdlib.h>");
        self.emit("#include <time.h>");
        self.emit("#include <locale.h>");
        self.emit("");
        self.emit_runtime_preamble();
        self.emit_networking_preamble();

        // User struct definitions AND wrapper typedefs (Vec/Array/Option/...),
        // emitted together in dependency order: a struct with a `Vec<T>` field
        // must follow `Vec_T`, and a `Vec<MyStruct>` must follow `MyStruct`.
        for def in self.collect_runtime_typedefs(program)? {
            self.emit(&def);
        }
        if !self.lines.last().is_some_and(|line| line.is_empty()) {
            self.emit("");
        }

        // Forward declarations so functions can reference each other in any
        // source order (a prerequisite for multi-file module merging too).
        for item in &program.items {
            match &item.node {
                Item::FnDecl { .. } => self.gen_fn_prototype(&item.node, None)?,
                Item::ImplDecl { type_name, methods } => {
                    for method in methods {
                        if let Item::FnDecl { name, .. } = &method.node {
                            self.gen_fn_prototype(
                                &method.node,
                                Some(&method_fn_name(type_name, name)),
                            )?;
                        }
                    }
                }
                Item::StructDecl { .. } | Item::TypeAliasDecl { .. } | Item::EnumDecl { .. } => {}
            }
        }
        self.emit("");

        for item in &program.items {
            match &item.node {
                Item::FnDecl { .. } => {
                    self.gen_fn(&item.node, None)?;
                    self.emit("");
                }
                Item::ImplDecl { type_name, methods } => {
                    for method in methods {
                        if let Item::FnDecl { name, .. } = &method.node {
                            self.gen_fn(&method.node, Some(&method_fn_name(type_name, name)))?;
                            self.emit("");
                        }
                    }
                }
                Item::StructDecl { .. } | Item::TypeAliasDecl { .. } | Item::EnumDecl { .. } => {}
            }
        }

        // If any tls_* builtin was called, activate the gated TLS preamble
        // (#ifdef __XLANG_TLS__) by defining the macro at the very top.
        if self.uses_tls.get() {
            self.lines.insert(0, "#define __XLANG_TLS__ 1".to_string());
        }

        Ok(format!("{}\n", self.lines.join("\n").trim_end()))
    }

    fn emit(&mut self, line: &str) {
        self.lines
            .push(format!("{}{}", "    ".repeat(self.indent), line));
    }

    fn collect_runtime_typedefs(&self, program: &Program) -> XResult<Vec<String>> {
        let mut typedefs = BTreeMap::new();
        for item in &program.items {
            match &item.node {
                Item::StructDecl { name, fields } => {
                    for field in fields {
                        self.collect_type_typedefs(&field.ty, &mut typedefs)?;
                    }
                    // Collect the struct's own definition too, so it is emitted
                    // in dependency order with the wrapper typedefs its fields
                    // reference (e.g. a struct with a `Vec<T>` field must come
                    // after `Vec_T`).
                    let def = self.struct_def(&item.node)?;
                    typedefs.entry(name.clone()).or_insert(def);
                }
                Item::TypeAliasDecl { ty, .. } => {
                    self.collect_type_typedefs(ty, &mut typedefs)?;
                }
                Item::EnumDecl { name, variants } => {
                    // A payload enum lowers to a tagged struct with a union of
                    // the payload types (one member per payload variant, by
                    // index). Unit-only enums lower to int32_t (no typedef).
                    let has_payload = variants.iter().any(|v| v.payload.is_some());
                    if has_payload {
                        for v in variants {
                            if let Some(ty) = &v.payload {
                                self.collect_type_typedefs(ty, &mut typedefs)?;
                            }
                        }
                        let def = self.enum_def(name, variants)?;
                        typedefs.entry(name.clone()).or_insert(def);
                    }
                }
                Item::FnDecl {
                    params,
                    return_type,
                    body,
                    ..
                } => {
                    self.collect_type_typedefs(return_type, &mut typedefs)?;
                    for param in params {
                        self.collect_type_typedefs(&param.ty, &mut typedefs)?;
                    }
                    self.collect_block_typedefs(body, &mut typedefs)?;
                }
                Item::ImplDecl { methods, .. } => {
                    for method in methods {
                        if let Item::FnDecl {
                            params,
                            return_type,
                            body,
                            ..
                        } = &method.node
                        {
                            self.collect_type_typedefs(return_type, &mut typedefs)?;
                            for param in params {
                                self.collect_type_typedefs(&param.ty, &mut typedefs)?;
                            }
                            self.collect_block_typedefs(body, &mut typedefs)?;
                        }
                    }
                }
            }
        }
        // Forward-declare all user structs and payload enums so that recursive
        // types work: a wrapper like Vec_Tree holds `Tree *data` (pointer), so
        // a forward decl of Tree suffices for the wrapper — even before Tree's
        // full definition. Without this, `enum Tree { Branch(BranchData) }`
        // where BranchData has a `Vec<Tree>` field would deadlock the fixpoint.
        let mut fwd_decls: Vec<String> = Vec::new();
        for item in &program.items {
            match &item.node {
                Item::StructDecl { name, .. } => {
                    fwd_decls.push(format!("typedef struct {name} {name};"));
                }
                Item::EnumDecl { name, variants }
                    if variants.iter().any(|v| v.payload.is_some()) =>
                {
                    fwd_decls.push(format!("typedef struct {name} {name};"));
                }
                _ => {}
            }
        }

        // Two-phase fixpoint: Phase 1 emits Vec_/Slice_ wrappers (their element
        // types are forward-declared → never blocked by structs/enums). Phase 2
        // emits structs/enums/Array/Option/Result, seeded with phase 1's names.
        // This breaks recursive cycles through Vec (Vec_Tree → Tree, where Tree
        // is forward-declared for the pointer, and Tree's full def comes later).
        let (phase1, phase2): (BTreeMap<String, String>, BTreeMap<String, String>) = typedefs
            .into_iter()
            .partition(|(k, _)| k.starts_with("Vec_") || k.starts_with("Slice_"));

        let (mut ordered1, emitted1) = self.typedef_fixpoint(phase1, HashSet::new())?;
        let (ordered2, _) = self.typedef_fixpoint(phase2, emitted1)?;

        ordered1.extend(ordered2);
        fwd_decls.extend(ordered1);
        Ok(fwd_decls)
    }

    /// Dependency-ordered emission of typedefs: a definition referencing
    /// another pending definition must wait. Returns the ordered definitions
    /// and the set of emitted names. `seed` pre-populates `emitted` (used to
    /// carry phase-1 results into phase-2).
    fn typedef_fixpoint(
        &self,
        mut pending: BTreeMap<String, String>,
        seed: HashSet<String>,
    ) -> XResult<(Vec<String>, HashSet<String>)> {
        let mut ordered: Vec<String> = Vec::new();
        let mut emitted = seed;
        while !pending.is_empty() {
            let names: Vec<String> = pending.keys().cloned().collect();
            let mut progressed = false;
            for name in &names {
                let Some(def) = pending.get(name) else {
                    continue;
                };
                let blocked = pending
                    .keys()
                    .any(|other| other != name && def.contains(other) && !emitted.contains(other));
                if !blocked {
                    ordered.push(def.clone());
                    emitted.insert(name.clone());
                    pending.remove(name);
                    progressed = true;
                }
            }
            if !progressed {
                break;
            }
        }
        for def in pending.into_values() {
            ordered.push(def);
        }
        Ok((ordered, emitted))
    }

    fn collect_block_typedefs(
        &self,
        block: &Block,
        typedefs: &mut BTreeMap<String, String>,
    ) -> XResult<()> {
        for stmt in &block.statements {
            match &stmt.node {
                Stmt::LetStmt { ty, .. } => self.collect_type_typedefs(ty, typedefs)?,
                Stmt::IfStmt {
                    then_block,
                    else_branch,
                    ..
                } => {
                    self.collect_block_typedefs(then_block, typedefs)?;
                    match else_branch {
                        Some(ElseBranch::Block(block)) => {
                            self.collect_block_typedefs(block, typedefs)?;
                        }
                        Some(ElseBranch::IfStmt(stmt)) => {
                            self.collect_stmt_typedefs(stmt, typedefs)?;
                        }
                        None => {}
                    }
                }
                Stmt::ForStmt { body, .. } | Stmt::WhileStmt { body, .. } => {
                    self.collect_block_typedefs(body, typedefs)?;
                }
                Stmt::MatchStmt { arms, .. } => {
                    for arm in arms {
                        self.collect_block_typedefs(&arm.body, typedefs)?;
                    }
                }
                Stmt::ReturnStmt { .. }
                | Stmt::BreakStmt
                | Stmt::ContinueStmt
                | Stmt::ExprStmt { .. } => {}
            }
        }
        Ok(())
    }

    fn collect_stmt_typedefs(
        &self,
        stmt: &Spanned<Stmt>,
        typedefs: &mut BTreeMap<String, String>,
    ) -> XResult<()> {
        self.collect_block_typedefs(
            &Block {
                kind: "Block",
                statements: vec![stmt.clone()],
            },
            typedefs,
        )
    }

    fn collect_type_typedefs(
        &self,
        ty: &TypeNode,
        typedefs: &mut BTreeMap<String, String>,
    ) -> XResult<()> {
        let TypeNode::TypeExpr { name, args } = ty else {
            return Ok(());
        };
        for arg in args {
            self.collect_type_typedefs(arg, typedefs)?;
        }
        if name == "Slice" {
            if args.len() != 1 {
                return Err(XError::Codegen(format!(
                    "Slice expects exactly one type argument, got {}",
                    args.len()
                )));
            }
            let elem_ty = &args[0];
            let alias = self.c_type(ty)?;
            let elem_c_type = self.c_type(elem_ty)?;
            typedefs.entry(alias.clone()).or_insert_with(|| {
                format!("typedef struct {{\n    {elem_c_type} *data;\n    size_t len;\n}} {alias};")
            });
        }
        if name == "Array" {
            if args.len() != 2 {
                return Err(XError::Codegen(format!(
                    "Array expects exactly two type arguments, got {}",
                    args.len()
                )));
            }
            let elem_ty = &args[0];
            let len = self.const_type_arg_value(&args[1], "Array length")?;
            let alias = self.c_type(ty)?;
            let elem_c_type = self.c_type(elem_ty)?;
            typedefs.entry(alias.clone()).or_insert_with(|| {
                format!("typedef struct {{\n    {elem_c_type} data[{len}];\n}} {alias};")
            });
        }
        if name == "Option" {
            if args.len() != 1 {
                return Err(XError::Codegen(format!(
                    "Option expects exactly one type argument, got {}",
                    args.len()
                )));
            }
            let payload_ty = &args[0];
            let alias = self.c_type(ty)?;
            let payload_c = self.c_type(payload_ty)?;
            typedefs.entry(alias.clone()).or_insert_with(|| {
                format!("typedef struct {{\n    bool some;\n    {payload_c} value;\n}} {alias};")
            });
        }
        if name == "Result" {
            if args.len() != 2 {
                return Err(XError::Codegen(format!(
                    "Result expects exactly two type arguments, got {}",
                    args.len()
                )));
            }
            let ok_ty = &args[0];
            let err_ty = &args[1];
            let alias = self.c_type(ty)?;
            let ok_c = self.c_type(ok_ty)?;
            let err_c = self.c_type(err_ty)?;
            typedefs.entry(alias.clone()).or_insert_with(|| {
                format!(
                    "typedef struct {{\n    bool ok;\n    {ok_c} value;\n    {err_c} error;\n}} {alias};"
                )
            });
        }
        if name == "Vec" {
            if args.len() != 1 {
                return Err(XError::Codegen(format!(
                    "Vec expects exactly one type argument, got {}",
                    args.len()
                )));
            }
            let elem_ty = &args[0];
            let alias = self.c_type(ty)?;
            let elem_c = self.c_type(elem_ty)?;
            let elem_suffix = self.c_type_suffix(elem_ty)?;
            typedefs.entry(alias.clone()).or_insert_with(|| {
                format!(
                    "typedef struct {{\n    {elem_c} *data;\n    size_t len;\n    size_t cap;\n}} {alias};"
                )
            });
            let push_name = format!("__xlang_vec_push_{elem_suffix}");
            typedefs.entry(push_name.clone()).or_insert_with(|| {
                format!(
                    "void {push_name}({alias} *v, {elem_c} x) {{\n    if (v->len == v->cap) {{\n        v->cap = v->cap ? v->cap * 2 : 4;\n        v->data = ({elem_c} *)realloc(v->data, v->cap * sizeof({elem_c}));\n    }}\n    v->data[v->len++] = x;\n}}"
                )
            });
            // Per-type pop helper: decrements len, returns the old last element.
            let pop_name = format!("__xlang_vec_pop_{elem_suffix}");
            typedefs.entry(pop_name.clone()).or_insert_with(|| {
                format!(
                    "{elem_c} {pop_name}({alias} *v) {{\n    if (v->len == 0) return ({elem_c}){{0}};\n    return v->data[--v->len];\n}}"
                )
            });
            // Per-type insert helper: shifts elements right, inserts at index.
            let insert_name = format!("__xlang_vec_insert_{elem_suffix}");
            typedefs.entry(insert_name.clone()).or_insert_with(|| {
                format!(
                    "void {insert_name}({alias} *v, size_t idx, {elem_c} x) {{\n    if (v->len == v->cap) {{\n        v->cap = v->cap ? v->cap * 2 : 4;\n        v->data = ({elem_c} *)realloc(v->data, v->cap * sizeof({elem_c}));\n    }}\n    if (idx > v->len) idx = v->len;\n    for (size_t i = v->len; i > idx; i--) v->data[i] = v->data[i-1];\n    v->data[idx] = x;\n    v->len++;\n}}"
                )
            });
            // Per-type remove_at helper: removes element at index, shifts left.
            let remove_name = format!("__xlang_vec_remove_{elem_suffix}");
            typedefs.entry(remove_name.clone()).or_insert_with(|| {
                format!(
                    "{elem_c} {remove_name}({alias} *v, size_t idx) {{\n    if (v->len == 0) return ({elem_c}){{0}};\n    {elem_c} old = v->data[idx];\n    for (size_t i = idx; i < v->len - 1; i++) v->data[i] = v->data[i+1];\n    v->len--;\n    return old;\n}}"
                )
            });
            // str_split helper: only emitted when Vec<String> exists.
            if elem_suffix == "String" {
                let split_name = "__xlang_str_split";
                typedefs.entry(split_name.to_string()).or_insert_with(|| {
                    "Vec_String __xlang_str_split(const char* s, char delim) {\n    Vec_String v = {0};\n    v.cap = 4;\n    v.data = (const char**)malloc(v.cap * sizeof(const char*));\n    size_t start = 0, i = 0;\n    while (1) {\n        char c = s[i];\n        if (c == delim || c == 0) {\n            size_t len = i - start;\n            char* part = (char*)malloc(len + 1);\n            memcpy(part, s + start, len);\n            part[len] = 0;\n            if (v.len == v.cap) { v.cap *= 2; v.data = (const char**)realloc(v.data, v.cap * sizeof(const char*)); }\n            v.data[v.len++] = part;\n            start = i + 1;\n            if (c == 0) break;\n        }\n        i++;\n    }\n    return v;\n}".to_string()
                });
            }
        }
        Ok(())
    }

    fn c_type(&self, ty: &TypeNode) -> XResult<String> {
        match ty {
            TypeNode::TypeExpr { name, args } if args.is_empty() => match name.as_str() {
                "i32" => Ok("int32_t".to_string()),
                "i64" => Ok("int64_t".to_string()),
                "f32" => Ok("float".to_string()),
                "f64" => Ok("double".to_string()),
                "bool" => Ok("bool".to_string()),
                "String" | "Str" => Ok("const char *".to_string()),
                other if self.struct_names.contains(other) => Ok(other.to_string()),
                // A unit-variant enum lowers to its tag integer; a payload enum
                // lowers to its tagged-struct typedef (emitted by enum_def).
                other if self.enum_names.contains(other) => {
                    if self.enum_has_payload(other) {
                        Ok(other.to_string())
                    } else {
                        Ok("int32_t".to_string())
                    }
                }
                other => Err(XError::Codegen(format!(
                    "C backend does not support type yet: {other}"
                ))),
            },
            TypeNode::TypeExpr { name, args } if name == "Slice" && args.len() == 1 => {
                Ok(format!("Slice_{}", self.c_type_suffix(&args[0])?))
            }
            TypeNode::TypeExpr { name, args } if name == "Array" && args.len() == 2 => Ok(format!(
                "Array_{}_{}",
                self.c_type_suffix(&args[0])?,
                self.const_type_arg_value(&args[1], "Array length")?
            )),
            TypeNode::TypeExpr { name, args } if name == "Option" && args.len() == 1 => {
                Ok(format!("Option_{}", self.c_type_suffix(&args[0])?))
            }
            TypeNode::TypeExpr { name, args } if name == "Result" && args.len() == 2 => {
                Ok(format!(
                    "Result_{}_{}",
                    self.c_type_suffix(&args[0])?,
                    self.c_type_suffix(&args[1])?
                ))
            }
            TypeNode::TypeExpr { name, args } if name == "Vec" && args.len() == 1 => {
                Ok(format!("Vec_{}", self.c_type_suffix(&args[0])?))
            }
            TypeNode::TypeExpr { name, .. } => Err(XError::Codegen(format!(
                "C backend does not support generic type yet: {name}<...>"
            ))),
            TypeNode::ConstTypeArg { value } => Err(XError::Codegen(format!(
                "unexpected const type argument in C type position: {value}"
            ))),
        }
    }

    fn c_type_suffix(&self, ty: &TypeNode) -> XResult<String> {
        match ty {
            TypeNode::TypeExpr { name, args } if args.is_empty() => match name.as_str() {
                "i32" | "i64" | "f32" | "f64" | "bool" | "String" | "Str" => Ok(name.clone()),
                other if self.struct_names.contains(other) => Ok(other.to_string()),
                // An enum (unit → int32_t-like, payload → struct) suffixes by name.
                other if self.enum_names.contains(other) => Ok(other.to_string()),
                other => Err(XError::Codegen(format!(
                    "C backend does not support {other} as a generated type suffix yet"
                ))),
            },
            TypeNode::TypeExpr { name, args } if name == "Slice" && args.len() == 1 => {
                Ok(format!("Slice_{}", self.c_type_suffix(&args[0])?))
            }
            TypeNode::TypeExpr { name, args } if name == "Array" && args.len() == 2 => Ok(format!(
                "Array_{}_{}",
                self.c_type_suffix(&args[0])?,
                self.const_type_arg_value(&args[1], "Array length")?
            )),
            TypeNode::TypeExpr { name, args } if name == "Option" && args.len() == 1 => {
                Ok(format!("Option_{}", self.c_type_suffix(&args[0])?))
            }
            TypeNode::TypeExpr { name, args } if name == "Result" && args.len() == 2 => {
                Ok(format!(
                    "Result_{}_{}",
                    self.c_type_suffix(&args[0])?,
                    self.c_type_suffix(&args[1])?
                ))
            }
            TypeNode::TypeExpr { name, args } if name == "Vec" && args.len() == 1 => {
                Ok(format!("Vec_{}", self.c_type_suffix(&args[0])?))
            }
            TypeNode::TypeExpr { name, .. } => Err(XError::Codegen(format!(
                "C backend does not support {name}<...> as a generated type suffix yet"
            ))),
            TypeNode::ConstTypeArg { value } => Err(XError::Codegen(format!(
                "unexpected const type argument in C type suffix: {value}"
            ))),
        }
    }

    fn const_type_arg_value<'a>(&self, ty: &'a TypeNode, label: &str) -> XResult<&'a str> {
        match ty {
            TypeNode::ConstTypeArg { value } => Ok(value),
            TypeNode::TypeExpr { name, .. } => Err(XError::Codegen(format!(
                "{label} must be a constant integer, got type {name}"
            ))),
        }
    }

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn declare_var(&mut self, name: &str, ty: TypeNode) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string(), ty);
        }
    }

    fn lookup_var(&self, name: &str) -> Option<&TypeNode> {
        self.scopes.iter().rev().find_map(|scope| scope.get(name))
    }

    fn next_temp(&mut self, prefix: &str) -> String {
        let id = self.temp_counter;
        self.temp_counter += 1;
        format!("__xlang_{prefix}{id}")
    }

    /// Whether an enum has any payload variant (→ tagged-struct representation).
    fn enum_has_payload(&self, name: &str) -> bool {
        self.enum_payloads
            .get(name)
            .map(|ps| ps.iter().any(|p| p.is_some()))
            .unwrap_or(false)
    }

    /// Build a user struct's `typedef struct {...} Name;` as a string (for
    /// dependency-ordered emission alongside wrapper typedefs — a struct with a
    /// `Vec<T>` field must be emitted after the `Vec_T` typedef).
    fn struct_def(&self, item: &Item) -> XResult<String> {
        let Item::StructDecl { name, fields } = item else {
            unreachable!();
        };
        let mut out = format!("typedef struct {name} {{\n");
        for field in fields {
            out.push_str(&format!(
                "    {} {};\n",
                self.c_type(&field.ty)?,
                field.name
            ));
        }
        out.push_str(&format!("}} {name};"));
        Ok(out)
    }

    /// Build a payload enum's tagged-struct typedef: `{ tag; union{...} u; }`.
    /// Union member `v<idx>` holds variant idx's payload (one per payload
    /// variant); unit variants set only the tag.
    fn enum_def(&self, name: &str, variants: &[EnumVariant]) -> XResult<String> {
        let mut out = format!("typedef struct {name} {{\n    int32_t tag;\n    union {{\n");
        for (idx, v) in variants.iter().enumerate() {
            if let Some(ty) = &v.payload {
                out.push_str(&format!("        {} v{idx};\n", self.c_type(ty)?));
            }
        }
        out.push_str(&format!("    }} u;\n}} {name};"));
        Ok(out)
    }

    /// Emit a forward declaration so functions may appear in any source order.
    fn gen_fn_prototype(&mut self, item: &Item, name_override: Option<&str>) -> XResult<()> {
        let Item::FnDecl {
            name,
            params,
            return_type,
            ..
        } = item
        else {
            unreachable!();
        };
        let name = name_override.unwrap_or(name.as_str());
        let ret = self.c_type(return_type)?;
        let params_text = if name == "main" && params.is_empty() {
            "int argc, char** argv".to_string()
        } else if params.is_empty() {
            "void".to_string()
        } else {
            let mut parts = Vec::new();
            for (i, param) in params.iter().enumerate() {
                if i == 0 && param.mutable && param.name == "self" {
                    parts.push(format!("{} *{}", self.c_type(&param.ty)?, param.name));
                } else {
                    parts.push(format!("{} {}", self.c_type(&param.ty)?, param.name));
                }
            }
            parts.join(", ")
        };
        self.emit(&format!("{ret} {name}({params_text});"));
        Ok(())
    }

    fn gen_fn(&mut self, item: &Item, name_override: Option<&str>) -> XResult<()> {
        let Item::FnDecl {
            name,
            params,
            return_type,
            body,
        } = item
        else {
            unreachable!();
        };
        let name = name_override.unwrap_or(name.as_str());
        let ret = self.c_type(return_type)?;
        let is_main = name == "main" && params.is_empty();
        let params_text = if is_main {
            "int argc, char** argv".to_string()
        } else if params.is_empty() {
            "void".to_string()
        } else {
            let mut parts = Vec::new();
            for (i, param) in params.iter().enumerate() {
                // `mut self: Type` → `Type *self` (by pointer, so mutations persist).
                if i == 0 && param.mutable && param.name == "self" {
                    parts.push(format!("{} *{}", self.c_type(&param.ty)?, param.name));
                } else {
                    parts.push(format!("{} {}", self.c_type(&param.ty)?, param.name));
                }
            }
            parts.join(", ")
        };
        self.emit(&format!("{ret} {name}({params_text}) {{"));
        self.indent += 1;
        self.push_scope();
        for param in params {
            self.declare_var(&param.name, param.ty.clone());
        }
        if is_main {
            self.emit("__xlang_argc_g = argc;");
            self.emit("__xlang_argv_g = argv;");
        }
        let prev_mut = self.in_mut_self;
        self.in_mut_self = !params.is_empty() && params[0].mutable && params[0].name == "self";
        self.fn_return = Some(return_type.clone());
        for stmt in &body.statements {
            self.gen_stmt(stmt)?;
        }
        self.fn_return = None;
        self.in_mut_self = prev_mut;
        self.pop_scope();
        self.indent -= 1;
        self.emit("}");
        Ok(())
    }

    fn gen_stmt(&mut self, stmt: &Spanned<Stmt>) -> XResult<()> {
        match &stmt.node {
            Stmt::LetStmt {
                name, ty, value, ..
            } => self.gen_let_stmt(name, ty, value)?,
            Stmt::ReturnStmt { value } => match value {
                Some(expr) => {
                    let rendered = if let Some(ret_ty) = &self.fn_return {
                        match self.try_constructor(ret_ty, expr)? {
                            Some(c) => c,
                            None => self.gen_expr(expr)?,
                        }
                    } else {
                        self.gen_expr(expr)?
                    };
                    self.emit(&format!("return {rendered};"));
                }
                None => self.emit("return;"),
            },
            Stmt::IfStmt {
                condition,
                then_block,
                else_branch,
            } => {
                self.emit(&format!("if ({}) {{", self.gen_expr(condition)?));
                self.indent += 1;
                for inner in &then_block.statements {
                    self.gen_stmt(inner)?;
                }
                self.indent -= 1;
                match else_branch {
                    None => self.emit("}"),
                    Some(ElseBranch::Block(block)) => {
                        self.emit("} else {");
                        self.indent += 1;
                        for inner in &block.statements {
                            self.gen_stmt(inner)?;
                        }
                        self.indent -= 1;
                        self.emit("}");
                    }
                    Some(ElseBranch::IfStmt(if_stmt)) => {
                        self.emit("} else {");
                        self.indent += 1;
                        self.gen_stmt(if_stmt)?;
                        self.indent -= 1;
                        self.emit("}");
                    }
                }
            }
            Stmt::WhileStmt { condition, body } => {
                self.emit(&format!("while ({}) {{", self.gen_expr(condition)?));
                self.indent += 1;
                self.push_scope();
                for inner in &body.statements {
                    self.gen_stmt(inner)?;
                }
                self.pop_scope();
                self.indent -= 1;
                self.emit("}");
            }
            Stmt::ForStmt {
                iterator,
                iterable,
                body,
            } => self.gen_for_stmt(iterator, iterable, body)?,
            Stmt::ExprStmt { expr } => self.emit(&format!("{};", self.gen_expr(expr)?)),
            Stmt::BreakStmt => self.emit("break;"),
            Stmt::ContinueStmt => self.emit("continue;"),
            Stmt::MatchStmt { value, arms } => self.gen_match_stmt(value, arms)?,
        }
        Ok(())
    }

    /// If `value` is a Some/None/Ok/Err constructor for the Option/Result `ty`,
    /// render the C compound literal; otherwise return `None`.
    fn try_constructor(&self, ty: &TypeNode, value: &Spanned<Expr>) -> XResult<Option<String>> {
        let TypeNode::TypeExpr { name, args } = ty else {
            return Ok(None);
        };
        let alias = self.c_type(ty)?;
        match (name.as_str(), args.len()) {
            ("Option", 1) => match &value.node {
                Expr::CallExpr {
                    callee,
                    args: cargs,
                } if matches!(&callee.node, Expr::Identifier { name: n } if n == "Some")
                    && cargs.len() == 1 =>
                {
                    let v = self.gen_expr(&cargs[0])?;
                    Ok(Some(format!("({alias}){{ .some = true, .value = {v} }}")))
                }
                Expr::Identifier { name: n } if n == "None" => {
                    Ok(Some(format!("({alias}){{ .some = false }}")))
                }
                _ => Ok(None),
            },
            ("Result", 2) => match &value.node {
                Expr::CallExpr {
                    callee,
                    args: cargs,
                } if matches!(&callee.node, Expr::Identifier { name: n } if n == "Ok")
                    && cargs.len() == 1 =>
                {
                    let v = self.gen_expr(&cargs[0])?;
                    Ok(Some(format!("({alias}){{ .ok = true, .value = {v} }}")))
                }
                Expr::CallExpr {
                    callee,
                    args: cargs,
                } if matches!(&callee.node, Expr::Identifier { name: n } if n == "Err")
                    && cargs.len() == 1 =>
                {
                    let v = self.gen_expr(&cargs[0])?;
                    Ok(Some(format!("({alias}){{ .ok = false, .error = {v} }}")))
                }
                _ => Ok(None),
            },
            ("Vec", 1) => match &value.node {
                Expr::CallExpr {
                    callee,
                    args: cargs,
                } if matches!(&callee.node, Expr::Identifier { name: n } if n == "vec_new")
                    && cargs.is_empty() =>
                {
                    Ok(Some(format!(
                        "({alias}){{ .data = 0, .len = 0, .cap = 0 }}"
                    )))
                }
                _ => Ok(None),
            },
            _ => Ok(None),
        }
    }

    fn gen_let_stmt(&mut self, name: &str, ty: &TypeNode, value: &Spanned<Expr>) -> XResult<()> {
        if let Expr::ArrayLiteral { elements } = &value.node {
            self.gen_array_let_stmt(name, ty, elements)?;
        } else if let Some(rendered) = self.try_constructor(ty, value)? {
            self.emit(&format!("{} {} = {};", self.c_type(ty)?, name, rendered));
        } else {
            self.emit(&format!(
                "{} {} = {};",
                self.c_type(ty)?,
                name,
                self.gen_expr(value)?
            ));
        }
        self.declare_var(name, ty.clone());
        Ok(())
    }

    fn gen_array_let_stmt(
        &mut self,
        name: &str,
        ty: &TypeNode,
        elements: &[Spanned<Expr>],
    ) -> XResult<()> {
        let TypeNode::TypeExpr {
            name: ty_name,
            args,
        } = ty
        else {
            return Err(XError::Codegen(
                "array literal initializer requires an Array<T, N> type annotation".to_string(),
            ));
        };
        if ty_name != "Array" || args.len() != 2 {
            return Err(XError::Codegen(format!(
                "array literal initializer requires Array<T, N>, got {ty_name}<...>"
            )));
        }

        let declared_len = self.const_type_arg_value(&args[1], "Array length")?;
        let declared_len = declared_len.parse::<usize>().map_err(|_| {
            XError::Codegen(format!(
                "Array length must fit usize for codegen, got {declared_len:?}"
            ))
        })?;
        if elements.len() != declared_len {
            return Err(XError::Codegen(format!(
                "Array literal length mismatch: Array expects {declared_len} elements, got {}",
                elements.len()
            )));
        }

        let mut rendered_elements = Vec::new();
        for element in elements {
            rendered_elements.push(self.gen_expr(element)?);
        }
        self.emit(&format!(
            "{} {} = {{ .data = {{{}}} }};",
            self.c_type(ty)?,
            name,
            rendered_elements.join(", ")
        ));
        Ok(())
    }

    fn gen_for_stmt(
        &mut self,
        iterator: &str,
        iterable: &Spanned<Expr>,
        body: &Block,
    ) -> XResult<()> {
        // Numeric range `for i in start..end` (or `start..=end`) -> C
        // `for (i = start; i (<|<=) end; i++)`.
        if let Expr::RangeExpr {
            start,
            end,
            inclusive,
        } = &iterable.node
        {
            return self.gen_range_for(iterator, start, end, *inclusive, body);
        }

        // Resolve the iterable's type: from a typed variable, or (for arbitrary
        // expressions like `for x in self.items` / `for x in get_vec()`) from the
        // type map. Lifts the old identifier-only restriction.
        let iter_ty = if let Expr::Identifier { name } = &iterable.node {
            self.lookup_var(name).cloned()
        } else {
            self.types.type_node(iterable)
        };
        let Some(TypeNode::TypeExpr {
            name: ty_name,
            args,
        }) = iter_ty
        else {
            return Err(XError::Codegen(
                "for-in iterable has an unknown type (annotate it or bind it to a variable first)"
                    .to_string(),
            ));
        };
        // A non-identifier iterable is bound to a temp so it is evaluated once
        // and `.len` / `.data` refer to a stable lvalue. (Identifiers are
        // already lvalues, so no temp.)
        let iter_c = if let Expr::Identifier { name } = &iterable.node {
            name.clone()
        } else {
            let c_ty = self.c_type(&TypeNode::TypeExpr {
                name: ty_name.clone(),
                args: args.clone(),
            })?;
            let temp = self.next_temp("it");
            let init = self.gen_expr(iterable)?;
            self.emit(&format!("{c_ty} {temp} = {init};"));
            temp
        };
        // Loop bound + element source differ: Slice uses a runtime `.len`;
        // Array<T, N> uses the compile-time N. Both store elements in `.data`.
        let (elem_ty, bound, data) = match (ty_name.as_str(), args.len()) {
            ("Slice", 1) => (
                args[0].clone(),
                format!("{iter_c}.len"),
                format!("{iter_c}.data"),
            ),
            ("Array", 2) => {
                let n = self.const_type_arg_value(&args[1], "Array length")?;
                (args[0].clone(), n.to_string(), format!("{iter_c}.data"))
            }
            ("Vec", 1) => (
                args[0].clone(),
                format!("{iter_c}.len"),
                format!("{iter_c}.data"),
            ),
            _ => {
                return Err(XError::Codegen(format!(
                    "C backend only supports for-in over Slice<T> or Array<T, N>, got {ty_name}<...>"
                )));
            }
        };
        let elem_c_type = self.c_type(&elem_ty)?;
        let index = self.next_temp("i");

        self.emit(&format!(
            "for (size_t {index} = 0; {index} < {bound}; {index}++) {{"
        ));
        self.indent += 1;
        self.push_scope();
        self.declare_var(iterator, elem_ty);
        self.emit(&format!("{elem_c_type} {iterator} = {data}[{index}];"));
        for inner in &body.statements {
            self.gen_stmt(inner)?;
        }
        self.pop_scope();
        self.indent -= 1;
        self.emit("}");
        Ok(())
    }

    /// Lower `for i in start..end` (or `start..=end`) to a C numeric for loop.
    /// The end bound is captured into a temp once so a loop like
    /// `for i in 0..vec.len` evaluates the bound a single time (matching the
    /// for-in-over-collection semantics, where the bound is fixed at loop entry).
    /// `inclusive` selects `<` (exclusive `..`) vs `<=` (inclusive `..=`).
    fn gen_range_for(
        &mut self,
        iterator: &str,
        start: &Spanned<Expr>,
        end: &Spanned<Expr>,
        inclusive: bool,
        body: &Block,
    ) -> XResult<()> {
        let start_c = self.gen_expr(start)?;
        let end_c = self.gen_expr(end)?;
        let end_tmp = self.next_temp("rg_end");
        let cmp = if inclusive { "<=" } else { "<" };
        // Wrap in a block so the captured bound temp doesn't leak, and so the
        // iterator name can shadow an outer variable of the same name safely.
        self.emit("{");
        self.indent += 1;
        self.emit(&format!("int32_t {end_tmp} = {end_c};"));
        self.emit(&format!(
            "for (int32_t {iterator} = {start_c}; {iterator} {cmp} {end_tmp}; {iterator}++) {{"
        ));
        self.indent += 1;
        self.push_scope();
        self.declare_var(iterator, i32_type());
        for inner in &body.statements {
            self.gen_stmt(inner)?;
        }
        self.pop_scope();
        self.indent -= 1;
        self.emit("}");
        self.indent -= 1;
        self.emit("}");
        Ok(())
    }

    /// The C condition for a literal-match pattern, or `None` for a wildcard
    /// (the fall-through arm). Handles literals, OR-patterns (`a | b`), and
    /// integer ranges (`a..b` / `a..=b`).
    fn pattern_cond(
        &self,
        pattern: &crate::ast::Pattern,
        scrut_c: &str,
        is_string: bool,
        ty_name: &str,
    ) -> XResult<Option<String>> {
        use crate::ast::Pattern;
        let cond = match pattern {
            Pattern::LiteralPattern { value: lit } => {
                if is_string {
                    format!("strcmp({scrut_c}, \"{lit}\") == 0")
                } else {
                    format!("{scrut_c} == {lit}")
                }
            }
            Pattern::OrPattern { alternatives } => {
                let parts: Vec<String> = alternatives
                    .iter()
                    .filter_map(|a| {
                        self.pattern_cond(a, scrut_c, is_string, ty_name)
                            .transpose()
                    })
                    .collect::<Result<_, _>>()?;
                if parts.is_empty() {
                    return Ok(None);
                }
                parts.join(" || ")
            }
            Pattern::RangePattern {
                start,
                end,
                inclusive,
            } => {
                if is_string {
                    return Err(XError::Codegen(format!(
                        "range pattern not supported in match on {ty_name}"
                    )));
                }
                if *inclusive {
                    format!("({scrut_c} >= {start} && {scrut_c} <= {end})")
                } else {
                    format!("({scrut_c} >= {start} && {scrut_c} < {end})")
                }
            }
            Pattern::WildcardPattern => return Ok(None),
            Pattern::VariantPattern { name, .. } => {
                // A unit-variant enum pattern resolves to its integer value:
                // `North =>` → `scrut == <value>`.
                if let Some(v) = self.enum_values.get(name) {
                    return Ok(Some(format!("{scrut_c} == {v}")));
                }
                return Err(XError::Codegen(format!(
                    "variant pattern {name:?} not supported in literal match on {ty_name}"
                )));
            }
        };
        Ok(Some(cond))
    }

    /// Match a payload enum: if-else on `scrut.tag`, binding each arm's
    /// payload from `scrut.u.v<idx>` (e.g. `Err(msg) =>` → `msg = scrut.u.v1`).
    fn gen_enum_struct_match(
        &mut self,
        value: &Spanned<Expr>,
        arms: &[MatchArm],
        ty_name: &str,
    ) -> XResult<()> {
        // The scrutinee is a struct; bind a temp if it isn't an lvalue.
        let scrut_c = if let Expr::Identifier { name } = &value.node {
            name.clone()
        } else {
            let c_ty = self.c_type(&TypeNode::TypeExpr {
                name: ty_name.to_string(),
                args: vec![],
            })?;
            let temp = self.next_temp("e");
            let init = self.gen_expr(value)?;
            self.emit(&format!("{c_ty} {temp} = {init};"));
            temp
        };
        let payloads = self.enum_payloads.get(ty_name).cloned();
        let mut first = true;
        let mut wildcard_body: Option<&crate::ast::Block> = None;

        for arm in arms {
            match &arm.pattern {
                crate::ast::Pattern::VariantPattern { name, bindings } => {
                    let Some(idx) = self.enum_values.get(name).copied() else {
                        continue;
                    };
                    if first {
                        self.emit(&format!("if ({scrut_c}.tag == {idx}) {{"));
                        first = false;
                    } else {
                        self.emit(&format!("}} else if ({scrut_c}.tag == {idx}) {{"));
                    }
                    self.indent += 1;
                    self.push_scope();
                    if bindings.len() == 1 {
                        let mut payload_ty: Option<TypeNode> = None;
                        if let Some(ps) = payloads.as_ref()
                            && let Some(Some(ty)) = ps.get(idx as usize)
                        {
                            payload_ty = Some(ty.clone());
                        }
                        if let Some(payload_ty) = payload_ty {
                            let c_ty = self.c_type(&payload_ty)?;
                            self.emit(&format!("{c_ty} {} = {scrut_c}.u.v{idx};", bindings[0]));
                            self.declare_var(&bindings[0], payload_ty);
                        }
                    }
                    for inner in &arm.body.statements {
                        self.gen_stmt(inner)?;
                    }
                    self.pop_scope();
                    self.indent -= 1;
                }
                crate::ast::Pattern::WildcardPattern => {
                    wildcard_body = Some(&arm.body);
                }
                _ => {}
            }
        }

        if let Some(body) = wildcard_body {
            if first {
                self.push_scope();
                for inner in &body.statements {
                    self.gen_stmt(inner)?;
                }
                self.pop_scope();
            } else {
                self.emit("} else {");
                self.indent += 1;
                self.push_scope();
                for inner in &body.statements {
                    self.gen_stmt(inner)?;
                }
                self.pop_scope();
                self.indent -= 1;
                self.emit("}");
            }
        } else if !first {
            self.emit("}");
        }
        Ok(())
    }

    /// Lower `match scrut { Some/Ok(v) => .., None/Err(..) => .. }` to a C
    /// `if/else` on the discriminant. v1: `scrut` must be a variable of type
    /// `Option<T>` or `Result<T, E>`.
    /// Generate if-else chains for match on i32/String literals + wildcard.
    fn gen_literal_match(
        &mut self,
        value: &Spanned<Expr>,
        arms: &[MatchArm],
        ty_name: &str,
    ) -> XResult<()> {
        let scrut_c = self.gen_expr(value)?;
        let is_string = matches!(ty_name, "String" | "Str");
        let mut first = true;
        let mut wildcard_body: Option<&crate::ast::Block> = None;

        for arm in arms {
            if let Some(cond) = self.pattern_cond(&arm.pattern, &scrut_c, is_string, ty_name)? {
                if first {
                    self.emit(&format!("if ({cond}) {{"));
                    first = false;
                } else {
                    self.emit(&format!("}} else if ({cond}) {{"));
                }
                self.indent += 1;
                self.push_scope();
                for inner in &arm.body.statements {
                    self.gen_stmt(inner)?;
                }
                self.pop_scope();
                self.indent -= 1;
            } else {
                // Wildcard (or unsupported) — the fall-through arm.
                wildcard_body = Some(&arm.body);
            }
        }

        if let Some(body) = wildcard_body {
            if first {
                self.push_scope();
                for inner in &body.statements {
                    self.gen_stmt(inner)?;
                }
                self.pop_scope();
            } else {
                self.emit("} else {");
                self.indent += 1;
                self.push_scope();
                for inner in &body.statements {
                    self.gen_stmt(inner)?;
                }
                self.pop_scope();
                self.indent -= 1;
                self.emit("}");
            }
        } else if !first {
            self.emit("}");
        }
        Ok(())
    }

    fn gen_match_stmt(&mut self, value: &Spanned<Expr>, arms: &[MatchArm]) -> XResult<()> {
        // Resolve the scrutinee's type: from a typed variable, or (for arbitrary
        // expressions like `match func() { .. }` and `if let Pat = func() { }`)
        // from the type map. This lifts the old identifier-only restriction.
        let scrut_ty = if let Expr::Identifier { name } = &value.node {
            self.lookup_var(name).cloned()
        } else {
            self.types.type_node(value)
        };
        let Some(TypeNode::TypeExpr {
            name: ty_name,
            args,
        }) = scrut_ty
        else {
            return Err(XError::Codegen(
                "match scrutinee has an unknown type (annotate it or bind it to a variable first)"
                    .to_string(),
            ));
        };
        // Literal match for i32 / String scrutinees, and for unit-only enums
        // (a variant pattern resolves to its integer value → scrut == value).
        if matches!(ty_name.as_str(), "i32" | "String" | "Str")
            || (self.enum_names.contains(&ty_name) && !self.enum_has_payload(&ty_name))
        {
            return self.gen_literal_match(value, arms, &ty_name);
        }
        // Payload enum → match on the tag, binding the variant's payload.
        if self.enum_has_payload(&ty_name) {
            return self.gen_enum_struct_match(value, arms, &ty_name);
        }
        let is_option = match (ty_name.as_str(), args.len()) {
            ("Option", 1) => true,
            ("Result", 2) => false,
            _ => {
                return Err(XError::Codegen(format!(
                    "match supports Option<T> / Result<T, E>, got {ty_name}"
                )));
            }
        };
        let discriminant = if is_option { "some" } else { "ok" };
        let payload_ty = args[0].clone();
        let err_ty = if is_option {
            None
        } else {
            Some(args[1].clone())
        };

        let mut positive: Option<&MatchArm> = None;
        let mut negative: Option<&MatchArm> = None;
        for arm in arms {
            let crate::ast::Pattern::VariantPattern { name, .. } = &arm.pattern else {
                continue;
            };
            match name.as_str() {
                "Some" | "Ok" => positive = Some(arm),
                "None" | "Err" => negative = Some(arm),
                other => {
                    return Err(XError::Codegen(format!(
                        "C backend does not support match variant {other:?}"
                    )));
                }
            }
        }

        // The Option/Result match reads `.some`/`.value`, so it needs an
        // lvalue. A plain variable is one; anything else is bound to a typed
        // temp first (so `match func() {..}` / `if let Some(v) = func() {..}`
        // work).
        let scrut_c = if let Expr::Identifier { name } = &value.node {
            name.clone()
        } else {
            let ty = self.types.type_node(value).ok_or_else(|| {
                XError::Codegen(
                    "match scrutinee has an unknown type (annotate it or bind it to a variable first)"
                        .to_string(),
                )
            })?;
            let c_ty = self.c_type(&ty)?;
            let temp = self.next_temp("m");
            let init = self.gen_expr(value)?;
            self.emit(&format!("{c_ty} {temp} = {init};"));
            temp
        };
        self.emit(&format!("if ({scrut_c}.{discriminant}) {{"));
        self.indent += 1;
        self.push_scope();
        if let Some(arm) = positive {
            let crate::ast::Pattern::VariantPattern { bindings, .. } = &arm.pattern else {
                unreachable!("non-variant arm in Option/Result match")
            };
            if let Some(binding) = bindings.first() {
                let payload_c = self.c_type(&payload_ty)?;
                self.declare_var(binding, payload_ty.clone());
                self.emit(&format!("{payload_c} {binding} = {scrut_c}.value;"));
            }
            for inner in &arm.body.statements {
                self.gen_stmt(inner)?;
            }
        }
        self.pop_scope();
        self.indent -= 1;
        if let Some(arm) = negative {
            self.emit("} else {");
            self.indent += 1;
            self.push_scope();
            if let Some(err_ty) = &err_ty {
                let crate::ast::Pattern::VariantPattern { bindings, .. } = &arm.pattern else {
                    unreachable!()
                };
                if let Some(binding) = bindings.first() {
                    let err_c = self.c_type(err_ty)?;
                    self.declare_var(binding, err_ty.clone());
                    self.emit(&format!("{err_c} {binding} = {scrut_c}.error;"));
                }
            }
            for inner in &arm.body.statements {
                self.gen_stmt(inner)?;
            }
            self.pop_scope();
            self.indent -= 1;
        }
        self.emit("}");
        Ok(())
    }

    /// Recognise the print builtins (`print_i32`/`print_f64`/`print_str`/
    /// `print_bool`) and lower a one-arg call to a `printf`; returns None for
    /// anything else so the normal call path handles it.
    /// Emit the small C runtime preamble — helpers that need allocation (string
    /// concatenation, int->str). Non-static so an unused helper doesn't trip
    /// -Wunused-function.
    fn emit_runtime_preamble(&mut self) {
        let lines = [
            "int __xlang_argc_g = 0;",
            "char** __xlang_argv_g = 0;",
            "char* __xlang_str_concat(const char* a, const char* b) {",
            "    size_t la = strlen(a), lb = strlen(b);",
            "    char* out = (char*)malloc(la + lb + 1);",
            "    memcpy(out, a, la);",
            "    memcpy(out + la, b, lb);",
            "    out[la + lb] = 0;",
            "    return out;",
            "}",
            "char* __xlang_int_to_str(int32_t n) {",
            "    char* buf = (char*)malloc(16);",
            "    snprintf(buf, 16, \"%d\", n);",
            "    return buf;",
            "}",
            "// assert / panic / unreachable — program-invariant self-checks.",
            "// Each prints to stderr and exits non-zero (so a failing assertion",
            "// surfaces as a non-zero exit code, not silent wrong output).",
            "void __xlang_assert(int32_t cond, const char* msg) {",
            "    if (!cond) { fprintf(stderr, \"xlang assertion failed: %s\\n\", msg ? msg : \"\"); exit(1); }",
            "}",
            "void __xlang_panic(const char* msg) {",
            "    fprintf(stderr, \"xlang panic: %s\\n\", msg ? msg : \"\"); exit(1);",
            "}",
            "void __xlang_unreachable_(void) {",
            "    fprintf(stderr, \"xlang: reached unreachable\\n\"); exit(1);",
            "}",
            "// SHA-256 hash → 64-char hex string. Standard FIPS 180-4 implementation.",
            "char* __xlang_sha256_hex(const char* data) {",
            "    static const uint32_t K[64] = {",
            "        0x428a2f98,0x71374491,0xb5c0fbcf,0xe9b5dba5,0x3956c25b,0x59f111f1,0x923f82a4,0xab1c5ed5,",
            "        0xd807aa98,0x12835b01,0x243185be,0x550c7dc3,0x72be5d74,0x80deb1fe,0x9bdc06a7,0xc19bf174,",
            "        0xe49b69c1,0xefbe4786,0x0fc19dc6,0x240ca1cc,0x2de92c6f,0x4a7484aa,0x5cb0a9dc,0x76f988da,",
            "        0x983e5152,0xa831c66d,0xb00327c8,0xbf597fc7,0xc6e00bf3,0xd5a79147,0x06ca6351,0x14292967,",
            "        0x27b70a85,0x2e1b2138,0x4d2c6dfc,0x53380d13,0x650a7354,0x766a0abb,0x81c2c92e,0x92722c85,",
            "        0xa2bfe8a1,0xa81a664b,0xc24b8b70,0xc76c51a3,0xd192e819,0xd6990624,0xf40e3585,0x106aa070,",
            "        0x19a4c116,0x1e376c08,0x2748774c,0x34b0bcb5,0x391c0cb3,0x4ed8aa4a,0x5b9cca4f,0x682e6ff3,",
            "        0x748f82ee,0x78a5636f,0x84c87814,0x8cc70208,0x90befffa,0xa4506ceb,0xbef9a3f7,0xc67178f2",
            "    };",
            "    uint32_t h[8] = {0x6a09e667,0xbb67ae85,0x3c6ef372,0xa54ff53a,0x510e527f,0x9b05688c,0x1f83d9ab,0x5be0cd19};",
            "    size_t dlen = strlen(data);",
            "    size_t padded = ((dlen + 9 + 63) / 64) * 64;",
            "    uint8_t* msg = (uint8_t*)calloc(padded, 1);",
            "    memcpy(msg, data, dlen);",
            "    msg[dlen] = 0x80;",
            "    uint64_t bits = (uint64_t)dlen * 8;",
            "    for (int i = 0; i < 8; i++) msg[padded - 1 - i] = (uint8_t)(bits >> (i * 8));",
            "    for (size_t off = 0; off < padded; off += 64) {",
            "        uint32_t w[64];",
            "        for (int i = 0; i < 16; i++) w[i] = ((uint32_t)msg[off+i*4]<<24)|((uint32_t)msg[off+i*4+1]<<16)|((uint32_t)msg[off+i*4+2]<<8)|((uint32_t)msg[off+i*4+3]);",
            "        for (int i = 16; i < 64; i++) {",
            "            uint32_t s0 = ((w[i-15]>>7)|(w[i-15]<<25)) ^ ((w[i-15]>>18)|(w[i-15]<<14)) ^ (w[i-15]>>3);",
            "            uint32_t s1 = ((w[i-2]>>17)|(w[i-2]<<15)) ^ ((w[i-2]>>19)|(w[i-2]<<13)) ^ (w[i-2]>>10);",
            "            w[i] = w[i-16] + s0 + w[i-7] + s1;",
            "        }",
            "        uint32_t a=h[0],b=h[1],c=h[2],d=h[3],e=h[4],f=h[5],g=h[6],hh=h[7];",
            "        for (int i = 0; i < 64; i++) {",
            "            uint32_t S1 = ((e>>6)|(e<<26)) ^ ((e>>11)|(e<<21)) ^ ((e>>25)|(e<<7));",
            "            uint32_t ch = (e & f) ^ (~e & g);",
            "            uint32_t t1 = hh + S1 + ch + K[i] + w[i];",
            "            uint32_t S0 = ((a>>2)|(a<<30)) ^ ((a>>13)|(a<<19)) ^ ((a>>22)|(a<<10));",
            "            uint32_t maj = (a & b) ^ (a & c) ^ (b & c);",
            "            uint32_t t2 = S0 + maj;",
            "            hh=g; g=f; f=e; e=d+t1; d=c; c=b; b=a; a=t1+t2;",
            "        }",
            "        h[0]+=a; h[1]+=b; h[2]+=c; h[3]+=d; h[4]+=e; h[5]+=f; h[6]+=g; h[7]+=hh;",
            "    }",
            "    free(msg);",
            "    char* hex = (char*)malloc(65);",
            "    const char* hc = \"0123456789abcdef\";",
            "    for (int i = 0; i < 8; i++) { hex[i*8]=(char)hc[(h[i]>>28)&15]; hex[i*8+1]=(char)hc[(h[i]>>24)&15]; hex[i*8+2]=(char)hc[(h[i]>>20)&15]; hex[i*8+3]=(char)hc[(h[i]>>16)&15]; hex[i*8+4]=(char)hc[(h[i]>>12)&15]; hex[i*8+5]=(char)hc[(h[i]>>8)&15]; hex[i*8+6]=(char)hc[(h[i]>>4)&15]; hex[i*8+7]=(char)hc[h[i]&15]; }",
            "    hex[64] = 0;",
            "    return hex;",
            "}",
            "// SHA-224 hash (FIPS 180-4) — SHA-256 with different IV, truncated to 56 hex chars.",
            "char* __xlang_sha224_hex(const char* data) {",
            "    static const uint32_t K[64] = {",
            "        0x428a2f98,0x71374491,0xb5c0fbcf,0xe9b5dba5,0x3956c25b,0x59f111f1,0x923f82a4,0xab1c5ed5,",
            "        0xd807aa98,0x12835b01,0x243185be,0x550c7dc3,0x72be5d74,0x80deb1fe,0x9bdc06a7,0xc19bf174,",
            "        0xe49b69c1,0xefbe4786,0x0fc19dc6,0x240ca1cc,0x2de92c6f,0x4a7484aa,0x5cb0a9dc,0x76f988da,",
            "        0x983e5152,0xa831c66d,0xb00327c8,0xbf597fc7,0xc6e00bf3,0xd5a79147,0x06ca6351,0x14292967,",
            "        0x27b70a85,0x2e1b2138,0x4d2c6dfc,0x53380d13,0x650a7354,0x766a0abb,0x81c2c92e,0x92722c85,",
            "        0xa2bfe8a1,0xa81a664b,0xc24b8b70,0xc76c51a3,0xd192e819,0xd6990624,0xf40e3585,0x106aa070,",
            "        0x19a4c116,0x1e376c08,0x2748774c,0x34b0bcb5,0x391c0cb3,0x4ed8aa4a,0x5b9cca4f,0x682e6ff3,",
            "        0x748f82ee,0x78a5636f,0x84c87814,0x8cc70208,0x90befffa,0xa4506ceb,0xbef9a3f7,0xc67178f2",
            "    };",
            "    uint32_t h[8]={0xc1059ed8,0x367cd507,0x3070dd17,0xf70e5939,0xffc00b31,0x68581511,0x64f98fa7,0xbefa4fa4};",
            "    size_t dlen=strlen(data),padded=((dlen+9+63)/64)*64;",
            "    uint8_t* msg=(uint8_t*)calloc(padded,1);",
            "    memcpy(msg,data,dlen);",
            "    msg[dlen]=0x80;",
            "    uint64_t bits=(uint64_t)dlen*8;",
            "    for(int i=0;i<8;i++) msg[padded-1-i]=(uint8_t)(bits>>(i*8));",
            "    for(size_t off=0;off<padded;off+=64){",
            "        uint32_t w[64];",
            "        for(int i=0;i<16;i++) w[i]=((uint32_t)msg[off+i*4]<<24)|((uint32_t)msg[off+i*4+1]<<16)|((uint32_t)msg[off+i*4+2]<<8)|((uint32_t)msg[off+i*4+3]);",
            "        for(int i=16;i<64;i++){uint32_t s0=((w[i-15]>>7)|(w[i-15]<<25))^((w[i-15]>>18)|(w[i-15]<<14))^(w[i-15]>>3);uint32_t s1=((w[i-2]>>17)|(w[i-2]<<15))^((w[i-2]>>19)|(w[i-2]<<13))^(w[i-2]>>10);w[i]=w[i-16]+s0+w[i-7]+s1;}",
            "        uint32_t a=h[0],b=h[1],c=h[2],d=h[3],e=h[4],f=h[5],g=h[6],hh=h[7];",
            "        for(int i=0;i<64;i++){uint32_t S1=((e>>6)|(e<<26))^((e>>11)|(e<<21))^((e>>25)|(e<<7));uint32_t ch=(e&f)^(~e&g);uint32_t t1=hh+S1+ch+K[i]+w[i];uint32_t S0=((a>>2)|(a<<30))^((a>>13)|(a<<19))^((a>>22)|(a<<10));uint32_t maj=(a&b)^(a&c)^(b&c);uint32_t t2=S0+maj;hh=g;g=f;f=e;e=d+t1;d=c;c=b;b=a;a=t1+t2;}",
            "        h[0]+=a;h[1]+=b;h[2]+=c;h[3]+=d;h[4]+=e;h[5]+=f;h[6]+=g;h[7]+=hh;",
            "    }",
            "    free(msg);",
            "    char* hex=(char*)malloc(57);",
            "    const char* hc=\"0123456789abcdef\";",
            "    for(int i=0;i<7;i++){hex[i*8]=(char)hc[(h[i]>>28)&15];hex[i*8+1]=(char)hc[(h[i]>>24)&15];hex[i*8+2]=(char)hc[(h[i]>>20)&15];hex[i*8+3]=(char)hc[(h[i]>>16)&15];hex[i*8+4]=(char)hc[(h[i]>>12)&15];hex[i*8+5]=(char)hc[(h[i]>>8)&15];hex[i*8+6]=(char)hc[(h[i]>>4)&15];hex[i*8+7]=(char)hc[h[i]&15];}",
            "    hex[56]=0;",
            "    return hex;",
            "}",
            "char* __xlang_pad_int(int32_t n, int32_t width) {",
            "    char* buf = (char*)malloc(32);",
            "    snprintf(buf, 32, \"%*d\", width, n);",
            "    return buf;",
            "}",
            "// Zero-pad n to `width` digits (%0*d): 5,3 → \"005\". Used by seq -w.",
            "char* __xlang_pad_zero(int32_t n, int32_t width) {",
            "    char* buf = (char*)malloc(32);",
            "    snprintf(buf, 32, \"%0*d\", width, n);",
            "    return buf;",
            "}",
            "// SHA-512 hash (FIPS 180-4) → 128-char hex string.",
            "char* __xlang_sha512_hex(const char* data) {",
            "    static const uint64_t K[80]={",
            "        0x428a2f98d728ae22ULL,0x7137449123ef65cdULL,0xb5c0fbcfec4d3b2fULL,0xe9b5dba58189dbbcULL,",
            "        0x3956c25bf348b538ULL,0x59f111f1b605d019ULL,0x923f82a4af194f9bULL,0xab1c5ed5da6d8118ULL,",
            "        0xd807aa98a3030242ULL,0x12835b0145706fbeULL,0x243185be4ee4b28cULL,0x550c7dc3d5ffb4e2ULL,",
            "        0x72be5d74f27b896fULL,0x80deb1fe3b1696b1ULL,0x9bdc06a725c71235ULL,0xc19bf174cf692694ULL,",
            "        0xe49b69c19ef14ad2ULL,0xefbe4786384f25e3ULL,0x0fc19dc68b8cd5b5ULL,0x240ca1cc77ac9c65ULL,",
            "        0x2de92c6f592b0275ULL,0x4a7484aa6ea6e483ULL,0x5cb0a9dcbd41fbd4ULL,0x76f988da831153b5ULL,",
            "        0x983e5152ee66dfabULL,0xa831c66d2db43210ULL,0xb00327c898fb213fULL,0xbf597fc7beef0ee4ULL,",
            "        0xc6e00bf33da88fc2ULL,0xd5a79147930aa725ULL,0x06ca6351e003826fULL,0x142929670a0e6e70ULL,",
            "        0x27b70a8546d22ffcULL,0x2e1b21385c26c926ULL,0x4d2c6dfc5ac42aedULL,0x53380d139d95b3dfULL,",
            "        0x650a73548baf63deULL,0x766a0abb3c77b2a8ULL,0x81c2c92e47edaee6ULL,0x92722c851482353bULL,",
            "        0xa2bfe8a14cf10364ULL,0xa81a664bbc423001ULL,0xc24b8b70d0f89791ULL,0xc76c51a30654be30ULL,",
            "        0xd192e819d6ef5218ULL,0xd69906245565a910ULL,0xf40e35855771202aULL,0x106aa07032bbd1b8ULL,",
            "        0x19a4c116b8d2d0c8ULL,0x1e376c085141ab53ULL,0x2748774cdf8eeb99ULL,0x34b0bcb5e19b48a8ULL,",
            "        0x391c0cb3c5c95a63ULL,0x4ed8aa4ae3418acbULL,0x5b9cca4f7763e373ULL,0x682e6ff3d6b2b8a3ULL,",
            "        0x748f82ee5defb2fcULL,0x78a5636f43172f60ULL,0x84c87814a1f0ab72ULL,0x8cc702081a6439ecULL,",
            "        0x90befffa23631e28ULL,0xa4506cebde82bde9ULL,0xbef9a3f7b2c67915ULL,0xc67178f2e372532bULL,",
            "        0xca273eceea26619cULL,0xd186b8c721c0c207ULL,0xeada7dd6cde0eb1eULL,0xf57d4f7fee6ed178ULL,",
            "        0x06f067aa72176fbaULL,0x0a637dc5a2c898a6ULL,0x113f9804bef90daeULL,0x1b710b35131c471bULL,",
            "        0x28db77f523047d84ULL,0x32caab7b40c72493ULL,0x3c9ebe0a15c9bebcULL,0x431d67c49c100d4cULL,",
            "        0x4cc5d4becb3e42b6ULL,0x597f299cfc657e2aULL,0x5fcb6fab3ad6faecULL,0x6c44198c4a475817ULL",
            "    };",
            "    uint64_t h[8]={0x6a09e667f3bcc908ULL,0xbb67ae8584caa73bULL,0x3c6ef372fe94f82bULL,0xa54ff53a5f1d36f1ULL,0x510e527fade682d1ULL,0x9b05688c2b3e6c1fULL,0x1f83d9abfb41bd6bULL,0x5be0cd19137e2179ULL};",
            "    size_t dlen=strlen(data);",
            "    size_t padded=((dlen+17+127)/128)*128;",
            "    uint8_t* msg=(uint8_t*)calloc(padded,1);",
            "    memcpy(msg,data,dlen);",
            "    msg[dlen]=0x80;",
            "    uint64_t bits=(uint64_t)dlen*8;",
            "    for(int i=0;i<8;i++) msg[padded-1-i]=(uint8_t)(bits>>(i*8));",
            "    for(size_t off=0;off<padded;off+=128){",
            "        uint64_t w[80];",
            "        for(int i=0;i<16;i++){size_t b=off+i*8;w[i]=((uint64_t)msg[b]<<56)|((uint64_t)msg[b+1]<<48)|((uint64_t)msg[b+2]<<40)|((uint64_t)msg[b+3]<<32)|((uint64_t)msg[b+4]<<24)|((uint64_t)msg[b+5]<<16)|((uint64_t)msg[b+6]<<8)|((uint64_t)msg[b+7]);}",
            "        for(int i=16;i<80;i++){uint64_t s0=((w[i-15]>>1)|(w[i-15]<<63))^((w[i-15]>>8)|(w[i-15]<<56))^(w[i-15]>>7);uint64_t s1=((w[i-2]>>19)|(w[i-2]<<45))^((w[i-2]>>61)|(w[i-2]<<3))^(w[i-2]>>6);w[i]=w[i-16]+s0+w[i-7]+s1;}",
            "        uint64_t a=h[0],b=h[1],c=h[2],d=h[3],e=h[4],f=h[5],g=h[6],hh=h[7];",
            "        for(int i=0;i<80;i++){",
            "            uint64_t S1=((e>>14)|(e<<50))^((e>>18)|(e<<46))^((e>>41)|(e<<23));",
            "            uint64_t ch=(e&f)^(~e&g);",
            "            uint64_t t1=hh+S1+ch+K[i]+w[i];",
            "            uint64_t S0=((a>>28)|(a<<36))^((a>>34)|(a<<30))^((a>>39)|(a<<25));",
            "            uint64_t maj=(a&b)^(a&c)^(b&c);",
            "            uint64_t t2=S0+maj;",
            "            hh=g;g=f;f=e;e=d+t1;d=c;c=b;b=a;a=t1+t2;",
            "        }",
            "        h[0]+=a;h[1]+=b;h[2]+=c;h[3]+=d;h[4]+=e;h[5]+=f;h[6]+=g;h[7]+=hh;",
            "    }",
            "    free(msg);",
            "    char* hex=(char*)malloc(129);",
            "    const char* hc=\"0123456789abcdef\";",
            "    for(int i=0;i<8;i++){for(int j=7;j>=0;j--){hex[i*16+(7-j)*2]=(char)hc[(h[i]>>(j*8+4))&15];hex[i*16+(7-j)*2+1]=(char)hc[(h[i]>>(j*8))&15];}}",
            "    hex[128]=0;",
            "    return hex;",
            "}",
            "// SHA-384 hash (FIPS 180-4) — SHA-512 with different IV, truncated to 96 hex chars.",
            "char* __xlang_sha384_hex(const char* data) {",
            "    static const uint64_t K384[80]={",
            "        0x428a2f98d728ae22ULL,0x7137449123ef65cdULL,0xb5c0fbcfec4d3b2fULL,0xe9b5dba58189dbbcULL,",
            "        0x3956c25bf348b538ULL,0x59f111f1b605d019ULL,0x923f82a4af194f9bULL,0xab1c5ed5da6d8118ULL,",
            "        0xd807aa98a3030242ULL,0x12835b0145706fbeULL,0x243185be4ee4b28cULL,0x550c7dc3d5ffb4e2ULL,",
            "        0x72be5d74f27b896fULL,0x80deb1fe3b1696b1ULL,0x9bdc06a725c71235ULL,0xc19bf174cf692694ULL,",
            "        0xe49b69c19ef14ad2ULL,0xefbe4786384f25e3ULL,0x0fc19dc68b8cd5b5ULL,0x240ca1cc77ac9c65ULL,",
            "        0x2de92c6f592b0275ULL,0x4a7484aa6ea6e483ULL,0x5cb0a9dcbd41fbd4ULL,0x76f988da831153b5ULL,",
            "        0x983e5152ee66dfabULL,0xa831c66d2db43210ULL,0xb00327c898fb213fULL,0xbf597fc7beef0ee4ULL,",
            "        0xc6e00bf33da88fc2ULL,0xd5a79147930aa725ULL,0x06ca6351e003826fULL,0x142929670a0e6e70ULL,",
            "        0x27b70a8546d22ffcULL,0x2e1b21385c26c926ULL,0x4d2c6dfc5ac42aedULL,0x53380d139d95b3dfULL,",
            "        0x650a73548baf63deULL,0x766a0abb3c77b2a8ULL,0x81c2c92e47edaee6ULL,0x92722c851482353bULL,",
            "        0xa2bfe8a14cf10364ULL,0xa81a664bbc423001ULL,0xc24b8b70d0f89791ULL,0xc76c51a30654be30ULL,",
            "        0xd192e819d6ef5218ULL,0xd69906245565a910ULL,0xf40e35855771202aULL,0x106aa07032bbd1b8ULL,",
            "        0x19a4c116b8d2d0c8ULL,0x1e376c085141ab53ULL,0x2748774cdf8eeb99ULL,0x34b0bcb5e19b48a8ULL,",
            "        0x391c0cb3c5c95a63ULL,0x4ed8aa4ae3418acbULL,0x5b9cca4f7763e373ULL,0x682e6ff3d6b2b8a3ULL,",
            "        0x748f82ee5defb2fcULL,0x78a5636f43172f60ULL,0x84c87814a1f0ab72ULL,0x8cc702081a6439ecULL,",
            "        0x90befffa23631e28ULL,0xa4506cebde82bde9ULL,0xbef9a3f7b2c67915ULL,0xc67178f2e372532bULL,",
            "        0xca273eceea26619cULL,0xd186b8c721c0c207ULL,0xeada7dd6cde0eb1eULL,0xf57d4f7fee6ed178ULL,",
            "        0x06f067aa72176fbaULL,0x0a637dc5a2c898a6ULL,0x113f9804bef90daeULL,0x1b710b35131c471bULL,",
            "        0x28db77f523047d84ULL,0x32caab7b40c72493ULL,0x3c9ebe0a15c9bebcULL,0x431d67c49c100d4cULL,",
            "        0x4cc5d4becb3e42b6ULL,0x597f299cfc657e2aULL,0x5fcb6fab3ad6faecULL,0x6c44198c4a475817ULL",
            "    };",
            "    uint64_t h[8]={0xcbbb9d5dc1059ed8ULL,0x629a292a367cd507ULL,0x9159015a3070dd17ULL,0x152fecd8f70e5939ULL,0x67332667ffc00b31ULL,0x8eb44a8768581511ULL,0xdb0c2e0d64f98fa7ULL,0x47b5481dbefa4fa4ULL};",
            "    size_t dlen=strlen(data),padded=((dlen+17+127)/128)*128;",
            "    uint8_t* msg=(uint8_t*)calloc(padded,1);",
            "    memcpy(msg,data,dlen);",
            "    msg[dlen]=0x80;",
            "    uint64_t bits=(uint64_t)dlen*8;",
            "    for(int i=0;i<8;i++) msg[padded-1-i]=(uint8_t)(bits>>(i*8));",
            "    for(size_t off=0;off<padded;off+=128){",
            "        uint64_t w[80];",
            "        for(int i=0;i<16;i++){size_t b=off+i*8;w[i]=((uint64_t)msg[b]<<56)|((uint64_t)msg[b+1]<<48)|((uint64_t)msg[b+2]<<40)|((uint64_t)msg[b+3]<<32)|((uint64_t)msg[b+4]<<24)|((uint64_t)msg[b+5]<<16)|((uint64_t)msg[b+6]<<8)|((uint64_t)msg[b+7]);}",
            "        for(int i=16;i<80;i++){uint64_t s0=((w[i-15]>>1)|(w[i-15]<<63))^((w[i-15]>>8)|(w[i-15]<<56))^(w[i-15]>>7);uint64_t s1=((w[i-2]>>19)|(w[i-2]<<45))^((w[i-2]>>61)|(w[i-2]<<3))^(w[i-2]>>6);w[i]=w[i-16]+s0+w[i-7]+s1;}",
            "        uint64_t a=h[0],b=h[1],c=h[2],d=h[3],e=h[4],f=h[5],g=h[6],hh=h[7];",
            "        for(int i=0;i<80;i++){uint64_t S1=((e>>14)|(e<<50))^((e>>18)|(e<<46))^((e>>41)|(e<<23));uint64_t ch=(e&f)^(~e&g);uint64_t t1=hh+S1+ch+K384[i]+w[i];uint64_t S0=((a>>28)|(a<<36))^((a>>34)|(a<<30))^((a>>39)|(a<<25));uint64_t maj=(a&b)^(a&c)^(b&c);uint64_t t2=S0+maj;hh=g;g=f;f=e;e=d+t1;d=c;c=b;b=a;a=t1+t2;}",
            "        h[0]+=a;h[1]+=b;h[2]+=c;h[3]+=d;h[4]+=e;h[5]+=f;h[6]+=g;h[7]+=hh;",
            "    }",
            "    free(msg);",
            "    char* hex=(char*)malloc(97);",
            "    const char* hc=\"0123456789abcdef\";",
            "    for(int i=0;i<6;i++){for(int j=7;j>=0;j--){hex[i*16+(7-j)*2]=(char)hc[(h[i]>>(j*8+4))&15];hex[i*16+(7-j)*2+1]=(char)hc[(h[i]>>(j*8))&15];}}",
            "    hex[96]=0;",
            "    return hex;",
            "}",
            "// SHA-1 hash (FIPS 180-4) → 40-char hex string.",
            "char* __xlang_sha1_hex(const char* data) {",
            "    uint32_t h[5]={0x67452301,0xEFCDAB89,0x98BADCFE,0x10325476,0xC3D2E1F0};",
            "    size_t dlen=strlen(data);",
            "    size_t padded=((dlen+9+63)/64)*64;",
            "    uint8_t* msg=(uint8_t*)calloc(padded,1);",
            "    memcpy(msg,data,dlen);",
            "    msg[dlen]=0x80;",
            "    uint64_t bits=(uint64_t)dlen*8;",
            "    for(int i=0;i<8;i++) msg[padded-1-i]=(uint8_t)(bits>>(i*8));",
            "    for(size_t off=0;off<padded;off+=64){",
            "        uint32_t w[80];",
            "        for(int i=0;i<16;i++) w[i]=((uint32_t)msg[off+i*4]<<24)|((uint32_t)msg[off+i*4+1]<<16)|((uint32_t)msg[off+i*4+2]<<8)|((uint32_t)msg[off+i*4+3]);",
            "        for(int i=16;i<80;i++){uint32_t t=w[i-3]^w[i-8]^w[i-14]^w[i-16]; w[i]=(t<<1)|(t>>31);}",
            "        uint32_t a=h[0],b=h[1],c=h[2],d=h[3],e=h[4];",
            "        for(int i=0;i<80;i++){",
            "            uint32_t f,k;",
            "            if(i<20){f=(b&c)|(~b&d);k=0x5A827999;}",
            "            else if(i<40){f=b^c^d;k=0x6ED9EBA1;}",
            "            else if(i<60){f=(b&c)|(b&d)|(c&d);k=0x8F1BBCDC;}",
            "            else{f=b^c^d;k=0xCA62C1D6;}",
            "            uint32_t temp=((a<<5)|(a>>27))+f+e+k+w[i];",
            "            e=d;d=c;c=((b<<30)|(b>>2));b=a;a=temp;",
            "        }",
            "        h[0]+=a;h[1]+=b;h[2]+=c;h[3]+=d;h[4]+=e;",
            "    }",
            "    free(msg);",
            "    char* hex=(char*)malloc(41);",
            "    const char* hc=\"0123456789abcdef\";",
            "    for(int i=0;i<5;i++){hex[i*8]=(char)hc[(h[i]>>28)&15];hex[i*8+1]=(char)hc[(h[i]>>24)&15];hex[i*8+2]=(char)hc[(h[i]>>20)&15];hex[i*8+3]=(char)hc[(h[i]>>16)&15];hex[i*8+4]=(char)hc[(h[i]>>12)&15];hex[i*8+5]=(char)hc[(h[i]>>8)&15];hex[i*8+6]=(char)hc[(h[i]>>4)&15];hex[i*8+7]=(char)hc[h[i]&15];}",
            "    hex[40]=0;",
            "    return hex;",
            "}",
            "// MD5 hash (RFC 1321) → 32-char hex string.",
            "char* __xlang_md5_hex(const char* data) {",
            "    static const uint32_t T[64] = {",
            "        0xd76aa478,0xe8c7b756,0x242070db,0xc1bdceee,0xf57c0faf,0x4787c62a,0xa8304613,0xfd469501,",
            "        0x698098d8,0x8b44f7af,0xffff5bb1,0x895cd7be,0x6b901122,0xfd987193,0xa679438e,0x49b40821,",
            "        0xf61e2562,0xc040b340,0x265e5a51,0xe9b6c7aa,0xd62f105d,0x02441453,0xd8a1e681,0xe7d3fbc8,",
            "        0x21e1cde6,0xc33707d6,0xf4d50d87,0x455a14ed,0xa9e3e905,0xfcefa3f8,0x676f02d9,0x8d2a4c8a,",
            "        0xfffa3942,0x8771f681,0x6d9d6122,0xfde5380c,0xa4beea44,0x4bdecfa9,0xf6bb4b60,0xbebfbc70,",
            "        0x289b7ec6,0xeaa127fa,0xd4ef3085,0x04881d05,0xd9d4d039,0xe6db99e5,0x1fa27cf8,0xc4ac5665,",
            "        0xf4292244,0x432aff97,0xab9423a7,0xfc93a039,0x655b59c3,0x8f0ccc92,0xffeff47d,0x85845dd1,",
            "        0x6fa87e4f,0xfe2ce6e0,0xa3014314,0x4e0811a1,0xf7537e82,0xbd3af235,0x2ad7d2bb,0xeb86d391",
            "    };",
            "    static const int s[64] = {7,12,17,22,7,12,17,22,7,12,17,22,7,12,17,22,5,9,14,20,5,9,14,20,5,9,14,20,5,9,14,20,4,11,16,23,4,11,16,23,4,11,16,23,4,11,16,23,6,10,15,21,6,10,15,21,6,10,15,21,6,10,15,21};",
            "    uint32_t a0=0x67452301,b0=0xefcdab89,c0=0x98badcfe,d0=0x10325476;",
            "    size_t dlen=strlen(data);",
            "    size_t padded=((dlen+9+63)/64)*64;",
            "    uint8_t* msg=(uint8_t*)calloc(padded,1);",
            "    memcpy(msg,data,dlen);",
            "    msg[dlen]=0x80;",
            "    uint64_t bits=(uint64_t)dlen*8;",
            "    for(int i=0;i<8;i++) msg[padded-8+i]=(uint8_t)(bits>>(i*8));",
            "    for(size_t off=0;off<padded;off+=64){",
            "        uint32_t M[16];",
            "        for(int i=0;i<16;i++) M[i]=((uint32_t)msg[off+i*4])|((uint32_t)msg[off+i*4+1]<<8)|((uint32_t)msg[off+i*4+2]<<16)|((uint32_t)msg[off+i*4+3]<<24);",
            "        uint32_t A=a0,B=b0,C=c0,D=d0;",
            "        for(int i=0;i<64;i++){",
            "            uint32_t F; int g;",
            "            if(i<16){F=(B&C)|(~B&D);g=i;}",
            "            else if(i<32){F=(D&B)|(~D&C);g=(5*i+1)%16;}",
            "            else if(i<48){F=B^C^D;g=(3*i+5)%16;}",
            "            else{F=C^(B|~D);g=(7*i)%16;}",
            "            F=F+A+T[i]+M[g];",
            "            A=D;D=C;C=B;",
            "            B=B+((F<<s[i])|(F>>(32-s[i])));",
            "        }",
            "        a0+=A;b0+=B;c0+=C;d0+=D;",
            "    }",
            "    free(msg);",
            "    char* hex=(char*)malloc(33);",
            "    const char* hc=\"0123456789abcdef\";",
            "    uint32_t hh[4]={a0,b0,c0,d0};",
            "    for(int i=0;i<4;i++){for(int j=0;j<4;j++){hex[i*8+j*2]=(char)hc[(hh[i]>>(j*8+4))&15];hex[i*8+j*2+1]=(char)hc[(hh[i]>>(j*8))&15];}}",
            "    hex[32]=0;",
            "    return hex;",
            "}",
            "char* __xlang_read_stdin() {",
            "    size_t cap = 65536, len = 0;",
            "    char* buf = (char*)malloc(cap);",
            "    size_t r;",
            "    while ((r = fread(buf + len, 1, cap - len, stdin)) > 0) {",
            "        len += r;",
            "        if (len + 1 >= cap) { cap *= 2; buf = (char*)realloc(buf, cap); }",
            "    }",
            "    buf[len] = 0;",
            "    return buf;",
            "}",
            "char* __xlang_read_file(const char* path) {",
            "    FILE* f = fopen(path, \"rb\");",
            "    if (!f) { char* e = (char*)malloc(1); e[0] = 0; return e; }",
            "    size_t cap = 65536, len = 0;",
            "    char* buf = (char*)malloc(cap);",
            "    size_t r;",
            "    while ((r = fread(buf + len, 1, cap - len, f)) > 0) {",
            "        len += r;",
            "        if (len + 1 >= cap) { cap *= 2; buf = (char*)realloc(buf, cap); }",
            "    }",
            "    buf[len] = 0; fclose(f);",
            "    return buf;",
            "}",
            "void __xlang_write_file(const char* path, const char* content) {",
            "    FILE* f = fopen(path, \"wb\");",
            "    if (!f) return;",
            "    fwrite(content, 1, strlen(content), f); fclose(f);",
            "}",
            "int32_t __xlang_str_find(const char* s, const char* sub) {",
            "    const char* p = strstr(s, sub);",
            "    return p ? (int32_t)(p - s) : -1;",
            "}",
            "char* __xlang_str_slice(const char* s, int32_t start, int32_t end) {",
            "    if (start < 0) start = 0;",
            "    if (end < start) end = start;",
            "    int32_t len = end - start;",
            "    char* out = (char*)malloc((size_t)len + 1);",
            "    memcpy(out, s + start, (size_t)len); out[len] = 0;",
            "    return out;",
            "}",
            "char* __xlang_str_trim(const char* s) {",
            "    size_t n = strlen(s), a = 0, b = n;",
            "    while (a < b && (s[a] == ' ' || s[a] == '\\t' || s[a] == '\\n' || s[a] == '\\r')) a++;",
            "    while (b > a && (s[b-1] == ' ' || s[b-1] == '\\t' || s[b-1] == '\\n' || s[b-1] == '\\r')) b--;",
            "    size_t len = b - a;",
            "    char* out = (char*)malloc(len + 1);",
            "    memcpy(out, s + a, len); out[len] = 0;",
            "    return out;",
            "}",
            "int32_t __xlang_str_contains(const char* s, const char* sub) {",
            "    return strstr(s, sub) != NULL ? 1 : 0;",
            "}",
            "char* __xlang_float_to_str(double f) {",
            "    char* out = (char*)malloc(32);",
            "    snprintf(out, 32, \"%g\", f);",
            "    return out;",
            "}",
            "double __xlang_str_to_float(const char* s) {",
            "    return strtod(s, 0);",
            "}",
            "int32_t __xlang_str_starts_with(const char* s, const char* prefix) {",
            "    size_t pl = strlen(prefix);",
            "    return strncmp(s, prefix, pl) == 0 ? 1 : 0;",
            "}",
            "int32_t __xlang_str_ends_with(const char* s, const char* suffix) {",
            "    size_t sl = strlen(s), fl = strlen(suffix);",
            "    if (fl > sl) return 0;",
            "    return strcmp(s + sl - fl, suffix) == 0 ? 1 : 0;",
            "}",
            "char* __xlang_str_replace(const char* s, const char* from, const char* to) {",
            "    size_t sl=strlen(s), fl=strlen(from), tl=strlen(to);",
            "    if(fl==0){char*d=(char*)malloc(sl+1);strcpy(d,s);return d;}",
            "    size_t count=0;",
            "    const char* p=s;",
            "    while((p=strstr(p,from))){count++;p+=fl;}",
            "    size_t outlen=sl+count*(tl>fl?tl-fl:0)+1;",
            "    char* out=(char*)malloc(outlen);",
            "    char* o=out;",
            "    const char* cur=s;",
            "    const char* next;",
            "    while((next=strstr(cur,from))){",
            "        memcpy(o,cur,next-cur);o+=next-cur;",
            "        memcpy(o,to,tl);o+=tl;",
            "        cur=next+fl;",
            "    }",
            "    strcpy(o,cur);",
            "    return out;",
            "}",
            "char* __xlang_str_reverse(const char* s) {",
            "    int32_t n = (int32_t)strlen(s);",
            "    char* out = (char*)malloc(n + 1);",
            "    for (int32_t i = 0; i < n; i++) out[i] = s[n - 1 - i];",
            "    out[n] = 0;",
            "    return out;",
            "}",
            "char* __xlang_str_lower(const char* s) {",
            "    size_t n = strlen(s);",
            "    char* out = (char*)malloc(n + 1);",
            "    for (size_t i = 0; i < n; i++) {",
            "        char c = s[i];",
            "        if (c >= 'A' && c <= 'Z') c = (char)(c + 32);",
            "        out[i] = c;",
            "    }",
            "    out[n] = 0;",
            "    return out;",
            "}",
            "char* __xlang_str_upper(const char* s) {",
            "    size_t n = strlen(s);",
            "    char* out = (char*)malloc(n + 1);",
            "    for (size_t i = 0; i < n; i++) {",
            "        char c = s[i];",
            "        if (c >= 'a' && c <= 'z') c = (char)(c - 32);",
            "        out[i] = c;",
            "    }",
            "    out[n] = 0;",
            "    return out;",
            "}",
            "char* __xlang_str_repeat(const char* s, int32_t n) {",
            "    size_t sl = strlen(s);",
            "    if (n < 0) n = 0;",
            "    size_t total = sl * (size_t)n;",
            "    char* out = (char*)malloc(total + 1);",
            "    for (int32_t i = 0; i < n; i++) memcpy(out + (size_t)i * sl, s, sl);",
            "    out[total] = 0;",
            "    return out;",
            "}",
            "char* __xlang_chr(int32_t n) {",
            "    char* out = (char*)malloc(2);",
            "    out[0] = (char)n;",
            "    out[1] = 0;",
            "    return out;",
            "}",
            "// Find sub in s starting at byte offset `from`; absolute index or -1.",
            "int32_t __xlang_str_find_from(const char* s, const char* sub, int32_t from) {",
            "    if (from < 0) from = 0;",
            "    // O(1) terminal check (no strlen — strlen-per-call made loops over",
            "    // str_find_from O(n^2), regressing wc -l's count_lines to 6s/100k).",
            "    // Callers keep `from` <= strlen, so s[from] is a valid read.",
            "    if (s[from] == 0) return -1;",
            "    const char* p = strstr(s + from, sub);",
            "    return p ? (int32_t)(p - s) : -1;",
            "}",
            "// Replace the FIRST occurrence of `from` with `to` (str_replace does all).",
            "char* __xlang_str_replace_first(const char* s, const char* from, const char* to) {",
            "    const char* p = strstr(s, from);",
            "    size_t sl = strlen(s), fl = strlen(from), tl = strlen(to);",
            "    if (!p) { char* d = (char*)malloc(sl + 1); strcpy(d, s); return d; }",
            "    size_t pre = (size_t)(p - s);",
            "    char* out = (char*)malloc(sl + (tl > fl ? tl - fl : 0) + 1);",
            "    memcpy(out, s, pre);",
            "    memcpy(out + pre, to, tl);",
            "    strcpy(out + pre + tl, p + fl);",
            "    return out;",
            "}",
            "int32_t __xlang_abs(int32_t n) { return n < 0 ? -n : n; }",
            "int32_t __xlang_max(int32_t a, int32_t b) { return a > b ? a : b; }",
            "int32_t __xlang_min(int32_t a, int32_t b) { return a < b ? a : b; }",
            "char* __xlang_str_translate(const char* s, const char* from, const char* to) {",
            "    int32_t n = (int32_t)strlen(s);",
            "    int32_t tn = (int32_t)strlen(to);",
            "    // Build a 256-entry translation table once (first-occurrence wins,",
            "    // matching strchr), then apply per char — O(n), vs the old O(n*|from|)",
            "    // strchr-per-char which made `tr a-z A-Z` ~8x slower than GNU tr.",
            "    char table[256];",
            "    unsigned char mapped[256];",
            "    for (int i = 0; i < 256; i++) { table[i] = (char)i; mapped[i] = 0; }",
            "    for (int32_t i = 0; from[i] && i < tn; i++) {",
            "        unsigned char c = (unsigned char)from[i];",
            "        if (!mapped[c]) { table[c] = to[i]; mapped[c] = 1; }",
            "    }",
            "    char* out = (char*)malloc(n + 1);",
            "    for (int32_t i = 0; i < n; i++) out[i] = table[(unsigned char)s[i]];",
            "    out[n] = 0;",
            "    return out;",
            "}",
            "// Delete every char in `set` from `s` — O(n) bulk via a 256-byte presence",
            "// table (built in C), vs a per-char str_char_at loop in xlang. For tr -d.",
            "char* __xlang_str_delete(const char* s, const char* set) {",
            "    int32_t n = (int32_t)strlen(s);",
            "    unsigned char keep[256];",
            "    for (int i = 0; i < 256; i++) keep[i] = 1;",
            "    for (int i = 0; set[i]; i++) keep[(unsigned char)set[i]] = 0;",
            "    char* out = (char*)malloc(n + 1);",
            "    int32_t j = 0;",
            "    for (int32_t i = 0; i < n; i++) {",
            "        if (keep[(unsigned char)s[i]]) out[j++] = s[i];",
            "    }",
            "    out[j] = 0;",
            "    return out;",
            "}",
            "// cat -A/-E/-T style 'show' of a string: show_tabs=1 → tab as \"^I\",",
            "// show_ends=1 → '$' before each newline. Bulk O(n) in C, vs the per-char",
            "// xlang loop that made cate/showall 3-6x slower than GNU cat -A/-E on Linux.",
            "char* __xlang_cat_show(const char* s, int32_t show_tabs, int32_t show_ends) {",
            "    int32_t n = (int32_t)strlen(s);",
            "    char* out = (char*)malloc((size_t)n * 2 + 1);",
            "    int32_t j = 0;",
            "    for (int32_t i = 0; i < n; i++) {",
            "        unsigned char c = (unsigned char)s[i];",
            "        if (c == 9 && show_tabs) { out[j++] = '^'; out[j++] = 'I'; }",
            "        else if (c == 10) { if (show_ends) out[j++] = '$'; out[j++] = '\\n'; }",
            "        else { out[j++] = (char)c; }",
            "    }",
            "    out[j] = 0;",
            "    return out;",
            "}",
            "char* __xlang_read_line() {",
            "    char* buf = (char*)malloc(65536);",
            "    if (!fgets(buf, 65536, stdin)) { buf[0] = 0; return buf; }",
            "    int32_t n = (int32_t)strlen(buf);",
            "    if (n > 0 && buf[n - 1] == '\\n') buf[n - 1] = 0;",
            "    return buf;",
            "}",
            "static char* __sb_buf = 0;",
            "static size_t __sb_len = 0;",
            "static size_t __sb_cap = 0;",
            "void __xlang_sb_new() {",
            "    if (!__sb_buf) { __sb_buf = (char*)malloc(65536); __sb_cap = 65536; }",
            "    __sb_len = 0; __sb_buf[0] = 0;",
            "}",
            "void __xlang_sb_push(const char* s) {",
            "    size_t sl = strlen(s);",
            "    if (__sb_len + sl + 1 > __sb_cap) {",
            "        while (__sb_len + sl + 1 > __sb_cap) __sb_cap *= 2;",
            "        __sb_buf = (char*)realloc(__sb_buf, __sb_cap);",
            "    }",
            "    memcpy(__sb_buf + __sb_len, s, sl);",
            "    __sb_len += sl;",
            "    __sb_buf[__sb_len] = 0;",
            "}",
            "const char* __xlang_sb_str() {",
            "    return __sb_buf ? __sb_buf : \"\";",
            "}",
            "void __xlang_sb_push_char(int32_t c) {",
            "    if (__sb_len + 2 > __sb_cap) { __sb_cap *= 2; __sb_buf = (char*)realloc(__sb_buf, __sb_cap); }",
            "    __sb_buf[__sb_len++] = (char)c;",
            "    __sb_buf[__sb_len] = 0;",
            "}",
            "char* __xlang_time_str() {",
            "    setlocale(LC_TIME, \"\");",
            "    time_t t = time(NULL);",
            "    struct tm* tm = localtime(&t);",
            "    char* s = (char*)malloc(64);",
            "    strftime(s, 64, \"%a %b %e %H:%M:%S %Z %Y\", tm);",
            "    return s;",
            "}",
            "// Format the current LOCAL time under a caller-supplied strftime",
            "// format string (e.g. \"%Y-%m-%d %H:%M:%S\"). Owned, malloc'd buffer.",
            "char* __xlang_time_format(const char* fmt) {",
            "    setlocale(LC_TIME, \"\");",
            "    time_t t = time(NULL);",
            "    struct tm* tm = localtime(&t);",
            "    char* s = (char*)malloc(256);",
            "    strftime(s, 256, fmt, tm);",
            "    return s;",
            "}",
            "// Same as __xlang_time_format but in UTC (gmtime). Used by `date -u`.",
            "// gmtime (not gmtime_r) so this compiles on MSVC/Windows too — the",
            "// preamble is always emitted, unlike the Linux-only networking block.",
            "char* __xlang_time_format_utc(const char* fmt) {",
            "    time_t t = time(NULL);",
            "    struct tm* tm = gmtime(&t);",
            "    char* s = (char*)malloc(256);",
            "    strftime(s, 256, fmt, tm);",
            "    return s;",
            "}",
            "// Format an ARBITRARY time given as a Unix epoch (int32 seconds), in local",
            "// time. Used by `date -d @EPOCH` and `date -r FILE` (file mtime). The",
            "// epoch is int32 (matches __xlang_stat_field's mtime) — Y2038-bounded.",
            "char* __xlang_time_format_at(const char* fmt, int32_t epoch) {",
            "    setlocale(LC_TIME, \"\");",
            "    time_t t = (time_t)epoch;",
            "    struct tm* tm = localtime(&t);",
            "    char* s = (char*)malloc(256);",
            "    strftime(s, 256, fmt, tm);",
            "    return s;",
            "}",
            "// Format an arbitrary epoch in UTC (gmtime). Same int32 caveat.",
            "char* __xlang_time_format_at_utc(const char* fmt, int32_t epoch) {",
            "    time_t t = (time_t)epoch;",
            "    struct tm* tm = gmtime(&t);",
            "    char* s = (char*)malloc(256);",
            "    strftime(s, 256, fmt, tm);",
            "    return s;",
            "}",
            "// Monotonic seconds since an arbitrary epoch (CLOCK_MONOTONIC), as int32.",
            "// Overflow-safe for ~68 years. Used for elapsed-time measurement (e.g. the",
            "// load generator's per-worker duration timing) — NOT wall-clock time.",
            "int32_t __xlang_now_s() {",
            "    struct timespec ts;",
            "    clock_gettime(CLOCK_MONOTONIC, &ts);",
            "    return (int32_t)ts.tv_sec;",
            "}",
            "// Wall-clock Unix epoch seconds (time(NULL)), as int32. Unlike now_s",
            "// (monotonic), this is the real date/time — use for `date -d yesterday`",
            "// style relative-date arithmetic. Y2038-bounded (int32).",
            "int32_t __xlang_time_now() {",
            "    return (int32_t)time(NULL);",
            "}",
            "",
        ];
        for line in lines {
            self.emit(line);
        }
    }

    /// Networking helpers (socket I/O), guarded so non-Linux builds (which lack
    /// these POSIX headers) skip them entirely. Programs use networking only on
    /// Linux (CI / the target server); on Windows the block is preprocessed out,
    /// so the run-safe tests (which cc the generated C locally) still pass.
    fn emit_networking_preamble(&mut self) {
        let lines = [
            "#if !defined(_WIN32)",
            "#include <unistd.h>",
            "#include <sys/socket.h>",
            "#include <netinet/in.h>",
            "#include <arpa/inet.h>",
            "#include <netdb.h>",
            "#include <dirent.h>",
            "#include <sys/stat.h>",
            "#include <signal.h>",
            "#include <sys/utsname.h>",
            "#include <sys/epoll.h>",
            "#include <fcntl.h>",
            "#include <sys/sendfile.h>",
            "#include <netinet/tcp.h>",
            "#include <errno.h>",
            "#include <sched.h>",
            "#include <sys/wait.h>",
            "int32_t __xlang_tcp_listen(int32_t port) {",
            "    int fd = socket(AF_INET, SOCK_STREAM, 0);",
            "    int opt = 1;",
            "    setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, &opt, sizeof(opt));",
            "    struct sockaddr_in addr;",
            "    addr.sin_family = AF_INET;",
            "    addr.sin_addr.s_addr = INADDR_ANY;",
            "    addr.sin_port = htons((uint16_t)port);",
            "    bind(fd, (struct sockaddr*)&addr, sizeof(addr));",
            "    listen(fd, 64);",
            "    return (int32_t)fd;",
            "}",
            "// Like __xlang_tcp_listen but sets SO_REUSEPORT before bind, so a prefork",
            "// pool of workers can all bind the same port and the kernel load-balances",
            "// incoming connections across them (the nginx multi-worker model). Each",
            "// worker accept()s on its own inherited fd inside its own epoll loop.",
            "// SO_REUSEPORT must be set before bind(). Linux 3.8+.",
            "int32_t __xlang_tcp_listen_reuseport(int32_t port) {",
            "    int fd = socket(AF_INET, SOCK_STREAM, 0);",
            "    int opt = 1;",
            "    setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, &opt, sizeof(opt));",
            "    setsockopt(fd, SOL_SOCKET, SO_REUSEPORT, &opt, sizeof(opt));",
            "    struct sockaddr_in addr;",
            "    addr.sin_family = AF_INET;",
            "    addr.sin_addr.s_addr = INADDR_ANY;",
            "    addr.sin_port = htons((uint16_t)port);",
            "    bind(fd, (struct sockaddr*)&addr, sizeof(addr));",
            "    listen(fd, 64);",
            "    return (int32_t)fd;",
            "}",
            "// Connect a TCP client to <host>:<port>. Resolves hostnames (via",
            "// getaddrinfo) as well as dotted-quads, so reverse-proxy upstreams can",
            "// be named. Returns the connected fd, or -1 on failure.",
            "int32_t __xlang_tcp_connect(const char* host, int32_t port) {",
            "    struct addrinfo hints, *res, *rp;",
            "    memset(&hints, 0, sizeof(hints));",
            "    hints.ai_family = AF_INET;",
            "    hints.ai_socktype = SOCK_STREAM;",
            "    char portstr[16];",
            "    snprintf(portstr, sizeof(portstr), \"%d\", (int)port);",
            "    if (getaddrinfo(host, portstr, &hints, &res) != 0) return -1;",
            "    int fd = -1;",
            "    for (rp = res; rp != NULL; rp = rp->ai_next) {",
            "        fd = (int)socket(rp->ai_family, rp->ai_socktype, rp->ai_protocol);",
            "        if (fd < 0) continue;",
            "        if (connect(fd, rp->ai_addr, rp->ai_addrlen) == 0) break;",
            "        close(fd); fd = -1;",
            "    }",
            "    freeaddrinfo(res);",
            "    return (int32_t)fd;",
            "}",
            "char* __xlang_recv_str(int32_t fd) {",
            "    static char buf[65536];",
            "    ssize_t n = recv(fd, buf, 65535, 0);",
            "    if (n < 0) n = 0;",
            "    buf[n] = 0;",
            "    return buf;",
            "}",
            "// Drain ALL currently-buffered data on a non-blocking socket into one",
            "// owned, growable string (loops recv until it returns <= 0 — EAGAIN on a",
            "// non-blocking socket, or peer close). Lets a server read a full HTTP",
            "// request (headers + body) that exceeds recv_str's 64KB single-recv cap.",
            "// Safe ONLY on non-blocking sockets: a blocking socket would hang here",
            "// waiting for data that never arrives. Owned malloc'd buffer (unlike the",
            "// static recv_str buffer, no aliasing across calls).",
            "char* __xlang_recv_all(int32_t fd) {",
            "    size_t cap = 65536, len = 0;",
            "    char* buf = (char*)malloc(cap);",
            "    buf[0] = 0;",
            "    for (;;) {",
            "        if (len + 2 > cap) { cap *= 2; buf = (char*)realloc(buf, cap); }",
            "        ssize_t n = recv(fd, buf + len, cap - len - 1, 0);",
            "        if (n > 0) { len += (size_t)n; buf[len] = 0; continue; }",
            "        break;",
            "    }",
            "    return buf;",
            "}",
            "// Binary-safe byte I/O: a static receive buffer with EXPLICIT lengths,",
            "// so NUL bytes in the stream don't truncate (every string builtin above",
            "// is strlen-terminated and stops at NUL). Used by the reverse proxy to",
            "// relay binary response bodies (images, compressed, ...).",
            "static char __xlang_rbuf[65536];",
            "// recv up to 65535 bytes into __xlang_rbuf; NUL-terminate at the count so",
            "// a text view (rbuf_str + str_find) works on the header region (headers",
            "// are NUL-free; only body bytes past the first NUL are invisible). Returns",
            "// the byte count, or 0 on EOF/error.",
            "int32_t __xlang_recv_n(int32_t fd) {",
            "    ssize_t n = recv(fd, __xlang_rbuf, 65535, 0);",
            "    if (n < 0) n = 0;",
            "    __xlang_rbuf[n] = 0;",
            "    return (int32_t)n;",
            "}",
            "// Read up to 65535 bytes from ANY fd (stdin, file, pipe) into __xlang_rbuf;",
            "// NUL-terminate at the count; return the count (0 = EOF). Binary-safe —",
            "// the recv_n analogue for regular fds (recv_n uses recv(), sockets only).",
            "int32_t __xlang_read_rbuf(int32_t fd) {",
            "    ssize_t n = read(fd, __xlang_rbuf, 65535);",
            "    if (n < 0) n = 0;",
            "    __xlang_rbuf[n] = 0;",
            "    return (int32_t)n;",
            "}",
            "// View __xlang_rbuf as a (NUL-terminated) C string — valid for the header",
            "// region only; body bytes after the first NUL are invisible to str_* builtins.",
            "const char* __xlang_rbuf_str() { return __xlang_rbuf; }",
            "// Send exactly len bytes from __xlang_rbuf (loops on partial writes for",
            "// blocking sockets; binary-safe — NULs are ignored). Returns bytes sent.",
            "int32_t __xlang_send_rbuf(int32_t fd, int32_t len) {",
            "    size_t off = 0;",
            "    size_t remaining = (size_t)len;",
            "    while (remaining > 0) {",
            "        ssize_t s = send(fd, __xlang_rbuf + off, remaining, 0);",
            "        if (s > 0) { off += (size_t)s; remaining -= (size_t)s; continue; }",
            "        if (s < 0 && (errno == EAGAIN || errno == EWOULDBLOCK)) { sched_yield(); continue; }",
            "        break;",
            "    }",
            "    return (int32_t)((size_t)len - remaining);",
            "}",
            "// Write len bytes from __xlang_rbuf+offset to ANY fd (file, stdout, ...)",
            "// via write(2) — binary-safe (NULs ignored), unlike send_rbuf which uses",
            "// send() (sockets only). Loops past partial writes. Used by httpget to",
            "// save binary response bodies to a file or stdout.",
            "int32_t __xlang_write_rbuf(int32_t fd, int32_t offset, int32_t len) {",
            "    size_t off = (size_t)offset;",
            "    size_t remaining = (size_t)len;",
            "    while (remaining > 0) {",
            "        ssize_t s = write(fd, __xlang_rbuf + off, remaining);",
            "        if (s > 0) { off += (size_t)s; remaining -= (size_t)s; continue; }",
            "        if (s < 0 && errno == EINTR) { continue; }",
            "        break;",
            "    }",
            "    return (int32_t)((size_t)len - remaining);",
            "}",
            "// epoll event-loop support. A single global epoll fd + a ready-fd",
            "// ring buffer, so xlang treats epoll_wait(timeout) as \"next ready fd\".",
            "#define __XLANG_EPQ_CAP 8192",
            "static int32_t __xlang_epfd_g = -1;",
            "static int __xlang_epq_fd[__XLANG_EPQ_CAP];",
            "static int __xlang_epq_head = 0;",
            "static int __xlang_epq_tail = 0;",
            "int32_t __xlang_epoll_create() {",
            "    __xlang_epfd_g = epoll_create1(0);",
            "    return __xlang_epfd_g;",
            "}",
            "int32_t __xlang_epoll_add(int32_t fd) {",
            "    struct epoll_event ev;",
            "    ev.events = EPOLLIN;",
            "    ev.data.fd = fd;",
            "    return epoll_ctl(__xlang_epfd_g, EPOLL_CTL_ADD, fd, &ev) == 0 ? 0 : -1;",
            "}",
            "int32_t __xlang_epoll_del(int32_t fd) {",
            "    epoll_ctl(__xlang_epfd_g, EPOLL_CTL_DEL, fd, 0);",
            "    return 0;",
            "}",
            "int32_t __xlang_epoll_wait(int32_t timeout) {",
            "    if (__xlang_epq_head != __xlang_epq_tail) {",
            "        int fd = __xlang_epq_fd[__xlang_epq_head];",
            "        __xlang_epq_head = (__xlang_epq_head + 1) % __XLANG_EPQ_CAP;",
            "        return (int32_t)fd;",
            "    }",
            "    struct epoll_event events[256];",
            "    int n = epoll_wait(__xlang_epfd_g, events, 256, timeout);",
            "    if (n <= 0) return -1;",
            "    int i;",
            "    for (i = 0; i < n; i++) {",
            "        __xlang_epq_fd[__xlang_epq_tail] = events[i].data.fd;",
            "        __xlang_epq_tail = (__xlang_epq_tail + 1) % __XLANG_EPQ_CAP;",
            "    }",
            "    int fd = __xlang_epq_fd[__xlang_epq_head];",
            "    __xlang_epq_head = (__xlang_epq_head + 1) % __XLANG_EPQ_CAP;",
            "    return (int32_t)fd;",
            "}",
            "int32_t __xlang_set_nonblock(int32_t fd) {",
            "    int flags = fcntl(fd, F_GETFL, 0);",
            "    return fcntl(fd, F_SETFL, flags | O_NONBLOCK) == 0 ? 0 : -1;",
            "}",
            "int32_t __xlang_set_nodelay(int32_t fd) {",
            "    int flag = 1;",
            "    return setsockopt(fd, IPPROTO_TCP, TCP_NODELAY, &flag, sizeof(flag)) == 0 ? 0 : -1;",
            "}",
            "int32_t __xlang_open_read(const char* path) {",
            "    return (int32_t)open(path, O_RDONLY);",
            "}",
            "int32_t __xlang_open_write(const char* path) {",
            "    return (int32_t)open(path, O_WRONLY | O_CREAT | O_TRUNC, 0644);",
            "}",
            "int32_t __xlang_open_append(const char* path) {",
            "    return (int32_t)open(path, O_WRONLY | O_CREAT | O_APPEND, 0644);",
            "}",
            "// lseek(fd, offset, SEEK_SET) — set the file offset. Returns the new",
            "// offset, or -1. Used by dd for skip/seek (random file access).",
            "int32_t __xlang_seek(int32_t fd, int32_t offset) {",
            "    return (int32_t)lseek(fd, (off_t)offset, SEEK_SET);",
            "}",
            "// Process control for the shell: pipe(2) ends in globals (one pipeline",
            "// at a time — the shell waits on each line before reading the next).",
            "static int32_t __xlang_pipe_r = -1;",
            "static int32_t __xlang_pipe_w = -1;",
            "int32_t __xlang_make_pipe() {",
            "    int p[2];",
            "    if (pipe(p) != 0) return -1;",
            "    __xlang_pipe_r = p[0];",
            "    __xlang_pipe_w = p[1];",
            "    return 0;",
            "}",
            "int32_t __xlang_pipe_read_end() { return __xlang_pipe_r; }",
            "// Indexed pipe pool: supports N-stage pipelines (up to 17 stages).",
            "#define __XLANG_PIPE_POOL 16",
            "static int32_t __xlang_pr[__XLANG_PIPE_POOL];",
            "static int32_t __xlang_pw[__XLANG_PIPE_POOL];",
            "int32_t __xlang_make_pipe_at(int32_t idx) {",
            "    int p[2];",
            "    if (idx < 0 || idx >= __XLANG_PIPE_POOL) return -1;",
            "    if (pipe(p) != 0) return -1;",
            "    __xlang_pr[idx] = p[0];",
            "    __xlang_pw[idx] = p[1];",
            "    return 0;",
            "}",
            "int32_t __xlang_pipe_r_at(int32_t idx) {",
            "    return (idx >= 0 && idx < __XLANG_PIPE_POOL) ? __xlang_pr[idx] : -1;",
            "}",
            "int32_t __xlang_pipe_w_at(int32_t idx) {",
            "    return (idx >= 0 && idx < __XLANG_PIPE_POOL) ? __xlang_pw[idx] : -1;",
            "}",
            "int32_t __xlang_pipe_write_end() { return __xlang_pipe_w; }",
            "int32_t __xlang_dup2(int32_t oldfd, int32_t newfd) {",
            "    return dup2(oldfd, newfd) < 0 ? -1 : 0;",
            "}",
            "int32_t __xlang_exec_sh(const char* cmd) {",
            "    execl(\"/bin/sh\", \"sh\", \"-c\", cmd, (char*)NULL);",
            "    return -1;",
            "}",
            "// Tokenize cmd by whitespace and execvp(argv[0], argv) — PATH-based, so a",
            "// shell with PATH=xlang-bin runs ONLY xlang coreutils (a pure xlang",
            "// userland). Returns -1 only if exec fails (child should then exit).",
            "int32_t __xlang_exec_split(const char* cmd) {",
            "    char buf[4096];",
            "    strncpy(buf, cmd, 4095); buf[4095] = 0;",
            "    char* argv[128];",
            "    int ac = 0;",
            "    char* p = buf;",
            "    while (*p) {",
            "        while (*p == ' ' || *p == '\\t') p++;",
            "        if (!*p) break;",
            "        if (ac >= 127) break;",
            "        argv[ac++] = p;",
            "        while (*p && *p != ' ' && *p != '\\t') p++;",
            "        if (*p) { *p = 0; p++; }",
            "    }",
            "    argv[ac] = (char*)NULL;",
            "    if (ac == 0) return -1;",
            "    execvp(argv[0], argv);",
            "    return -1;",
            "}",
            "int32_t __xlang_wait_child() {",
            "    int st = 0;",
            "    pid_t p = wait(&st);",
            "    return (int32_t)p;",
            "}",
            "int32_t __xlang_wait_status() {",
            "    int st = 0;",
            "    wait(&st);",
            "    if (WIFEXITED(st)) return WEXITSTATUS(st);",
            "    return 1;",
            "}",
            "// waitpid for a SPECIFIC child, return its exit status. If the child was",
            "// killed by a signal, return 128+signo (shell convention, e.g. SIGTERM",
            "// =15 -> 143). Used by timeout to wait for the command child.",
            "int32_t __xlang_wait_pid_status(int32_t pid) {",
            "    int st = 0;",
            "    waitpid((pid_t)pid, &st, 0);",
            "    if (WIFEXITED(st)) return WEXITSTATUS(st);",
            "    if (WIFSIGNALED(st)) return 128 + WTERMSIG(st);",
            "    return 1;",
            "}",
            "char* __xlang_read_fd(int32_t fd) {",
            "    size_t cap = 65536, len = 0;",
            "    char* buf = (char*)malloc(cap);",
            "    ssize_t r;",
            "    while ((r = read(fd, buf + len, cap - len)) > 0) {",
            "        len += (size_t)r;",
            "        if (len + 1 >= cap) { cap *= 2; buf = (char*)realloc(buf, cap); }",
            "    }",
            "    buf[len] = 0;",
            "    return buf;",
            "}",
            "int32_t __xlang_setenv(const char* name, const char* value) {",
            "    return setenv(name, value, 1) == 0 ? 0 : -1;",
            "}",
            "// File fd cache: hot files keep their fd open + size known, so a request",
            "// skips open/fstat/close (what nginx does). Simple linear map, cap 512.",
            "#define __XLANG_FC_N 512",
            "static char* __xlang_fc_path[__XLANG_FC_N];",
            "static int __xlang_fc_fd[__XLANG_FC_N];",
            "static int32_t __xlang_fc_size[__XLANG_FC_N];",
            "static int __xlang_fc_len = 0;",
            "int32_t __xlang_cache_open(const char* path) {",
            "    int i;",
            "    for (i = 0; i < __xlang_fc_len; i++) {",
            "        if (strcmp(__xlang_fc_path[i], path) == 0) return (int32_t)__xlang_fc_fd[i];",
            "    }",
            "    if (__xlang_fc_len >= __XLANG_FC_N) return -1;",
            "    int fd = open(path, O_RDONLY);",
            "    if (fd < 0) return -1;",
            "    struct stat st;",
            "    if (fstat(fd, &st) != 0) { close(fd); return -1; }",
            "    __xlang_fc_path[__xlang_fc_len] = strdup(path);",
            "    __xlang_fc_fd[__xlang_fc_len] = fd;",
            "    __xlang_fc_size[__xlang_fc_len] = (int32_t)st.st_size;",
            "    __xlang_fc_len++;",
            "    return (int32_t)fd;",
            "}",
            "int32_t __xlang_cache_size(const char* path) {",
            "    int i;",
            "    for (i = 0; i < __xlang_fc_len; i++) {",
            "        if (strcmp(__xlang_fc_path[i], path) == 0) return __xlang_fc_size[i];",
            "    }",
            "    return -1;",
            "}",
            "int32_t __xlang_sendfile_fd(int32_t out_fd, int32_t in_fd, int32_t len) {",
            "    off_t off = 0;",
            "    size_t remaining = (size_t)len;",
            "    while (remaining > 0) {",
            "        ssize_t s = sendfile(out_fd, in_fd, &off, remaining);",
            "        if (s > 0) { remaining -= (size_t)s; continue; }",
            "        // non-blocking socket buffer full: retry when writable. This keeps",
            "        // the send complete (no truncation) on non-blocking sockets while",
            "        // staying out of the way for small bodies that never hit EAGAIN.",
            "        if (s < 0 && (errno == EAGAIN || errno == EWOULDBLOCK)) { sched_yield(); continue; }",
            "        break;",
            "    }",
            "    return (int32_t)((size_t)len - remaining);",
            "}",
            "// Like sendfile_fd but starts at a given byte offset — used for HTTP 206",
            "// Partial Content / Range requests. `off` is the starting offset; the",
            "// kernel sendfile(2) advances its own offset pointer from there.",
            "int32_t __xlang_sendfile_range(int32_t out_fd, int32_t in_fd, int32_t offset, int32_t len) {",
            "    off_t off = (off_t)offset;",
            "    size_t remaining = (size_t)len;",
            "    while (remaining > 0) {",
            "        ssize_t s = sendfile(out_fd, in_fd, &off, remaining);",
            "        if (s > 0) { remaining -= (size_t)s; continue; }",
            "        if (s < 0 && (errno == EAGAIN || errno == EWOULDBLOCK)) { sched_yield(); continue; }",
            "        break;",
            "    }",
            "    return (int32_t)((size_t)len - remaining);",
            "}",
            "int32_t __xlang_dir_count(const char* path) {",
            "    DIR* d = opendir(path);",
            "    if (!d) return 0;",
            "    int32_t n = 0;",
            "    while (readdir(d)) n++;",
            "    closedir(d);",
            "    return n;",
            "}",
            "char* __xlang_dir_entry(const char* path, int32_t idx) {",
            "    DIR* d = opendir(path);",
            "    if (!d) return \"\";",
            "    struct dirent* e;",
            "    int32_t i = 0;",
            "    while ((e = readdir(d))) {",
            "        if (i == idx) {",
            "            char* copy = (char*)malloc(strlen(e->d_name) + 1);",
            "            strcpy(copy, e->d_name);",
            "            closedir(d);",
            "            return copy;",
            "        }",
            "        i++;",
            "    }",
            "    closedir(d);",
            "    return \"\";",
            "}",
            "int32_t __xlang_is_dir(const char* path) {",
            "    struct stat st;",
            "    if (stat(path, &st) != 0) return 0;",
            "    return S_ISDIR(st.st_mode) ? 1 : 0;",
            "}",
            "int32_t __xlang_file_size(const char* path) {",
            "    struct stat st;",
            "    if (stat(path, &st) != 0) return 0;",
            "    return (int32_t)st.st_size;",
            "}",
            "int32_t __xlang_file_exists(const char* path) {",
            "    struct stat st;",
            "    return stat(path, &st) == 0 ? 1 : 0;",
            "}",
            "// One stat(2) field by selector (so ls -l needs only one builtin):",
            "// 0=mode, 1=nlink, 2=uid, 3=gid, 4=size, 5=mtime. -1 on error.",
            "int32_t __xlang_stat_field(const char* path, int32_t field) {",
            "    struct stat st;",
            "    if (stat(path, &st) != 0) return -1;",
            "    switch (field) {",
            "        case 0: return (int32_t)st.st_mode;",
            "        case 1: return (int32_t)st.st_nlink;",
            "        case 2: return (int32_t)st.st_uid;",
            "        case 3: return (int32_t)st.st_gid;",
            "        case 4: return (int32_t)st.st_size;",
            "        case 5: return (int32_t)st.st_mtime;",
            "    }",
            "    return -1;",
            "}",
            "// ctime(t) without the trailing newline (for ls -l dates).",
            "char* __xlang_fmt_ctime(int32_t t) {",
            "    time_t tt = (time_t)t;",
            "    char* s = ctime(&tt);",
            "    if (!s) { char* e = (char*)malloc(1); e[0] = 0; return e; }",
            "    char* out = strdup(s);",
            "    int n = (int)strlen(out);",
            "    if (n > 0 && out[n-1] == '\\n') out[n-1] = 0;",
            "    return out;",
            "}",
            "// HTTP-date (RFC 7231): Wed, 03 Jul 2024 12:00:00 GMT",
            "char* __xlang_fmt_http_date(int32_t t) {",
            "    time_t tt = (time_t)t;",
            "    struct tm tm;",
            "    gmtime_r(&tt, &tm);",
            "    char* buf = (char*)malloc(32);",
            "    strftime(buf, 32, \"%a, %d %b %Y %H:%M:%S GMT\", &tm);",
            "    return buf;",
            "}",
            "char* __xlang_getcwd() {",
            "    char* buf = (char*)malloc(4096);",
            "    return getcwd(buf, 4096);",
            "}",
            "char* __xlang_readlink(const char* path) {",
            "    char* buf = (char*)malloc(4096);",
            "    ssize_t n = readlink(path, buf, 4095);",
            "    if (n < 0) { buf[0] = 0; return buf; }",
            "    buf[n] = 0;",
            "    return buf;",
            "}",
            "char* __xlang_realpath(const char* path) {",
            "    char* resolved = realpath(path, NULL);",
            "    return resolved ? resolved : \"\";",
            "}",
            "extern char** environ;",
            "int32_t __xlang_env_count() {",
            "    int32_t n = 0;",
            "    while (environ[n]) n++;",
            "    return n;",
            "}",
            "const char* __xlang_env_entry(int32_t idx) {",
            "    extern char** environ;",
            "    int32_t n = 0;",
            "    while (environ[n]) {",
            "        if (n == idx) return environ[n];",
            "        n++;",
            "    }",
            "    return \"\";",
            "}",
            "const char* __xlang_tty() {",
            "    char* name = ttyname(0);",
            "    return name ? name : \"\";",
            "}",
            "const char* __xlang_uname_machine() {",
            "    struct utsname u;",
            "    if (uname(&u) != 0) return \"\";",
            "    char* m = (char*)malloc(strlen(u.machine) + 1);",
            "    strcpy(m, u.machine);",
            "    return m;",
            "}",
            "// TLS (HTTPS) via OpenSSL. Gated on __XLANG_TLS__ (defined only when a",
            "// tls_* builtin is used) so non-TLS programs don't pull in OpenSSL.",
            "// 64-bit SSL_CTX*/SSL* are hidden behind i32 table indices.",
            "#ifdef __XLANG_TLS__",
            "#include <openssl/ssl.h>",
            "#include <openssl/err.h>",
            "static SSL_CTX* __xlang_g_ctx[16];",
            "static SSL* __xlang_g_ssl[256];",
            "// Load cert+key into a new SSL_CTX; return a table index (or -1).",
            "int32_t __xlang_tls_ctx_new(const char* cert, const char* key) {",
            "    OPENSSL_init_ssl(OPENSSL_INIT_LOAD_SSL_STRINGS, NULL);",
            "    SSL_CTX* ctx = SSL_CTX_new(TLS_server_method());",
            "    if (!ctx) return -1;",
            "    if (SSL_CTX_use_certificate_file(ctx, cert, SSL_FILETYPE_PEM) <= 0 ||",
            "        SSL_CTX_use_PrivateKey_file(ctx, key, SSL_FILETYPE_PEM) <= 0) {",
            "        SSL_CTX_free(ctx); return -1;",
            "    }",
            "    for (int i = 0; i < 16; i++) if (!__xlang_g_ctx[i]) { __xlang_g_ctx[i] = ctx; return i; }",
            "    SSL_CTX_free(ctx); return -1;",
            "}",
            "// TLS-accept on fd (blocking handshake); return an SSL table index (or -1).",
            "int32_t __xlang_tls_accept(int32_t ci, int32_t fd) {",
            "    SSL* ssl = SSL_new(__xlang_g_ctx[ci]);",
            "    if (!ssl) return -1;",
            "    SSL_set_fd(ssl, fd);",
            "    if (SSL_accept(ssl) <= 0) { SSL_free(ssl); return -1; }",
            "    for (int i = 0; i < 256; i++) if (!__xlang_g_ssl[i]) { __xlang_g_ssl[i] = ssl; return i; }",
            "    SSL_free(ssl); return -1;",
            "}",
            "// Read up to 64KB from a TLS connection into an owned, NUL-terminated buffer.",
            "char* __xlang_tls_read(int32_t si) {",
            "    char* buf = (char*)malloc(65536);",
            "    int n = SSL_read(__xlang_g_ssl[si], buf, 65535);",
            "    if (n < 0) n = 0;",
            "    buf[n] = 0;",
            "    return buf;",
            "}",
            "// Write a string to a TLS connection.",
            "int32_t __xlang_tls_write(int32_t si, const char* s) {",
            "    return (int32_t)SSL_write(__xlang_g_ssl[si], s, strlen(s));",
            "}",
            "// Shut down + free a TLS connection and release its table slot.",
            "int32_t __xlang_tls_close(int32_t si) {",
            "    if (__xlang_g_ssl[si]) {",
            "        SSL_shutdown(__xlang_g_ssl[si]);",
            "        SSL_free(__xlang_g_ssl[si]);",
            "        __xlang_g_ssl[si] = NULL;",
            "    }",
            "    return 0;",
            "}",
            "#endif",
            "#endif",
            "",
        ];
        for line in lines {
            self.emit(line);
        }
    }

    /// Lower the string builtins `str_len` / `str_concat` / `int_to_str`
    /// (strlen inline; the other two call the runtime-preamble helpers).
    fn try_string_call(
        &self,
        callee: &Spanned<Expr>,
        args: &[Spanned<Expr>],
    ) -> XResult<Option<String>> {
        let Expr::Identifier { name } = &callee.node else {
            return Ok(None);
        };
        let Some(first) = args.first() else {
            return Ok(None);
        };
        let a = self.gen_expr(first)?;
        let rendered = match name.as_str() {
            "str_len" => format!("(int32_t)strlen({a})"),
            "argv" => format!("__xlang_argv_g[{a}]"),
            "print_raw" => format!("printf(\"%s\", {a})"),
            // assert(cond) → runtime check + exit on failure. The condition is
            // evaluated as a C truthy int. Returns 0 so the call can sit in any
            // expression position (typically a statement).
            "assert" => format!("(__xlang_assert(({a}), \"assertion failed\"), 0)"),
            // panic(msg) → print + exit (never returns at runtime).
            "panic" => format!("(__xlang_panic({a}), 0)"),
            "int_to_str" => format!("__xlang_int_to_str({a})"),
            "float_to_str" => format!("__xlang_float_to_str({a})"),
            "int_to_f64" => format!("(double)({a})"),
            // int_to_i64 → widen an i32 to i64 (a C cast). Previously this was
            // in the typecheck builtin table but NOT lowered here, so it emitted
            // an undefined `int_to_i64(...)` call and failed to link.
            "int_to_i64" => format!("(int64_t)({a})"),
            "sha256_hex" => format!("__xlang_sha256_hex({a})"),
            "md5_hex" => format!("__xlang_md5_hex({a})"),
            "sha1_hex" => format!("__xlang_sha1_hex({a})"),
            "sha512_hex" => format!("__xlang_sha512_hex({a})"),
            "sha224_hex" => format!("__xlang_sha224_hex({a})"),
            "sha384_hex" => format!("__xlang_sha384_hex({a})"),
            "pad_int" => {
                let Some(second) = args.get(1) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                format!("__xlang_pad_int({a}, {b})")
            }
            "pad_zero" => {
                let Some(second) = args.get(1) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                format!("__xlang_pad_zero({a}, {b})")
            }
            "max" => {
                let Some(second) = args.get(1) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                format!("__xlang_max({a}, {b})")
            }
            "min" => {
                let Some(second) = args.get(1) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                format!("__xlang_min({a}, {b})")
            }
            "str_concat" => {
                let Some(second) = args.get(1) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                format!("__xlang_str_concat({a}, {b})")
            }
            "str_eq" => {
                let Some(second) = args.get(1) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                format!("(strcmp({a}, {b}) == 0)")
            }
            "str_find" => {
                let Some(second) = args.get(1) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                format!("__xlang_str_find({a}, {b})")
            }
            "str_slice" => {
                if args.len() < 3 {
                    return Ok(None);
                }
                let b = self.gen_expr(&args[1])?;
                let c = self.gen_expr(&args[2])?;
                format!("__xlang_str_slice({a}, {b}, {c})")
            }
            "str_reverse" => format!("__xlang_str_reverse({a})"),
            "str_lower" => format!("__xlang_str_lower({a})"),
            "str_upper" => format!("__xlang_str_upper({a})"),
            "str_repeat" => {
                let Some(second) = args.get(1) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                format!("__xlang_str_repeat({a}, {b})")
            }
            "time_format_at" => {
                let Some(second) = args.get(1) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                format!("__xlang_time_format_at({a}, {b})")
            }
            "time_format_at_utc" => {
                let Some(second) = args.get(1) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                format!("__xlang_time_format_at_utc({a}, {b})")
            }
            // TLS (OpenSSL) builtins. Each sets uses_tls so the gated TLS
            // preamble (#ifdef __XLANG_TLS__) + #define are emitted. Returns
            // are tracked via the typecheck builtin map.
            "tls_ctx_new" => {
                self.uses_tls.set(true);
                let Some(second) = args.get(1) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                format!("__xlang_tls_ctx_new({a}, {b})")
            }
            "tls_accept" => {
                self.uses_tls.set(true);
                let Some(second) = args.get(1) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                format!("__xlang_tls_accept({a}, {b})")
            }
            "tls_write" => {
                self.uses_tls.set(true);
                let Some(second) = args.get(1) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                format!("__xlang_tls_write({a}, {b})")
            }
            "tls_read" => {
                self.uses_tls.set(true);
                format!("__xlang_tls_read({a})")
            }
            "tls_close" => {
                self.uses_tls.set(true);
                format!("__xlang_tls_close({a})")
            }
            "chr" => format!("__xlang_chr({a})"),
            "abs" => format!("__xlang_abs({a})"),
            "str_trim" => format!("__xlang_str_trim({a})"),
            "time_format" => format!("__xlang_time_format({a})"),
            "time_format_utc" => format!("__xlang_time_format_utc({a})"),
            "str_split" => {
                let Some(second) = args.get(1) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                format!("__xlang_str_split({a}, {b}[0])")
            }
            "str_contains" => {
                let Some(second) = args.get(1) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                format!("__xlang_str_contains({a}, {b})")
            }
            "str_starts_with" => {
                let Some(second) = args.get(1) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                format!("__xlang_str_starts_with({a}, {b})")
            }
            "str_ends_with" => {
                let Some(second) = args.get(1) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                format!("__xlang_str_ends_with({a}, {b})")
            }
            "str_replace" => {
                let (Some(second), Some(third)) = (args.get(1), args.get(2)) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                let c = self.gen_expr(third)?;
                format!("__xlang_str_replace({a}, {b}, {c})")
            }
            "str_replace_first" => {
                let (Some(second), Some(third)) = (args.get(1), args.get(2)) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                let c = self.gen_expr(third)?;
                format!("__xlang_str_replace_first({a}, {b}, {c})")
            }
            "str_find_from" => {
                if args.len() < 3 {
                    return Ok(None);
                }
                let b = self.gen_expr(&args[1])?;
                let c = self.gen_expr(&args[2])?;
                format!("__xlang_str_find_from({a}, {b}, {c})")
            }
            "str_translate" => {
                let (Some(second), Some(third)) = (args.get(1), args.get(2)) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                let c = self.gen_expr(third)?;
                format!("__xlang_str_translate({a}, {b}, {c})")
            }
            "str_delete" => {
                let Some(second) = args.get(1) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                format!("__xlang_str_delete({a}, {b})")
            }
            "cat_show" => {
                let (Some(second), Some(third)) = (args.get(1), args.get(2)) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                let c = self.gen_expr(third)?;
                format!("__xlang_cat_show({a}, {b}, {c})")
            }
            "str_char_at" => {
                let Some(second) = args.get(1) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                format!("(int32_t)(unsigned char)({a}[{b}])")
            }
            "rbuf_byte_at" => format!("((int32_t)(unsigned char)__xlang_rbuf[{a}])"),
            "dir_count" => format!("__xlang_dir_count({a})"),
            "is_dir" => format!("__xlang_is_dir({a})"),
            "file_size" => format!("__xlang_file_size({a})"),
            "file_exists" => format!("__xlang_file_exists({a})"),
            "fmt_ctime" => format!("__xlang_fmt_ctime({a})"),
            "fmt_http_date" => format!("__xlang_fmt_http_date({a})"),
            "stat_field" => {
                let Some(second) = args.get(1) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                format!("__xlang_stat_field({a}, {b})")
            }
            "chdir" => format!("chdir(({a}))"),
            "make_dir" => format!("mkdir({a}, 0755)"),
            "make_fifo" => format!("mkfifo({a}, 0644)"),
            "chroot_call" => format!("chroot({a})"),
            "mknod_dev" => {
                let Some(second) = args.get(1) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                let Some(third) = args.get(2) else {
                    return Ok(None);
                };
                let c = self.gen_expr(third)?;
                let Some(fourth) = args.get(3) else {
                    return Ok(None);
                };
                let d = self.gen_expr(fourth)?;
                format!("mknod({a}, {b}, ((({c}) << 8) | (({d}) & 0xff)))")
            }
            "flush_stdout" => "fflush(stdout)".to_string(),
            "chown_file" => {
                let Some(second) = args.get(1) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                let Some(third) = args.get(2) else {
                    return Ok(None);
                };
                let c = self.gen_expr(third)?;
                format!("chown({a}, {b}, {c})")
            }
            "chgrp_file" => {
                let Some(second) = args.get(1) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                format!("chown({a}, -1, {b})")
            }
            "kill" => {
                let Some(second) = args.get(1) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                format!("kill(({a}), ({b}))")
            }
            "random_int" => format!("(int32_t)(rand() % ({a}))"),
            "sb_push" => format!("__xlang_sb_push({a})"),
            "sb_push_char" => format!("__xlang_sb_push_char({a})"),
            "getenv" => format!("(getenv({a}) ? getenv({a}) : \"\")"),
            "setenv" => {
                let Some(second) = args.get(1) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                format!("__xlang_setenv({a}, {b})")
            }
            "readlink" => format!("__xlang_readlink({a})"),
            "realpath" => format!("__xlang_realpath({a})"),
            "env_entry" => format!("__xlang_env_entry({a})"),
            "link_file" => {
                let Some(second) = args.get(1) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                format!("link(({a}), ({b}))")
            }
            "truncate_file" => {
                let Some(second) = args.get(1) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                format!("truncate(({a}), ({b}))")
            }
            "mkfifo" => format!("mkfifo(({a}), 0644)"),
            "wait_pid_status" => format!("__xlang_wait_pid_status({a})"),
            "rmdir" => format!("rmdir(({a}))"),
            "str_to_int_oct" => format!("(int32_t)strtol({a}, 0, 8)"),
            "chmod" => {
                let Some(second) = args.get(1) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                format!("chmod(({a}), ({b}))")
            }
            "symlink" => {
                let Some(second) = args.get(1) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                format!("symlink(({a}), ({b}))")
            }
            "dir_entry" => {
                let Some(second) = args.get(1) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                format!("__xlang_dir_entry({a}, {b})")
            }
            "str_cmp" => {
                let Some(second) = args.get(1) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                format!("(int32_t)strcmp({a}, {b})")
            }
            "vec_len" => format!("((int32_t)({a}).len)"),
            "str_to_int" => format!("(int32_t)strtol({a}, 0, 10)"),
            "str_to_float" => format!("strtod({a}, 0)"),
            "remove_file" => format!("remove({a})"),
            "system" => format!("system({a})"),
            "sleep_sec" => format!("(unsigned)sleep(({a}))"),
            "rename_file" => {
                let Some(second) = args.get(1) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                format!("rename({a}, {b})")
            }
            "read_file" => format!("__xlang_read_file({a})"),
            "write_file" => {
                let Some(second) = args.get(1) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                format!("__xlang_write_file({a}, {b})")
            }
            "tcp_listen" => format!("__xlang_tcp_listen({a})"),
            "tcp_listen_reuseport" => format!("__xlang_tcp_listen_reuseport({a})"),
            "tcp_connect" => {
                let Some(second) = args.get(1) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                format!("__xlang_tcp_connect({a}, {b})")
            }
            "accept" => format!("accept({a}, 0, 0)"),
            "recv_str" => format!("__xlang_recv_str({a})"),
            "recv_all" => format!("__xlang_recv_all({a})"),
            "recv_n" => format!("__xlang_recv_n({a})"),
            "read_rbuf" => format!("__xlang_read_rbuf({a})"),
            "close_fd" => format!("close({a})"),
            "shutdown_wr" => format!("shutdown({a}, SHUT_WR)"),
            "epoll_add" => format!("__xlang_epoll_add({a})"),
            "epoll_del" => format!("__xlang_epoll_del({a})"),
            "epoll_wait" => format!("__xlang_epoll_wait({a})"),
            "set_nonblock" => format!("__xlang_set_nonblock({a})"),
            "set_nodelay" => format!("__xlang_set_nodelay({a})"),
            "open_read" => format!("__xlang_open_read({a})"),
            "read_fd" => format!("__xlang_read_fd({a})"),
            "open_write" => format!("__xlang_open_write({a})"),
            "open_append" => format!("__xlang_open_append({a})"),
            "seek" => {
                let Some(second) = args.get(1) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                format!("__xlang_seek({a}, {b})")
            }
            "make_pipe_at" => format!("__xlang_make_pipe_at({a})"),
            "pipe_r_at" => format!("__xlang_pipe_r_at({a})"),
            "pipe_w_at" => format!("__xlang_pipe_w_at({a})"),
            "exec_sh" => format!("__xlang_exec_sh({a})"),
            "exec_split" => format!("__xlang_exec_split({a})"),
            "dup2" => {
                let Some(second) = args.get(1) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                format!("__xlang_dup2({a}, {b})")
            }
            "cache_open" => format!("__xlang_cache_open({a})"),
            "cache_size" => format!("__xlang_cache_size({a})"),
            "sendfile_fd" => {
                let (Some(second), Some(third)) = (args.get(1), args.get(2)) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                let c = self.gen_expr(third)?;
                format!("__xlang_sendfile_fd({a}, {b}, {c})")
            }
            "sendfile_range" => {
                if args.len() < 4 {
                    return Ok(None);
                }
                let b = self.gen_expr(&args[1])?;
                let c = self.gen_expr(&args[2])?;
                let d = self.gen_expr(&args[3])?;
                format!("__xlang_sendfile_range({a}, {b}, {c}, {d})")
            }
            "send_str" => {
                let Some(second) = args.get(1) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                format!("send({a}, {b}, strlen({b}), 0)")
            }
            "send_bytes" => {
                let (Some(second), Some(third)) = (args.get(1), args.get(2)) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                let c = self.gen_expr(third)?;
                format!("send({a}, {b}, (size_t)({c}), 0)")
            }
            "send_rbuf" => {
                let Some(second) = args.get(1) else {
                    return Ok(None);
                };
                let b = self.gen_expr(second)?;
                format!("__xlang_send_rbuf({a}, {b})")
            }
            "write_rbuf" => {
                if args.len() < 3 {
                    return Ok(None);
                }
                let b = self.gen_expr(&args[1])?;
                let c = self.gen_expr(&args[2])?;
                format!("__xlang_write_rbuf({a}, {b}, {c})")
            }
            _ => return Ok(None),
        };
        Ok(Some(rendered))
    }

    /// Lower `v.push(x)` on a `Vec<T>` variable to a call of the per-element
    /// runtime helper `__xlang_vec_push_T(&v, x)` (emitted in the typedef pass).
    fn try_vec_push_call(
        &self,
        callee: &Spanned<Expr>,
        args: &[Spanned<Expr>],
    ) -> XResult<Option<String>> {
        let Expr::FieldAccessExpr { object, field } = &callee.node else {
            return Ok(None);
        };
        if field != "push" || args.len() != 1 {
            return Ok(None);
        }
        // Resolve the Vec element type from a variable OR the type map (so
        // `self.items.push(x)` works — with `mut self`, `self` is a pointer
        // and `&(*self).items` is the real address → mutation persists).
        let obj_ty = if let Expr::Identifier { name } = &object.node {
            self.lookup_var(name).cloned()
        } else {
            self.types.type_node(object)
        };
        let Some(TypeNode::TypeExpr { name, args: targs }) = obj_ty else {
            return Ok(None);
        };
        if name != "Vec" || targs.len() != 1 {
            return Ok(None);
        }
        let elem_suffix = self.c_type_suffix(&targs[0])?;
        let v_c = self.gen_expr(object)?;
        let x_c = self.gen_expr(&args[0])?;
        Ok(Some(format!(
            "__xlang_vec_push_{elem_suffix}(&{v_c}, {x_c})"
        )))
    }

    /// Vec pop: `v.pop()` → `__xlang_vec_pop_T(&v)`. Mirrors try_vec_push_call's
    /// receiver-type resolution (variable or type-map field access).
    fn try_vec_pop_call(
        &self,
        callee: &Spanned<Expr>,
        args: &[Spanned<Expr>],
    ) -> XResult<Option<String>> {
        let Expr::FieldAccessExpr { object, field } = &callee.node else {
            return Ok(None);
        };
        if field != "pop" || !args.is_empty() {
            return Ok(None);
        }
        let obj_ty = if let Expr::Identifier { name } = &object.node {
            self.lookup_var(name).cloned()
        } else {
            self.types.type_node(object)
        };
        let Some(TypeNode::TypeExpr { name, args: targs }) = obj_ty else {
            return Ok(None);
        };
        if name != "Vec" || targs.len() != 1 {
            return Ok(None);
        }
        let elem_suffix = self.c_type_suffix(&targs[0])?;
        let v_c = self.gen_expr(object)?;
        Ok(Some(format!("__xlang_vec_pop_{elem_suffix}(&{v_c})")))
    }

    /// Vec/Slice/Array method `len()` → `obj.len` (the runtime field).
    /// Also handles `is_empty()` → `(obj.len == 0)`.
    /// Also handles Vec `insert(idx, val)` and `remove_at(idx)`.
    fn try_vec_len_call(
        &self,
        callee: &Spanned<Expr>,
        args: &[Spanned<Expr>],
    ) -> XResult<Option<String>> {
        let Expr::FieldAccessExpr { object, field } = &callee.node else {
            return Ok(None);
        };
        let obj_ty = if let Expr::Identifier { name } = &object.node {
            self.lookup_var(name).cloned()
        } else {
            self.types.type_node(object)
        };
        let Some(TypeNode::TypeExpr { name, args: targs }) = obj_ty else {
            return Ok(None);
        };
        let is_collection = matches!(
            (name.as_str(), targs.len()),
            ("Vec", 1) | ("Slice", 1) | ("Array", 2)
        );
        if !is_collection {
            return Ok(None);
        }
        let obj_c = self.gen_expr(object)?;
        match field.as_str() {
            "len" if args.is_empty() => Ok(Some(format!("({obj_c}.len)"))),
            "is_empty" if args.is_empty() => Ok(Some(format!("({obj_c}.len == 0)"))),
            _ if name == "Vec" && targs.len() == 1 => {
                let elem_suffix = self.c_type_suffix(&targs[0])?;
                match field.as_str() {
                    "insert" if args.len() == 2 => {
                        let idx = self.gen_expr(&args[0])?;
                        let val = self.gen_expr(&args[1])?;
                        Ok(Some(format!(
                            "__xlang_vec_insert_{elem_suffix}(&{obj_c}, {idx}, {val})"
                        )))
                    }
                    "remove_at" if args.len() == 1 => {
                        let idx = self.gen_expr(&args[0])?;
                        Ok(Some(format!(
                            "__xlang_vec_remove_{elem_suffix}(&{obj_c}, {idx})"
                        )))
                    }
                    _ => Ok(None),
                }
            }
            _ => Ok(None),
        }
    }

    /// Zero-argument builtins (`fork`, `getpid`) — lower to the C calls. They
    /// need <unistd.h>, which the guarded networking preamble includes on Linux.
    fn try_zero_arg_call(
        &self,
        callee: &Spanned<Expr>,
        args: &[Spanned<Expr>],
    ) -> XResult<Option<String>> {
        if !args.is_empty() {
            return Ok(None);
        }
        let Expr::Identifier { name } = &callee.node else {
            return Ok(None);
        };
        Ok(Some(match name.as_str() {
            "fork" => "fork()".to_string(),
            "getpid" => "getpid()".to_string(),
            "make_pipe" => "__xlang_make_pipe()".to_string(),
            "pipe_read_end" => "__xlang_pipe_read_end()".to_string(),
            "pipe_write_end" => "__xlang_pipe_write_end()".to_string(),
            "wait_child" => "__xlang_wait_child()".to_string(),
            "wait_status" => "__xlang_wait_status()".to_string(),
            "epoll_create" => "__xlang_epoll_create()".to_string(),
            "argc" => "(__xlang_argc_g)".to_string(),
            "read_stdin" => "__xlang_read_stdin()".to_string(),
            "read_line" => "__xlang_read_line()".to_string(),
            "sb_new" => "__xlang_sb_new()".to_string(),
            "sb_str" => "__xlang_sb_str()".to_string(),
            "ignore_sigpipe" => "signal(SIGPIPE, SIG_IGN)".to_string(),
            "time_str" => "__xlang_time_str()".to_string(),
            "now_s" => "__xlang_now_s()".to_string(),
            "time_now" => "__xlang_time_now()".to_string(),
            "random_seed" => "srand((unsigned)time(NULL))".to_string(),
            "getcwd" => "__xlang_getcwd()".to_string(),
            "env_count" => "__xlang_env_count()".to_string(),
            "tty" => "__xlang_tty()".to_string(),
            "uname_machine" => "__xlang_uname_machine()".to_string(),
            "rbuf_str" => "__xlang_rbuf_str()".to_string(),
            // unreachable() → runtime abort (prints + exits). The trailing 0
            // lets it sit in any expression position.
            "unreachable" => "(__xlang_unreachable_(), 0)".to_string(),
            _ => return Ok(None),
        }))
    }

    fn try_print_call(
        &self,
        callee: &Spanned<Expr>,
        args: &[Spanned<Expr>],
    ) -> XResult<Option<String>> {
        let Expr::Identifier { name } = &callee.node else {
            return Ok(None);
        };
        if args.len() != 1 {
            return Ok(None);
        }
        self.try_print_builtin(name, &args[0])
    }

    fn try_print_builtin(&self, name: &str, arg: &Spanned<Expr>) -> XResult<Option<String>> {
        let arg_c = self.gen_expr(arg)?;
        let rendered = match name {
            "print_i32" => format!("printf(\"%d\\n\", {arg_c})"),
            "print_f64" => format!("printf(\"%f\\n\", {arg_c})"),
            "print_str" => format!("printf(\"%s\\n\", {arg_c})"),
            "print_bool" => format!("printf(\"%s\\n\", ({arg_c}) ? \"true\" : \"false\")"),
            _ => return Ok(None),
        };
        Ok(Some(rendered))
    }

    fn gen_expr(&self, expr: &Spanned<Expr>) -> XResult<String> {
        match &expr.node {
            Expr::IntLiteral { value } | Expr::FloatLiteral { value } => Ok(value.clone()),
            Expr::StringLiteral { value } => {
                Ok(serde_json::to_string(value)?.replace("\\u001b", "\\x1b"))
            }
            Expr::BoolLiteral { value } => Ok(if *value { "true" } else { "false" }.to_string()),
            Expr::Identifier { name } => {
                // In a `mut self` method, `self` is a pointer → `(*self)`.
                // This makes `self.field` → `(*self).field`, `return self` →
                // `(*self)`, and `self.m()` → receiver `(*self)` → `&(*self)` = self.
                if self.in_mut_self && name == "self" {
                    return Ok("(*self)".to_string());
                }
                // A unit-variant enum constant. For a unit-only enum → its index;
                // for a unit variant of a payload enum → a tagged struct literal.
                if let Some(idx) = self.enum_values.get(name) {
                    if let Some(en) = self.variant_enum.get(name)
                        && self.enum_has_payload(en)
                    {
                        return Ok(format!("({en}){{ .tag = {idx} }}"));
                    }
                    return Ok(idx.to_string());
                }
                Ok(name.clone())
            }
            Expr::ArrayLiteral { .. } => Err(XError::Codegen(
                "array literals are only supported in typed Array<T, N> let initializers"
                    .to_string(),
            )),
            Expr::BinaryExpr { op, left, right } => {
                let str_operand = self.types.is_string(left) || self.types.is_string(right);
                // `+` on strings lowers to __xlang_str_concat (decided by the
                // type map: an operand inferred as String makes this a concat).
                if op == "+" && str_operand {
                    let l = self.gen_expr(left)?;
                    let r = self.gen_expr(right)?;
                    Ok(format!("__xlang_str_concat({l}, {r})"))
                }
                // `*` on a string repeats it: `s * n` or `n * s` → str_repeat.
                else if op == "*" && str_operand {
                    if self.types.is_string(left) {
                        let s = self.gen_expr(left)?;
                        let n = self.gen_expr(right)?;
                        Ok(format!("__xlang_str_repeat({s}, {n})"))
                    } else {
                        let s = self.gen_expr(right)?;
                        let n = self.gen_expr(left)?;
                        Ok(format!("__xlang_str_repeat({s}, {n})"))
                    }
                }
                // String comparison (`< <= > >= == !=`) lowers to strcmp(...) <op> 0.
                // Without this, `s1 == s2` would be C pointer comparison (a bug —
                // always false for distinct allocations); strcmp compares content.
                else if matches!(op.as_str(), "<" | "<=" | ">" | ">=" | "==" | "!=")
                    && str_operand
                {
                    let l = self.gen_expr(left)?;
                    let r = self.gen_expr(right)?;
                    Ok(format!("(strcmp({l}, {r}) {op} 0)"))
                } else {
                    Ok(format!(
                        "({} {} {})",
                        self.gen_expr(left)?,
                        op,
                        self.gen_expr(right)?
                    ))
                }
            }
            Expr::UnaryExpr { op, value } => Ok(format!("({}{})", op, self.gen_expr(value)?)),
            Expr::AssignmentExpr { target, value } => Ok(format!(
                "({} = {})",
                self.gen_expr(target)?,
                self.gen_expr(value)?
            )),
            Expr::CallExpr { callee, args } => {
                if let Some(rendered) = self.try_zero_arg_call(callee, args)? {
                    return Ok(rendered);
                }
                if let Some(rendered) = self.try_print_call(callee, args)? {
                    return Ok(rendered);
                }
                if let Some(rendered) = self.try_string_call(callee, args)? {
                    return Ok(rendered);
                }
                if let Some(rendered) = self.try_vec_push_call(callee, args)? {
                    return Ok(rendered);
                }
                // Vec pop: `v.pop()` → __xlang_vec_pop_T(&v).
                if let Some(rendered) = self.try_vec_pop_call(callee, args)? {
                    return Ok(rendered);
                }
                // Vec/Slice/Array len/is_empty: `v.len()` → v.len.
                if let Some(rendered) = self.try_vec_len_call(callee, args)? {
                    return Ok(rendered);
                }
                // Method call: `obj.method(args)` → if obj's type has a method
                // `method`, dispatch to the mangled free function with obj
                // prepended: __xlang_method_<Type>_<method>(obj, args...).
                if let Expr::FieldAccessExpr { object, field } = &callee.node
                    && let Some(ty_name) = self.types.type_name(object)
                    && let Some(mangled) =
                        self.methods.get(&(ty_name.clone(), field.clone())).cloned()
                {
                    // For `mut self` methods, pass the receiver by address (&)
                    // so the method's pointer param sees the caller's object.
                    let is_mut = self.mut_self.contains(&(ty_name, field.clone()));
                    let receiver = self.gen_expr(object)?;
                    let prefix = if is_mut { "&" } else { "" };
                    let mut parts = vec![format!("{prefix}{receiver}")];
                    for arg in args {
                        parts.push(self.gen_expr(arg)?);
                    }
                    return Ok(format!("{mangled}({})", parts.join(", ")));
                }
                // Enum payload-variant construction: `Err(msg)` →
                // (E){ .tag = idx, .u.v<idx> = <payload> }.
                if let Expr::Identifier { name } = &callee.node
                    && let Some(idx) = self.enum_values.get(name).copied()
                    && let Some(en) = self.variant_enum.get(name).cloned()
                    && self.enum_has_payload(&en)
                    && args.len() == 1
                {
                    let val = self.gen_expr(&args[0])?;
                    return Ok(format!("({en}){{ .tag = {idx}, .u.v{idx} = {val} }}"));
                }
                let mut parts = Vec::new();
                for arg in args {
                    parts.push(self.gen_expr(arg)?);
                }
                Ok(format!("{}({})", self.gen_expr(callee)?, parts.join(", ")))
            }
            Expr::FieldAccessExpr { object, field } => {
                Ok(format!("{}.{}", self.gen_expr(object)?, field))
            }
            Expr::StructLiteral { name, fields } => {
                // Look up field types so type-directed values (like vec_new())
                // can be lowered via try_constructor — the declared field type
                // provides the Vec<T> / Option<T> context that gen_expr lacks.
                let field_types = self.struct_fields.get(name).cloned();
                let mut parts = Vec::new();
                for f in fields {
                    let val = if let Some(ref fts) = field_types {
                        let field_ty = fts
                            .iter()
                            .find(|(n, _)| n == &f.name)
                            .map(|(_, t)| t.clone());
                        if let Some(ty) = field_ty
                            && let Some(rendered) = self.try_constructor(&ty, &f.value)?
                        {
                            rendered
                        } else {
                            self.gen_expr(&f.value)?
                        }
                    } else {
                        self.gen_expr(&f.value)?
                    };
                    parts.push(format!(".{} = {val}", f.name));
                }
                Ok(format!("({name}){{ {} }}", parts.join(", ")))
            }
            Expr::IndexExpr { object, index } => {
                let idx_c = self.gen_expr(index)?;
                // String/Str indexing: `s[i]` → byte value as i32 (unsigned).
                // Same as str_char_at but with the natural subscript syntax.
                if self.types.is_string(object) {
                    let obj_c = self.gen_expr(object)?;
                    return Ok(format!("((int32_t)(unsigned char){obj_c}[{idx_c}])"));
                }
                // Array<T,N> and Slice<T>/Vec<T> store elements in `.data`.
                Ok(format!("{}.data[{idx_c}]", self.gen_expr(object)?))
            }
            Expr::RangeExpr { .. } => Err(XError::Codegen(
                "range expressions (a..b) are only supported as the iterable of a `for` loop"
                    .to_string(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CGen;
    use crate::lexer::Lexer;
    use crate::parser::Parser;

    fn gen_c(source: &str) -> String {
        let (tokens, _) = Lexer::new(source).tokenize();
        let program = Parser::new(tokens, "<test>").parse().expect("parse source");
        CGen::new().generate(&program).expect("codegen")
    }

    /// Like `gen_c` but runs the full typed pipeline (parse → typecheck →
    /// codegen with the TypeMap), so type-dependent lowering (string `+`) is
    /// exercised the same way `write_c` does it.
    fn gen_c_typed(source: &str) -> String {
        let (tokens, _) = Lexer::new(source).tokenize();
        let program = Parser::new(tokens, "<test>").parse().expect("parse source");
        let (_diags, types) = crate::typecheck::check_program_typed(&program);
        CGen::with_types(types).generate(&program).expect("codegen")
    }

    #[test]
    fn lowers_option_match_to_if_else() {
        let c = gen_c(
            "module main\nfn f(o: Option<i32>): i32 { match o { Some(v) => { return v } None => { return 0 } } }\nfn main(): i32 { return 0 }",
        );
        assert!(c.contains("typedef struct"), "no Option struct: {c}");
        assert!(c.contains(".some"), "no .some field: {c}");
        assert!(c.contains("if (o.some)"), "no match lowering: {c}");
    }

    #[test]
    fn lowers_result_match_to_if_else() {
        let c = gen_c(
            "module main\nfn f(r: Result<i32, String>): i32 { match r { Ok(v) => { return v } Err(e) => { return 0 } } }\nfn main(): i32 { return 0 }",
        );
        assert!(c.contains(".ok"), "no .ok field: {c}");
        assert!(c.contains("if (r.ok)"), "no result match lowering: {c}");
    }

    #[test]
    fn emits_struct_literal_compound() {
        let c = gen_c(
            "module main\nstruct P { x: i32 }\nfn main(): i32 { let p: P = P { x: 1 } return p.x }",
        );
        assert!(c.contains("(P){ .x ="), "no struct literal: {c}");
    }

    #[test]
    fn emits_vec_push_helper_call() {
        let c = gen_c(
            "module main\nfn main(): i32 { let mut v: Vec<i32> = vec_new() v.push(1) return 0 }",
        );
        assert!(c.contains("__xlang_vec_push_i32(&v,"), "no vec push: {c}");
    }

    #[test]
    fn emits_fork_call() {
        let c = gen_c("module main\nfn main(): i32 { let p: i32 = fork() return p }");
        assert!(c.contains("fork();"), "no fork: {c}");
    }

    #[test]
    fn lowers_for_in_over_array() {
        let c = gen_c(
            "module main\nfn main(): i32 { let a: Array<i32, 3> = [1, 2, 3] for n in a { print_i32(n) } return 0 }",
        );
        assert!(c.contains("< 3;"), "no array bound N: {c}");
        assert!(c.contains(".data["), "no .data index: {c}");
    }

    #[test]
    fn lowers_numeric_range_for_loop() {
        // `for i in 0..n` -> C `for (i = 0; i < bound; i++)`, with the end
        // captured once into a temp so the bound is fixed at loop entry.
        let c = gen_c(
            "module main\nfn sum(n: i32): i32 { let mut s: i32 = 0 for i in 0..n { s += i } return s } fn main(): i32 { return sum(5) }",
        );
        assert!(c.contains("__xlang_rg_end"), "no captured range bound: {c}");
        assert!(
            c.contains("for (int32_t i = 0; i < __xlang_rg_end"),
            "no numeric for-loop: {c}"
        );
        assert!(c.contains("i++)"), "no increment: {c}");
    }

    #[test]
    fn lowers_inclusive_range_for_loop() {
        // `for i in 0..=n` -> C `for (i = 0; i <= bound; i++)`.
        let c = gen_c(
            "module main\nfn main(): i32 { let mut c: i32 = 0 for k in 1..=5 { c += k } return c }",
        );
        assert!(
            c.contains("for (int32_t k = 1; k <= __xlang_rg_end"),
            "no inclusive numeric for-loop: {c}"
        );
    }

    #[test]
    fn lowers_string_plus_to_concat() {
        // `s1 + s2` lowers to __xlang_str_concat (driven by the type map).
        let c = gen_c_typed(
            "module main\nfn cat(a: String, b: String): String { return a + b }\nfn main(): i32 { return 0 }",
        );
        assert!(
            c.contains("return __xlang_str_concat(a, b);"),
            "string + should lower to concat: {c}"
        );
        // A numeric + must stay a plain add (the runtime always *defines*
        // __xlang_str_concat, so check the call site, not the bare name).
        let c2 = gen_c_typed(
            "module main\nfn add(a: i32, b: i32): i32 { return a + b }\nfn main(): i32 { return 0 }",
        );
        assert!(
            c2.contains("return (a + b);"),
            "numeric + should stay an add: {c2}"
        );
        assert!(
            !c2.contains("__xlang_str_concat(a, b)"),
            "numeric + must not call concat: {c2}"
        );
    }

    #[test]
    fn lowers_string_comparison_to_strcmp() {
        // `s1 < s2` → strcmp(s1, s2) < 0 ; `s1 == s2` → strcmp(s1, s2) == 0
        // (without this, == would be C pointer comparison — a latent bug).
        let c = gen_c_typed(
            "module main\nfn lt(a: String, b: String): bool { return a < b }\nfn main(): i32 { return 0 }",
        );
        assert!(
            c.contains("return (strcmp(a, b) < 0);"),
            "string < should lower to strcmp: {c}"
        );
        let c2 = gen_c_typed(
            "module main\nfn eq(a: String, b: String): bool { return a == b }\nfn main(): i32 { return 0 }",
        );
        assert!(
            c2.contains("return (strcmp(a, b) == 0);"),
            "string == should lower to strcmp: {c2}"
        );
    }

    #[test]
    fn lowers_method_call_to_mangled_function() {
        // `p.length()` → __xlang_method_Point_length(p); method bodies compile
        // as the mangled free function.
        let c = gen_c_typed(
            "module main\nstruct Point { x: i32 }\nimpl Point { fn sq(self: Point): i32 { return self.x * self.x } }\nfn main(): i32 { let p: Point = Point { x: 3 } return p.sq() }",
        );
        assert!(
            c.contains("__xlang_method_Point_sq"),
            "method should compile to a mangled function: {c}"
        );
        assert!(
            c.contains("return __xlang_method_Point_sq(p);"),
            "method call should dispatch to the mangled function with receiver: {c}"
        );
    }

    #[test]
    fn lowers_if_let_to_match_with_temp() {
        // `if let Some(v) = func() { .. }` desugars to a match whose scrutinee
        // is a call; codegen binds it to a typed temp so it can read .some.
        let c = gen_c_typed(
            "module main\nfn f(): Option<i32> { return Some(1) }\nfn main(): i32 { if let Some(v) = f() { return v } return 0 }",
        );
        assert!(
            c.contains("__xlang_m") && c.contains(".some"),
            "if let on a call should bind a temp and test the discriminant: {c}"
        );
    }

    #[test]
    fn orders_vec_typedef_before_struct_using_it() {
        // A struct with a `Vec<T>` field must be emitted AFTER the `Vec_T`
        // typedef (it contains `Vec_T counts;` by value). Regression for a bug
        // where the struct was emitted first → "unknown type name Vec_i32".
        let c =
            gen_c_typed("module main\nstruct Bag { items: Vec<i32> }\nfn main(): i32 { return 0 }");
        let vec_pos = c.find("} Vec_i32;").unwrap_or(usize::MAX);
        let struct_pos = c.find("} Bag;").unwrap_or(usize::MAX);
        assert!(
            vec_pos < struct_pos,
            "Vec_i32 typedef should precede the Bag struct that uses it: {c}"
        );
    }

    #[test]
    fn lowers_for_in_over_field_access() {
        // `for v in self.items` (a field-access iterable) binds a temp and
        // iterates it — the iterable no longer has to be a bare identifier.
        let c = gen_c_typed(
            "module main\nstruct B { xs: Vec<i32> }\nimpl B { fn s(self: B): i32 { let mut t: i32 = 0 for v in self.xs { t += v } return t } }\nfn main(): i32 { return 0 }",
        );
        assert!(
            c.contains("__xlang_it") && c.contains(".len;"),
            "for-in over a field should bind a temp and use .len: {c}"
        );
    }

    #[test]
    fn lowers_match_or_and_range_patterns() {
        // `1 | 2` → `(x == 1 || x == 2)`; `3..=5` → `(x >= 3 && x <= 5)`.
        let c = gen_c(
            "module main\nfn f(x: i32): i32 { match x { 1 | 2 => { return 1 } 3..=5 => { return 2 } _ => { return 0 } } }\nfn main(): i32 { return 0 }",
        );
        assert!(
            c.contains("x == 1 || x == 2"),
            "OR pattern should lower to ||: {c}"
        );
        assert!(
            c.contains("x >= 3 && x <= 5"),
            "inclusive range should lower to <=: {c}"
        );
    }

    #[test]
    fn lowers_int_to_i64_to_cast() {
        // int_to_i64 was in the type table but not lowered → undefined symbol.
        // It must lower to a C cast, not a function call.
        let c = gen_c(
            "module main\nfn widen(x: i32): i64 { return int_to_i64(x) }\nfn main(): i32 { return 0 }",
        );
        assert!(
            c.contains("(int64_t)(x)") && !c.contains("int_to_i64("),
            "int_to_i64 should lower to a cast: {c}"
        );
    }

    #[test]
    fn lowers_string_repeat_to_str_repeat() {
        // `s * n` → __xlang_str_repeat(s, n); numeric `*` must stay a multiply.
        let c = gen_c_typed(
            "module main\nfn rep(s: String, n: i32): String { return s * n }\nfn main(): i32 { return 0 }",
        );
        assert!(
            c.contains("__xlang_str_repeat(s, n)"),
            "string * should lower to str_repeat: {c}"
        );
        let c2 = gen_c_typed(
            "module main\nfn mul(a: i32, b: i32): i32 { return a * b }\nfn main(): i32 { return 0 }",
        );
        assert!(
            c2.contains("return (a * b)"),
            "numeric * should stay a multiply: {c2}"
        );
        // The runtime always *defines* __xlang_str_repeat, so check the call
        // site, not the bare name.
        assert!(
            !c2.contains("__xlang_str_repeat(a, b)"),
            "numeric * must not call str_repeat: {c2}"
        );
    }

    #[test]
    fn lowers_unit_variant_enum() {
        // An enum lowers to int32_t; a variant `B` → its index; `match` on a
        // variant pattern compares the index.
        let c = gen_c(
            "module main\nenum E { A, B, C }\nfn f(x: E): i32 { match x { A => { return 0 } B => { return 1 } _ => { return 9 } } }\nfn main(): i32 { let e: E = B return f(e) }",
        );
        // variant B is index 1, both at construction and in the match arm.
        assert!(
            c.contains("int32_t e = 1;"),
            "variant B should be value 1: {c}"
        );
        assert!(c.contains("x == 1"), "match arm B should compare to 1: {c}");
    }

    #[test]
    fn lowers_enum_method_call() {
        // `impl Enum { fn m(self) }` reuses the struct-method machinery: the
        // receiver's type (an enum → its name) dispatches to the mangled fn.
        let c = gen_c_typed(
            "module main\nenum E { A, B }\nimpl E { fn idx(self: E): i32 { return self } }\nfn main(): i32 { let e: E = B return e.idx() }",
        );
        assert!(
            c.contains("__xlang_method_E_idx"),
            "enum method should compile to the mangled function: {c}"
        );
        assert!(
            c.contains("return __xlang_method_E_idx(e);"),
            "enum method call should dispatch with the receiver: {c}"
        );
    }

    #[test]
    fn lowers_payload_enum_construction_and_match() {
        // `enum S { Ok, Err(String) }` → tagged struct; `Err("x")` sets
        // .tag + .u.v1; `Err(m) =>` reads m from .u.v1.
        let c = gen_c_typed(
            "module main\nenum S { Ok, Err(String) }\nfn f(s: S): String { match s { Ok => { return \"\" } Err(m) => { return m } } }\nfn main(): i32 { let s: S = Err(\"x\") print_str(f(s)) return 0 }",
        );
        assert!(
            c.contains("typedef struct S") && c.contains("union"),
            "payload enum should emit a tagged struct with a union: {c}"
        );
        assert!(
            c.contains(".tag = 1, .u.v1 ="),
            "Err(...) construction should set tag 1 and the v1 payload: {c}"
        );
        assert!(
            c.contains(".tag == 1)") && c.contains("= s.u.v1;"),
            "Err(m) arm should test .tag==1 and bind m from .u.v1: {c}"
        );
    }

    #[test]
    fn lowers_enum_with_struct_payload_and_wildcard() {
        // A struct payload (`Two(Pair)`) and a wildcard arm: the struct binds
        // through the union member, and `_ =>` lowers to the final `else`.
        let c = gen_c_typed(
            "module main\nstruct Pair { a: i32\n b: i32 }\nenum E { Empty, Two(Pair) }\nfn f(e: E): i32 { match e { Empty => { return 0 } Two(p) => { return p.a + p.b } _ => { return -1 } } }\nfn main(): i32 { return 0 }",
        );
        assert!(
            c.contains("= e.u.v1;"),
            "Two(p) arm should bind the struct payload from .u.v1: {c}"
        );
        assert!(
            c.contains("} else {"),
            "wildcard arm should lower to the final else: {c}"
        );
    }

    #[test]
    fn emits_match_literal_if_else() {
        let c = gen_c(
            "module main\nfn main(): i32 { let x: i32 = 2 match x { 1 => { return 1 } _ => { return 0 } } }",
        );
        assert!(c.contains("if (x == 1)"), "no literal match if: {c}");
        assert!(c.contains("} else {"), "no wildcard else: {c}");
    }

    #[test]
    fn emits_print_printf() {
        let c = gen_c("module main\nfn main(): i32 { print_i32(42) return 0 }");
        assert!(c.contains("printf("), "no printf: {c}");
    }

    #[test]
    fn emits_array_literal_initializer() {
        let c = gen_c("module main\nfn main(): i32 { let a: Array<i32, 2> = [1, 2] return 0 }");
        assert!(c.contains(".data = {"), "no array literal init: {c}");
    }

    #[test]
    fn emits_function_prototype() {
        let c = gen_c(
            "module main\nfn helper(x: i32): i32 { return x }\nfn main(): i32 { return helper(1) }",
        );
        assert!(
            c.contains("int32_t helper(int32_t x);"),
            "no prototype: {c}"
        );
    }

    #[test]
    fn emits_str_eq_as_strcmp() {
        let c = gen_c(
            "module main\nfn f(a: String, b: String): bool { return str_eq(a, b) }\nfn main(): i32 { return 0 }",
        );
        assert!(c.contains("strcmp("), "no strcmp for str_eq: {c}");
    }

    #[test]
    fn emits_str_find_and_slice_helpers() {
        let c = gen_c(
            "module main\nfn main(): i32 { let s: String = \"hi\" let i: i32 = str_find(s, \"h\") let t: String = str_slice(s, 0, 1) return 0 }",
        );
        assert!(c.contains("__xlang_str_find("), "no str_find: {c}");
        assert!(c.contains("__xlang_str_slice("), "no str_slice: {c}");
    }

    #[test]
    fn emits_str_lower_upper() {
        let c = gen_c(
            "module main\nfn main(): i32 { let s: String = \"Hi\" let a: String = str_lower(s) let b: String = str_upper(s) return 0 }",
        );
        assert!(c.contains("__xlang_str_lower("), "no str_lower: {c}");
        assert!(c.contains("__xlang_str_upper("), "no str_upper: {c}");
    }

    #[test]
    fn emits_str_repeat_chr() {
        let c = gen_c(
            "module main\nfn main(): i32 { let s: String = str_repeat(\"ab\", 3) let c: String = chr(65) return 0 }",
        );
        assert!(c.contains("__xlang_str_repeat("), "no str_repeat: {c}");
        assert!(c.contains("__xlang_chr("), "no chr: {c}");
    }

    #[test]
    fn emits_time_format_helpers_and_calls() {
        // `time_format(fmt)` / `time_format_utc(fmt)` lower to the strftime
        // wrappers, and the wrappers themselves are emitted into the preamble.
        let c = gen_c(
            "module main\nfn main(): i32 { let a: String = time_format(\"%Y-%m-%d\") let b: String = time_format_utc(\"%H:%M:%S\") let d: String = time_format_at(\"%Y\", 0) let e: String = time_format_at_utc(\"%Y\", 0) return 0 }",
        );
        assert!(
            c.contains("__xlang_time_format("),
            "no time_format call: {c}"
        );
        assert!(
            c.contains("__xlang_time_format_utc("),
            "no time_format_utc call: {c}"
        );
        assert!(
            c.contains("__xlang_time_format_at("),
            "no time_format_at call: {c}"
        );
        assert!(
            c.contains("__xlang_time_format_at_utc("),
            "no time_format_at_utc call: {c}"
        );
        assert!(
            c.contains("char* __xlang_time_format(const char* fmt)"),
            "no time_format helper definition: {c}"
        );
        assert!(
            c.contains("char* __xlang_time_format_utc(const char* fmt)"),
            "no time_format_utc helper definition: {c}"
        );
        assert!(
            c.contains("char* __xlang_time_format_at(const char* fmt, int32_t epoch)"),
            "no time_format_at helper definition: {c}"
        );
    }

    #[test]
    fn emits_time_now_wall_epoch_builtin() {
        // `time_now()` → wall-clock epoch (time(NULL)), distinct from the
        // monotonic `now_s`. Used by relative-date arithmetic (date -d yesterday).
        let c = gen_c("module main\nfn main(): i32 { let t: i32 = time_now() return t }");
        assert!(c.contains("__xlang_time_now()"), "no time_now call: {c}");
        assert!(
            c.contains("int32_t __xlang_time_now()"),
            "no time_now helper definition: {c}"
        );
    }

    #[test]
    fn emits_recv_all_drain_builtin() {
        // `recv_all(fd)` drains a non-blocking socket into one growable buffer
        // (loops recv to EAGAIN), so servers can read HTTP bodies > 64KB.
        let c = gen_c("module main\nfn main(): i32 { let s: String = recv_all(3) return 0 }");
        assert!(c.contains("__xlang_recv_all("), "no recv_all call: {c}");
        assert!(
            c.contains("char* __xlang_recv_all(int32_t fd)"),
            "no recv_all helper definition: {c}"
        );
    }

    #[test]
    fn emits_tcp_listen_reuseport_builtin() {
        // `tcp_listen_reuseport(port)` sets SO_REUSEPORT before bind so a prefork
        // worker pool can share one port (nginx multi-worker model).
        let c = gen_c(
            "module main\nfn main(): i32 { let fd: i32 = tcp_listen_reuseport(8080) return fd }",
        );
        assert!(
            c.contains("__xlang_tcp_listen_reuseport("),
            "no tcp_listen_reuseport call: {c}"
        );
        assert!(
            c.contains("int32_t __xlang_tcp_listen_reuseport(int32_t port)"),
            "no tcp_listen_reuseport helper definition: {c}"
        );
    }

    #[test]
    fn str_find_from_has_no_per_call_strlen() {
        // str_find_from must NOT call strlen(s) — that's O(n) per call, making
        // loops over it (wc -l's count_lines) O(n²). Regressed wc -l on Linux.
        let c = gen_c("module main\nfn f(s: String): i32 { return str_find_from(s, \"x\", 0) }");
        let helper = c
            .find("int32_t __xlang_str_find_from")
            .map(|i| &c[i..])
            .unwrap_or("");
        assert!(
            !helper[..400].contains("strlen(s)"),
            "str_find_from calls strlen(s) per call (O(n²) in loops): {helper}"
        );
    }

    #[test]
    fn str_translate_is_table_based_and_emits_str_delete() {
        // str_translate now builds a 256-entry table (O(n), vs old O(n*|from|)
        // strchr-per-char), and str_delete is a new O(n) bulk builtin (for tr -d).
        let c = gen_c(
            "module main\nfn f(s: String): i32 { let a: String = str_translate(s, \"a\", \"b\") let d: String = str_delete(s, \"x\") return 0 }",
        );
        assert!(
            !c.contains("strchr(from, s[i])"),
            "str_translate still uses O(n*|from|) strchr: {c}"
        );
        assert!(
            c.contains("char table[256]"),
            "str_translate not table-based: {c}"
        );
        assert!(c.contains("__xlang_str_delete("), "no str_delete call: {c}");
        assert!(
            c.contains("char* __xlang_str_delete(const char* s, const char* set)"),
            "no str_delete helper: {c}"
        );
    }

    #[test]
    fn emits_cat_show_builtin() {
        // cat_show(s, tabs, ends) — bulk cat -A/-E/-T expansion (tab→^I, $ before
        // newline), O(n) in C, vs the per-char loop that made cate/showall slow.
        let c = gen_c(
            "module main\nfn f(s: String): i32 { let a: String = cat_show(s, 1, 1) return 0 }",
        );
        assert!(c.contains("__xlang_cat_show("), "no cat_show call: {c}");
        assert!(
            c.contains(
                "char* __xlang_cat_show(const char* s, int32_t show_tabs, int32_t show_ends)"
            ),
            "no cat_show helper: {c}"
        );
    }

    #[test]
    fn tls_builtins_are_gated_on_usage() {
        // Using a tls_* builtin emits #define __XLANG_TLS__ + the OpenSSL section.
        let with_tls = gen_c(
            "module main\nfn f(cert: String, key: String): i32 { return tls_ctx_new(cert, key) }\nfn main(): i32 { return 0 }",
        );
        assert!(
            with_tls.contains("#define __XLANG_TLS__ 1"),
            "tls user missing #define: {with_tls}"
        );
        assert!(
            with_tls.contains("__xlang_tls_ctx_new("),
            "no tls call: {with_tls}"
        );
        assert!(
            with_tls.contains("int32_t __xlang_tls_ctx_new(const char* cert, const char* key)"),
            "no tls helper: {with_tls}"
        );
        // A program NOT using tls_* must NOT #define the macro (the #ifdef
        // directive is always emitted, but undefined → body excluded, no OpenSSL
        // link needed). Non-TLS servers stay free of the dependency.
        let no_tls =
            gen_c("module main\nfn main(): i32 { let fd: i32 = tcp_listen(8080) return fd }");
        assert!(
            !no_tls.contains("#define __XLANG_TLS__"),
            "non-TLS program defined the TLS macro: {no_tls}"
        );
    }

    #[test]
    fn emits_misc_builtins() {
        let c = gen_c(
            "module main\nfn main(): i32 { let i: i32 = str_find_from(\"a-b\", \"-\", 1) let s: String = str_replace_first(\"a-b\", \"-\", \"+\") let x: i32 = abs(-5) let y: i32 = max(1, 2) let z: i32 = min(1, 2) return 0 }",
        );
        assert!(
            c.contains("__xlang_str_find_from("),
            "no str_find_from: {c}"
        );
        assert!(
            c.contains("__xlang_str_replace_first("),
            "no str_replace_first: {c}"
        );
        assert!(c.contains("__xlang_abs("), "no abs: {c}");
        assert!(c.contains("__xlang_max("), "no max: {c}");
        assert!(c.contains("__xlang_min("), "no min: {c}");
    }

    #[test]
    fn emits_for_in_loop() {
        let c = gen_c(
            "module main\nfn main(): i32 { let v: Vec<i32> = vec_new() for x in v { print_raw(\"hi\") } return 0 }",
        );
        assert!(c.contains(".data["), "no for-in data access: {c}");
        assert!(c.contains("for ("), "no for loop: {c}");
    }
}
