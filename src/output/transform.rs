//! Post-processing pipeline for command output.
//!
//! Implements the `--jq` and `--template` global flags. A command produces
//! a `serde_json::Value`; this module runs it through an optional jq
//! filter, then an optional minijinja template, and yields either another
//! JSON value or a rendered text string for the reporter layer to emit.
//!
//! The pipeline is intentionally minimal:
//! 1. jq runs first, so templates see the filtered shape.
//! 2. An empty jq stream collapses to `Value::Null` which the writer treats
//!    as "print nothing".
//! 3. Templates are rendered from the current value as the root context, so
//!    `{{ foo.bar }}` works without any wrapping.
//!
//! jq is provided by the jaq 2.x family:
//! - `jaq-core = "2"` — compiler + runtime.
//! - `jaq-std = "2"` — .jq standard library (definitions like `map`).
//! - `jaq-json = "1"` — value type + native filters (`length`, `keys`, …).
//!
//! jaq 3.x exists but was not yet compatible with the 2.x core API surface
//! used elsewhere; we pinned to 2.x so that core, std and json line up.

use anyhow::{Context, Result};
use jaq_core::load::{Arena, File, Loader};
use jaq_core::{Compiler, Ctx, Native, RcIter};
use jaq_json::Val;
use minijinja::{Environment, Value as MjValue};
use serde_json::Value;

use crate::error::Error;

/// Optional post-processing transforms attached to the current invocation.
///
/// Borrowed from the parsed CLI so the struct is zero-copy and cheap to
/// thread through every command handler.
#[derive(Debug, Clone, Copy)]
pub struct Transforms<'a> {
    /// A jq expression applied to the command's JSON result.
    pub jq: Option<&'a str>,
    /// A minijinja template rendered against the (possibly jq-filtered)
    /// result. Takes precedence over the `--format` flag — whenever a
    /// template is set, the output is text.
    pub template: Option<&'a str>,
}

impl<'a> Transforms<'a> {
    /// An empty [`Transforms`] — both filters disabled.
    #[must_use]
    pub fn none() -> Self {
        Self {
            jq: None,
            template: None,
        }
    }

    /// Returns true when neither `--jq` nor `--template` is set, so the
    /// writer can skip the transform pipeline entirely.
    #[must_use]
    pub fn is_noop(&self) -> bool {
        self.jq.is_none() && self.template.is_none()
    }
}

/// Result of running the transform pipeline.
///
/// `Json` values are handed off to the selected reporter for rendering;
/// `Text` values bypass the reporter entirely and are written verbatim.
#[derive(Debug, Clone)]
pub enum Transformed {
    /// A JSON value that should still be rendered by the reporter.
    Json(Value),
    /// A rendered string that should be emitted directly.
    Text(String),
}

/// Runs the transform pipeline on `value`.
///
/// Rules, in order:
/// 1. When `--jq` is set, compile and execute the expression against
///    `value`. A zero-value result stream becomes `Value::Null`, a single
///    value replaces the current value unchanged, and multiple values are
///    wrapped as a JSON array so downstream transforms still see a single
///    JSON root.
/// 2. When `--template` is set, render the (possibly jq-filtered) value as
///    the template's root context and return [`Transformed::Text`].
/// 3. Otherwise return [`Transformed::Json`] of the current value.
///
/// Any jq parse/compile/runtime error is returned as an `anyhow::Error`
/// (exit code 1 via the existing error mapping). Template errors are wrapped
/// in [`Error::Template`] so they surface with the same exit code but a
/// dedicated message prefix.
pub fn apply(value: Value, t: &Transforms<'_>) -> Result<Transformed> {
    let current = match t.jq {
        Some(expr) => run_jq(expr, value)?,
        None => value,
    };

    if let Some(tmpl) = t.template {
        let rendered = render_template(tmpl, &current)?;
        return Ok(Transformed::Text(rendered));
    }

    Ok(Transformed::Json(current))
}

/// Compiles and runs a jq expression against a single JSON value, collapsing
/// the result stream into a single [`Value`] per the P3 plan:
///
/// - 0 values → `Value::Null`
/// - 1 value → that value
/// - N values → `Value::Array`
fn run_jq(expr: &str, input: Value) -> Result<Value> {
    // Parse + load the expression. The loader's static library is jaq-std's
    // .jq definitions — `map`, `join`, `flatten`, …
    let arena = Arena::default();
    let loader = Loader::new(jaq_std::defs());
    let modules = loader
        .load(
            &arena,
            File {
                path: (),
                code: expr,
            },
        )
        .map_err(|e| anyhow::anyhow!("jq parse error: {}", format_load_errors(&e)))?;

    // Compile with the native filter tables from jaq-std (math, regex,
    // time, …) **and** jaq-json (length, keys, type, …). Both must be
    // present or simple filters like `length` fail to resolve.
    let filter = Compiler::<_, Native<Val>>::default()
        .with_funs(jaq_std::funs().chain(jaq_json::funs()))
        .compile(modules)
        .map_err(|e| anyhow::anyhow!("jq compile error: {e:?}"))?;

    // The runtime needs an `Inputs` stream; we never feed extra inputs, so
    // we hand it an empty iterator.
    let inputs = RcIter::new(core::iter::empty());
    let ctx = Ctx::new([], &inputs);
    let val: Val = input.into();

    let mut results: Vec<Value> = Vec::new();
    for item in filter.run((ctx, val)) {
        let v = item.map_err(|e| anyhow::anyhow!("jq runtime error: {e}"))?;
        results.push(Value::from(v));
    }

    Ok(match results.len() {
        0 => Value::Null,
        1 => results.into_iter().next().expect("len checked"),
        _ => Value::Array(results),
    })
}

/// Joins the per-file error payload returned by jaq's loader into something
/// readable. jaq's error types don't implement `Display`, so we render them
/// with `Debug` and trim the outermost wrapper. This is only used to build
/// an error message — the exact text is best-effort.
fn format_load_errors<E: core::fmt::Debug>(err: &E) -> String {
    format!("{err:?}")
}

/// Renders a minijinja template against `value`.
///
/// The template's context exposes the value in two ways so both object
/// and scalar shapes render naturally:
///
/// - If `value` is an object, its keys are flattened into the root scope
///   so `{{ key }}` works directly on `{"key": "hi"}`.
/// - In all cases the value is also bound to a single top-level variable
///   named `this`, so scalar/array shapes can reference the whole value
///   (`{{ this }}`, `{% for i in this %}…{% endfor %}`).
///
/// Any parse or render error is mapped to [`Error::Template`].
fn render_template(template: &str, value: &Value) -> Result<String> {
    let mut env = Environment::new();
    env.add_template("atl-inline", template)
        .map_err(|e| Error::Template(format!("{e}")))
        .context("invalid template")?;

    let ctx = build_template_context(value);
    let tmpl = env
        .get_template("atl-inline")
        .map_err(|e| Error::Template(format!("{e}")))
        .context("template lookup failed")?;

    tmpl.render(ctx)
        .map_err(|e| Error::Template(format!("{e}")))
        .context("template render failed")
}

/// Builds the minijinja root context for a JSON value.
///
/// Object keys are flattened into the root scope; the whole value is also
/// bound to `this`. Scalars and arrays are reachable only through `this`.
///
/// Only the common `{"foo": ...}` shape gets flattened. Non-object values
/// (scalars, arrays) would otherwise have no accessible root reference in
/// minijinja, so `this` is always present.
fn build_template_context(value: &Value) -> MjValue {
    let this = MjValue::from_serialize(value);
    let this_ctx = minijinja::context! { this => this };
    match value {
        Value::Object(_) => {
            let flattened = MjValue::from_serialize(value);
            // In minijinja's `context!` macro, the *first* spread wins on
            // key collisions. Putting `this_ctx` first therefore guarantees
            // that `{{ this }}` always resolves to the whole input, even
            // if the object happens to contain a user-supplied key named
            // `this`. All other object keys pass through from `flattened`.
            minijinja::context! { ..this_ctx, ..flattened }
        }
        _ => this_ctx,
    }
}

// -------------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn jq_only(expr: &str) -> Transforms<'_> {
        Transforms {
            jq: Some(expr),
            template: None,
        }
    }

    fn tmpl_only(t: &str) -> Transforms<'_> {
        Transforms {
            jq: None,
            template: Some(t),
        }
    }

    fn json_result(t: Transformed) -> Value {
        match t {
            Transformed::Json(v) => v,
            Transformed::Text(s) => panic!("expected Json, got Text({s:?})"),
        }
    }

    fn text_result(t: Transformed) -> String {
        match t {
            Transformed::Text(s) => s,
            Transformed::Json(v) => panic!("expected Text, got Json({v:?})"),
        }
    }

    #[test]
    fn noop_transform_returns_value_unchanged() {
        let input = json!({"hello": "world"});
        let out = apply(input.clone(), &Transforms::none()).unwrap();
        assert_eq!(json_result(out), input);
    }

    #[test]
    fn is_noop_reports_correctly() {
        assert!(Transforms::none().is_noop());
        assert!(!jq_only(".foo").is_noop());
        assert!(!tmpl_only("{{ x }}").is_noop());
    }

    #[test]
    fn jq_projects_single_field() {
        let out = apply(json!({"foo": 42}), &jq_only(".foo")).unwrap();
        assert_eq!(json_result(out), json!(42));
    }

    #[test]
    fn jq_multi_value_stream_wraps_as_array() {
        let out = apply(
            json!({"items": [{"name": "a"}, {"name": "b"}]}),
            &jq_only(".items[].name"),
        )
        .unwrap();
        assert_eq!(json_result(out), json!(["a", "b"]));
    }

    #[test]
    fn jq_length_native_function() {
        let out = apply(json!([1, 2, 3]), &jq_only("length")).unwrap();
        assert_eq!(json_result(out), json!(3));
    }

    #[test]
    fn jq_empty_stream_becomes_null() {
        let out = apply(json!({"anything": true}), &jq_only("empty")).unwrap();
        assert_eq!(json_result(out), Value::Null);
    }

    #[test]
    fn jq_parse_error_is_surfaced() {
        let err = apply(json!({}), &jq_only(".invalid(")).unwrap_err();
        assert!(
            err.to_string().to_ascii_lowercase().contains("jq"),
            "expected jq error prefix, got: {err}"
        );
    }

    #[test]
    fn jq_map_from_std_library() {
        // `map` is defined in jaq-std's defs.jq — confirms the .jq stdlib
        // is wired into the loader.
        let out = apply(json!([1, 2, 3]), &jq_only("map(. + 1)")).unwrap();
        assert_eq!(json_result(out), json!([2, 3, 4]));
    }

    #[test]
    fn template_renders_object_field() {
        let out = apply(json!({"key": "hi"}), &tmpl_only("{{ key }}")).unwrap();
        assert_eq!(text_result(out), "hi");
    }

    #[test]
    fn template_iterates_array_with_builtin_loop() {
        let value = json!({"items": [{"name": "a"}, {"name": "b"}]});
        let out = apply(
            value,
            &tmpl_only("{% for i in items %}{{ i.name }}{% endfor %}"),
        )
        .unwrap();
        assert_eq!(text_result(out), "ab");
    }

    #[test]
    fn template_after_jq_combines_both_filters() {
        let value = json!({"items": [{"name": "a"}, {"name": "b"}]});
        let t = Transforms {
            jq: Some(".items"),
            template: Some("{% for i in this %}{{ i.name }}{% endfor %}"),
        };
        let out = apply(value, &t).unwrap();
        assert_eq!(text_result(out), "ab");
    }

    #[test]
    fn template_parse_error_is_surfaced_as_template_error() {
        let err = apply(json!({"x": 1}), &tmpl_only("{{ ")).unwrap_err();
        // Root cause must be Error::Template so the error-to-exit-code
        // mapping in src/error.rs surfaces it with the dedicated prefix.
        let template_err = err
            .chain()
            .find_map(|e| e.downcast_ref::<crate::error::Error>())
            .expect("expected Error::Template in the anyhow chain");
        assert!(
            matches!(template_err, crate::error::Error::Template(_)),
            "expected Error::Template, got: {template_err:?}"
        );
    }

    #[test]
    fn template_numeric_scalar_renders_via_this() {
        let out = apply(json!(42), &tmpl_only("value={{ this }}")).unwrap();
        assert_eq!(text_result(out), "value=42");
    }
}
