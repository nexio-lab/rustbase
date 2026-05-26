//! TypeScript → JavaScript stripping for hook files.
//!
//! Hooks landing on disk as `*.ts` go through `transpile()` before
//! the rquickjs runtime sees them. We use swc's TypeScript pass to
//! erase type annotations, interfaces, type aliases, `as` casts,
//! non-null `!` assertions, etc. — everything that isn't valid JS
//! is removed, the executable code stays.
//!
//! Out of scope on this branch: target downleveling (we emit
//! whatever ECMA version the input was — QuickJS handles modern
//! syntax fine), JSX, decorators with metadata, tsconfig.json
//! support. None of those are needed for hook scripts.

use crate::RuntimeError;
use swc_common::{FileName, Globals, Mark, SourceMap, comments::SingleThreadedComments, sync::Lrc, GLOBALS};
use swc_ecma_ast::{EsVersion, Pass};
use swc_ecma_codegen::{Config as CodegenConfig, Emitter, text_writer::JsWriter};
use swc_ecma_parser::{Parser, StringInput, Syntax, TsSyntax, lexer::Lexer};
use swc_ecma_transforms_base::resolver;
use swc_ecma_transforms_typescript::typescript;

/// Strip TypeScript from `src`, returning equivalent JavaScript.
pub fn transpile(src: &str) -> Result<String, RuntimeError> {
    let globals = Globals::default();
    GLOBALS.set(&globals, || {
        let cm: Lrc<SourceMap> = Default::default();
        let comments = SingleThreadedComments::default();
        let fm = cm.new_source_file(Lrc::new(FileName::Anon), src.to_string());

        let lexer = Lexer::new(
            Syntax::Typescript(TsSyntax::default()),
            EsVersion::EsNext,
            StringInput::from(&*fm),
            Some(&comments),
        );
        let mut parser = Parser::new_from(lexer);
        let mut program = parser
            .parse_program()
            .map_err(|e| RuntimeError::Js(format!("ts parse: {e:?}")))?;

        let unresolved_mark = Mark::new();
        let top_level_mark = Mark::new();

        resolver(unresolved_mark, top_level_mark, true).process(&mut program);
        typescript(
            Default::default(),
            unresolved_mark,
            top_level_mark,
        )
        .process(&mut program);

        let mut buf = Vec::with_capacity(src.len());
        {
            let writer = JsWriter::new(cm.clone(), "\n", &mut buf, None);
            let mut emitter = Emitter {
                cfg: CodegenConfig::default(),
                cm: cm.clone(),
                comments: Some(&comments),
                wr: writer,
            };
            emitter
                .emit_program(&program)
                .map_err(|e| RuntimeError::Js(format!("ts emit: {e}")))?;
        }
        String::from_utf8(buf)
            .map_err(|e| RuntimeError::Js(format!("ts emit utf-8: {e}")))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_param_type_annotations() {
        let js = transpile("function add(a: number, b: number): number { return a + b; }").unwrap();
        assert!(!js.contains(": number"), "got: {js}");
        assert!(js.contains("function add(a, b)"), "got: {js}");
        assert!(js.contains("return a + b;"), "got: {js}");
    }

    #[test]
    fn drops_interfaces_and_type_aliases() {
        let js = transpile(
            r#"
            interface Cat { name: string }
            type Pair = [number, number];
            const tom: Cat = { name: "tom" };
            "#,
        )
        .unwrap();
        assert!(!js.contains("interface"));
        assert!(!js.contains("type Pair"));
        assert!(js.contains("const tom = {"), "got: {js}");
    }

    #[test]
    fn strips_as_casts_and_non_null_bangs() {
        // swc preserves the parens that originally wrapped the cast expression;
        // the type annotations themselves (`as any`, `!`) are what we want gone.
        let js =
            transpile(r#"const id = ($app.request as any).auth!.id;"#).unwrap();
        assert!(!js.contains(" as "), "got: {js}");
        assert!(!js.contains("!."), "got: {js}");
        assert!(js.contains(".auth.id"), "got: {js}");
        assert!(js.contains("$app.request"), "got: {js}");
    }

    #[test]
    fn parse_error_surfaces_as_js_error() {
        let err = transpile("function broken(: { ").unwrap_err();
        assert!(matches!(err, RuntimeError::Js(_)));
    }

    #[test]
    fn plain_js_passes_through_unchanged_in_spirit() {
        // emit may reformat (semicolons, spacing) but the executable
        // shape should still match a basic structural check.
        let js = transpile("const x = 1; const y = x + 2;").unwrap();
        assert!(js.contains("const x = 1"));
        assert!(js.contains("const y = x + 2"));
    }
}
