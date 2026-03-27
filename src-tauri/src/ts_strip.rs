use swc_common::input::StringInput;
use swc_common::{sync::Lrc, FileName, SourceMap, Span, Spanned};
use swc_ecma_ast::{
    Class, ClassMethod, ClassProp, Constructor, ExportAll, ExportNamedSpecifier, Function,
    ImportDecl, Module, ParamOrTsParamProp, PrivateMethod, PrivateProp, TsAsExpr, TsEnumDecl,
    TsExprWithTypeArgs, TsInterfaceDecl, TsModuleDecl, TsNonNullExpr, TsSatisfiesExpr,
    TsTypeAliasDecl, TsTypeAnn, TsTypeAssertion, TsTypeParamDecl, TsTypeParamInstantiation,
};
use swc_ecma_parser::{lexer::Lexer, Parser, Syntax, TsSyntax};
use swc_ecma_visit::{Visit, VisitWith};

pub(crate) fn strip_typescript_module(
    source_name: &str,
    source_text: &[u8],
) -> Result<Vec<u8>, String> {
    let source_string = std::str::from_utf8(source_text)
        .map_err(|error| {
            format!("TypeScript source must be valid UTF-8 in {source_name}: {error}")
        })?
        .to_string();
    let source_map: Lrc<SourceMap> = Default::default();
    let source_file = source_map.new_source_file(
        FileName::Custom(source_name.to_string()).into(),
        source_string.clone(),
    );
    let lexer = Lexer::new(
        Syntax::Typescript(TsSyntax {
            tsx: false,
            decorators: false,
            dts: false,
            no_early_errors: false,
            disallow_ambiguous_jsx_like: false,
        }),
        Default::default(),
        StringInput::from(&*source_file),
        None,
    );
    let mut parser = Parser::new_from(lexer);
    let module = parser.parse_module().map_err(|error| {
        format!(
            "failed to parse TypeScript in {source_name}: {:?}",
            error.kind()
        )
    })?;
    if let Some(error) = parser.take_errors().into_iter().next() {
        return Err(format!(
            "failed to parse TypeScript in {source_name}: {:?}",
            error.kind()
        ));
    }

    let mut collector = StripCollector::default();
    module.visit_with(&mut collector);
    if let Some(error) = collector.unsupported.into_iter().next() {
        return Err(format!("unsupported TypeScript in {source_name}: {error}"));
    }

    let start_pos = source_file.start_pos.0;
    let mut bytes = source_text.to_vec();
    for (start, end) in collapse_ranges(collector.removals) {
        let start = start.saturating_sub(start_pos) as usize;
        let end = end.saturating_sub(start_pos) as usize;
        let bounded_end = end.min(bytes.len());
        for byte in &mut bytes[start..bounded_end] {
            if *byte != b'\n' && *byte != b'\r' {
                *byte = b' ';
            }
        }
    }
    Ok(bytes)
}

#[derive(Default)]
struct StripCollector {
    removals: Vec<(u32, u32)>,
    unsupported: Vec<String>,
}

impl StripCollector {
    fn remove(&mut self, span: Span) {
        self.remove_range(span.lo.0, span.hi.0);
    }

    fn remove_range(&mut self, start: u32, end: u32) {
        if end > start {
            self.removals.push((start, end));
        }
    }

    fn unsupported(&mut self, span: Span, description: &str) {
        self.unsupported.push(format!(
            "{description} at {}..{} requires code generation",
            span.lo.0, span.hi.0
        ));
    }

    fn visit_function_like(&mut self, function: &Function) {
        if !function.decorators.is_empty() {
            self.unsupported(function.span, "decorators");
        }
        if let Some(type_params) = &function.type_params {
            self.remove(type_params.span);
        }
        if let Some(return_type) = &function.return_type {
            self.remove(return_type.span);
        }
    }

    fn visit_class_like(&mut self, class: &Class) {
        if !class.decorators.is_empty() {
            self.unsupported(class.span, "class decorators");
        }
        if class.is_abstract {
            self.unsupported(class.span, "abstract classes");
        }
        if !class.implements.is_empty() {
            self.unsupported(class.span, "implements clauses");
        }
        if let Some(type_params) = &class.type_params {
            self.remove(type_params.span);
        }
        if let Some(type_args) = &class.super_type_params {
            self.remove(type_args.span);
        }
    }
}

impl Visit for StripCollector {
    fn visit_module(&mut self, module: &Module) {
        module.visit_children_with(self);
    }

    fn visit_import_decl(&mut self, import_decl: &ImportDecl) {
        if import_decl.type_only {
            self.remove(import_decl.span);
            return;
        }
        if import_decl
            .specifiers
            .iter()
            .any(|specifier| specifier.is_type_only())
        {
            self.unsupported(import_decl.span, "inline type-only import specifiers");
        }
        import_decl.visit_children_with(self);
    }

    fn visit_export_all(&mut self, export_all: &ExportAll) {
        if export_all.type_only {
            self.remove(export_all.span);
            return;
        }
        export_all.visit_children_with(self);
    }

    fn visit_export_named_specifier(&mut self, specifier: &ExportNamedSpecifier) {
        if specifier.is_type_only {
            self.unsupported(specifier.span, "inline type-only export specifiers");
            return;
        }
        specifier.visit_children_with(self);
    }

    fn visit_function(&mut self, function: &Function) {
        self.visit_function_like(function);
        function.visit_children_with(self);
    }

    fn visit_class(&mut self, class: &Class) {
        self.visit_class_like(class);
        class.visit_children_with(self);
    }

    fn visit_constructor(&mut self, constructor: &Constructor) {
        if constructor.accessibility.is_some() || constructor.is_optional {
            self.unsupported(constructor.span, "TypeScript constructor modifiers");
        }
        constructor.visit_children_with(self);
    }

    fn visit_param_or_ts_param_prop(&mut self, param: &ParamOrTsParamProp) {
        if matches!(param, ParamOrTsParamProp::TsParamProp(_)) {
            self.unsupported(param.span(), "parameter properties");
            return;
        }
        param.visit_children_with(self);
    }

    fn visit_class_prop(&mut self, class_prop: &ClassProp) {
        if class_prop.accessibility.is_some()
            || class_prop.is_abstract
            || class_prop.is_optional
            || class_prop.is_override
            || class_prop.readonly
            || class_prop.declare
            || class_prop.definite
            || !class_prop.decorators.is_empty()
        {
            self.unsupported(class_prop.span, "TypeScript class field modifiers");
        }
        class_prop.visit_children_with(self);
    }

    fn visit_private_prop(&mut self, private_prop: &PrivateProp) {
        if private_prop.accessibility.is_some()
            || private_prop.is_optional
            || private_prop.is_override
            || private_prop.readonly
            || private_prop.definite
            || !private_prop.decorators.is_empty()
        {
            self.unsupported(private_prop.span, "TypeScript private field modifiers");
        }
        private_prop.visit_children_with(self);
    }

    fn visit_class_method(&mut self, class_method: &ClassMethod) {
        if class_method.accessibility.is_some()
            || class_method.is_abstract
            || class_method.is_optional
            || class_method.is_override
        {
            self.unsupported(class_method.span, "TypeScript class method modifiers");
        }
        class_method.visit_children_with(self);
    }

    fn visit_private_method(&mut self, private_method: &PrivateMethod) {
        if private_method.accessibility.is_some()
            || private_method.is_abstract
            || private_method.is_optional
            || private_method.is_override
        {
            self.unsupported(private_method.span, "TypeScript private method modifiers");
        }
        private_method.visit_children_with(self);
    }

    fn visit_ts_type_ann(&mut self, type_ann: &TsTypeAnn) {
        self.remove(type_ann.span);
    }

    fn visit_ts_type_param_decl(&mut self, type_params: &TsTypeParamDecl) {
        self.remove(type_params.span);
    }

    fn visit_ts_type_param_instantiation(&mut self, type_args: &TsTypeParamInstantiation) {
        self.remove(type_args.span);
    }

    fn visit_ts_expr_with_type_args(&mut self, expr: &TsExprWithTypeArgs) {
        if let Some(type_args) = &expr.type_args {
            self.remove(type_args.span);
        }
        expr.visit_children_with(self);
    }

    fn visit_ts_interface_decl(&mut self, interface_decl: &TsInterfaceDecl) {
        self.remove(interface_decl.span);
    }

    fn visit_ts_type_alias_decl(&mut self, type_alias: &TsTypeAliasDecl) {
        self.remove(type_alias.span);
    }

    fn visit_ts_enum_decl(&mut self, enum_decl: &TsEnumDecl) {
        self.unsupported(enum_decl.span, "enums");
    }

    fn visit_ts_module_decl(&mut self, module_decl: &TsModuleDecl) {
        self.unsupported(module_decl.span, "namespaces");
    }

    fn visit_ts_as_expr(&mut self, expr: &TsAsExpr) {
        self.remove_range(expr.expr.span().hi.0, expr.span.hi.0);
        expr.visit_children_with(self);
    }

    fn visit_ts_satisfies_expr(&mut self, expr: &TsSatisfiesExpr) {
        self.remove_range(expr.expr.span().hi.0, expr.span.hi.0);
        expr.visit_children_with(self);
    }

    fn visit_ts_non_null_expr(&mut self, expr: &TsNonNullExpr) {
        self.remove_range(expr.expr.span().hi.0, expr.span.hi.0);
        expr.visit_children_with(self);
    }

    fn visit_ts_type_assertion(&mut self, expr: &TsTypeAssertion) {
        self.remove_range(expr.span.lo.0, expr.expr.span().lo.0);
        expr.visit_children_with(self);
    }
}

fn collapse_ranges(mut ranges: Vec<(u32, u32)>) -> Vec<(u32, u32)> {
    ranges.sort_by_key(|(start, end)| (*start, *end));
    let mut merged: Vec<(u32, u32)> = Vec::new();
    for (start, end) in ranges {
        match merged.last_mut() {
            Some((merged_start, merged_end)) if start <= *merged_end => {
                *merged_end = (*merged_end).max(end);
                *merged_start = (*merged_start).min(start);
            }
            _ => merged.push((start, end)),
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::strip_typescript_module;

    #[test]
    fn strip_typescript_module_removes_erasable_syntax() {
        let source = br#"
import type { Foo } from './foo.ts';

type Balance = { amount: number };

export function parseAmount<T>(value: string): number {
  return Number(value as string satisfies string)!;
}
"#;
        let stripped = match strip_typescript_module("driver.ts", source) {
            Ok(stripped) => stripped,
            Err(error) => panic!("erasable TS should strip: {error}"),
        };
        let stripped_text = match String::from_utf8(stripped) {
            Ok(text) => text,
            Err(error) => panic!("stripped text should be utf-8: {error}"),
        };
        assert!(stripped_text.contains("return Number(value"));
        assert!(!stripped_text.contains("type Balance"));
        assert!(!stripped_text.contains("import type"));
    }

    #[test]
    fn strip_typescript_module_rejects_non_erasable_syntax() {
        let err = match strip_typescript_module("driver.ts", b"enum Status { Ready }\n") {
            Ok(_) => panic!("enum should fail"),
            Err(error) => error,
        };
        assert!(err.contains("unsupported TypeScript"));
        assert!(err.contains("enums"));
    }
}
