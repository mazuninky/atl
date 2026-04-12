//! User-defined alias management and pre-clap expansion.
//!
//! Aliases are plain `name -> shell-quoted expansion` mappings stored in the
//! TOML config file under `[aliases]`. They are expanded **before** clap sees
//! the argv: [`expand_aliases`] inspects `std::env::args()`, locates the first
//! non-flag token, and if it matches a user-defined alias (and is not a
//! built-in command), replaces it with `shlex::split(expansion)`.
//!
//! Expansion runs exactly once — a recursive pass would be a footgun
//! (`a -> b`, `b -> a` loops) for no real gain.

use std::collections::HashMap;

use camino::{Utf8Path, Utf8PathBuf};
use tracing::debug;

use crate::cli::args::{AliasCommand, AliasSubcommand};
use crate::config::{Config, ConfigLoader};
use crate::error::Error;
use crate::io::IoStreams;
use crate::output::{OutputFormat, Transforms, write_output};

/// Built-in top-level commands that must never be shadowed by a user alias.
///
/// Keep this list in sync with the `Command` enum in `src/cli/args/mod.rs`,
/// including hidden commands like `generate-docs` so power users who invoke
/// them cannot accidentally alias-shadow them.
pub(crate) const BUILTINS: &[&str] = &[
    "confluence",
    "conf",
    "c",
    "jira",
    "j",
    "config",
    "init",
    "completions",
    "self",
    "alias",
    "api",
    "auth",
    "browse",
    "generate-docs",
];

/// Global flags that take a value (either as `--flag value` or `--flag=value`).
///
/// The short-form alternatives (`-p`, `-F`) are handled separately by
/// [`find_subcommand_index`] but listed here so both forms stay in sync.
const VALUE_FLAGS_LONG: &[&str] = &["--config", "--profile", "--format", "--jq", "--template"];
const VALUE_FLAGS_SHORT: &[&str] = &["-p", "-F"];

/// Expand user-defined aliases in `argv` using the current config.
///
/// Best-effort: if the config cannot be loaded (missing, malformed, I/O
/// error) the input is returned unchanged. Called from `main.rs` before
/// `Cli::parse_from`.
///
/// If `--config <path>` (or `--config=<path>`) appears before the
/// subcommand, that path is honoured so aliases defined in an alternative
/// config file are resolved — without this, `atl --config alt.toml myq`
/// would expand `myq` from the default config instead.
#[must_use]
pub fn expand_aliases(argv: Vec<String>) -> Vec<String> {
    let explicit_config = extract_config_path(&argv);
    let config = match ConfigLoader::load(explicit_config.as_deref()) {
        Ok(Some(cfg)) => cfg,
        Ok(None) => return argv,
        Err(err) => {
            debug!("alias expansion: config load failed: {err}");
            return argv;
        }
    };
    expand_with_aliases(argv, &config.aliases)
}

/// Pre-clap scan for `--config <path>` / `--config=<path>` on `argv`, used
/// by [`expand_aliases`] so the alias map is loaded from whichever file
/// the user explicitly selected. Stops scanning at the first positional
/// token so values that happen to look like flags (e.g. `--config` in a
/// subcommand's own args) are never misinterpreted.
#[must_use]
fn extract_config_path(argv: &[String]) -> Option<Utf8PathBuf> {
    let mut i = 1;
    while i < argv.len() {
        let tok = argv[i].as_str();

        if tok == "--config" {
            return argv.get(i + 1).map(|s| Utf8PathBuf::from(s.as_str()));
        }
        if let Some(rest) = tok.strip_prefix("--config=") {
            return Some(Utf8PathBuf::from(rest));
        }

        // Skip past other long `--flag=value` tokens so we don't misread
        // them as the subcommand boundary.
        if let Some(rest) = tok.strip_prefix("--")
            && rest.contains('=')
        {
            i += 1;
            continue;
        }

        // Long value-taking flag: `--profile foo`, etc. Skip the value.
        if VALUE_FLAGS_LONG.contains(&tok) {
            i += 2;
            continue;
        }

        // Short value-taking flag: `-F json`, `-F=json`, `-Fjson`.
        if let Some(short) = VALUE_FLAGS_SHORT.iter().find(|&&f| {
            tok == f || tok.starts_with(&format!("{f}=")) || (tok.starts_with(f) && tok.len() > 2)
        }) {
            if tok == *short {
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }

        // Any other flag-looking token is a boolean global.
        if tok.starts_with('-') && tok != "-" {
            i += 1;
            continue;
        }

        // Positional token reached — `--config` did not precede it.
        return None;
    }
    None
}

/// Pure expansion step — takes an explicit alias map so tests can exercise
/// the logic without touching the real filesystem.
#[must_use]
pub fn expand_with_aliases(
    mut argv: Vec<String>,
    aliases: &HashMap<String, String>,
) -> Vec<String> {
    if aliases.is_empty() {
        return argv;
    }

    let Some(sub_idx) = find_subcommand_index(&argv) else {
        return argv;
    };

    let sub = &argv[sub_idx];
    if BUILTINS.contains(&sub.as_str()) {
        return argv;
    }

    let Some(expansion) = aliases.get(sub) else {
        return argv;
    };

    let Some(tokens) = shlex::split(expansion) else {
        debug!("alias '{sub}' has unparseable expansion: {expansion:?}");
        return argv;
    };

    // Replace argv[sub_idx] with `tokens`, preserving prefix and suffix.
    let suffix: Vec<String> = argv.drain(sub_idx + 1..).collect();
    argv.pop(); // discard the alias token itself
    argv.extend(tokens);
    argv.extend(suffix);
    argv
}

/// Walk `argv[1..]` and return the index of the first positional token
/// (i.e. the subcommand). Global flags and their values are skipped.
///
/// Returns `None` when `argv` is empty/contains only global flags.
#[must_use]
pub(crate) fn find_subcommand_index(argv: &[String]) -> Option<usize> {
    let mut i = 1;
    while i < argv.len() {
        let tok = argv[i].as_str();

        // `--foo=value` collapses into a single token — accept any long flag
        // of that shape and move on.
        if let Some(rest) = tok.strip_prefix("--")
            && let Some(eq) = rest.find('=')
        {
            let _ = eq;
            i += 1;
            continue;
        }

        // Long value-taking flag: `--config path`
        if VALUE_FLAGS_LONG.contains(&tok) {
            i += 2;
            continue;
        }

        // Short value-taking flag: `-F json`. Also support `-F=json` and the
        // glued `-Fjson` form (clap accepts these).
        if let Some(short) = VALUE_FLAGS_SHORT.iter().find(|&&f| {
            tok == f || tok.starts_with(&format!("{f}=")) || (tok.starts_with(f) && tok.len() > 2)
        }) {
            if tok == *short {
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }

        // Any other flag-looking token (`-v`, `-vv`, `--quiet`, `--no-color`,
        // etc.) is assumed to be a boolean global.
        if tok.starts_with('-') && tok != "-" {
            i += 1;
            continue;
        }

        return Some(i);
    }
    None
}

/// Dispatch for `atl alias ...`.
pub fn run(
    args: &AliasCommand,
    config_path: Option<&Utf8Path>,
    format: &OutputFormat,
    io: &mut IoStreams,
    transforms: &Transforms<'_>,
) -> anyhow::Result<()> {
    match &args.command {
        AliasSubcommand::Set(a) => set(config_path, &a.name, &a.expansion, format, io, transforms),
        AliasSubcommand::List => list(config_path, format, io, transforms),
        AliasSubcommand::Delete(a) => delete(config_path, &a.name, format, io, transforms),
    }
}

fn set(
    config_path: Option<&Utf8Path>,
    name: &str,
    expansion: &str,
    format: &OutputFormat,
    io: &mut IoStreams,
    transforms: &Transforms<'_>,
) -> anyhow::Result<()> {
    validate_name(name)?;
    if expansion.trim().is_empty() {
        return Err(Error::Config("alias expansion cannot be empty".into()).into());
    }

    // Load existing config (or start from defaults if none is present) so we
    // preserve profiles, defaults, and other aliases.
    let mut config = ConfigLoader::load(config_path)?.unwrap_or_default();

    config
        .aliases
        .insert(name.to_string(), expansion.to_string());

    let path = ConfigLoader::save(&config, config_path)?;

    let value = serde_json::json!({
        "action": "set",
        "name": name,
        "expansion": expansion,
        "path": path.to_string(),
    });
    write_output(value, format, io, transforms)
}

fn list(
    config_path: Option<&Utf8Path>,
    format: &OutputFormat,
    io: &mut IoStreams,
    transforms: &Transforms<'_>,
) -> anyhow::Result<()> {
    let config = ConfigLoader::load(config_path)?.unwrap_or_default();

    let mut entries: Vec<(&String, &String)> = config.aliases.iter().collect();
    entries.sort_by(|a, b| a.0.cmp(b.0));
    let items: Vec<serde_json::Value> = entries
        .into_iter()
        .map(|(name, expansion)| {
            serde_json::json!({
                "name": name,
                "expansion": expansion,
            })
        })
        .collect();
    write_output(serde_json::Value::Array(items), format, io, transforms)
}

fn delete(
    config_path: Option<&Utf8Path>,
    name: &str,
    format: &OutputFormat,
    io: &mut IoStreams,
    transforms: &Transforms<'_>,
) -> anyhow::Result<()> {
    let mut config: Config = ConfigLoader::load(config_path)?
        .ok_or_else(|| Error::Config("no config file found; run `atl init` first".into()))?;

    if config.aliases.remove(name).is_none() {
        return Err(Error::NotFound(format!("alias '{name}' not found")).into());
    }

    let path = ConfigLoader::save(&config, config_path)?;

    let value = serde_json::json!({
        "action": "deleted",
        "name": name,
        "path": path.to_string(),
    });
    write_output(value, format, io, transforms)
}

/// Reject names that would collide with built-ins, look like flags, or be
/// empty. Called from `set`.
pub(crate) fn validate_name(name: &str) -> anyhow::Result<()> {
    if name.is_empty() {
        return Err(Error::Config("alias name cannot be empty".into()).into());
    }
    if name.starts_with('-') {
        return Err(Error::Config(format!(
            "alias name '{name}' cannot start with '-' (would be parsed as a flag)"
        ))
        .into());
    }
    if BUILTINS.contains(&name) {
        return Err(Error::Config(format!(
            "alias name '{name}' collides with a built-in command"
        ))
        .into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| (*s).to_string()).collect()
    }

    // --- find_subcommand_index -------------------------------------------------

    #[test]
    fn find_sub_plain_command() {
        assert_eq!(
            find_subcommand_index(&argv(&["atl", "jira", "me"])),
            Some(1)
        );
    }

    #[test]
    fn find_sub_after_boolean_flag() {
        assert_eq!(
            find_subcommand_index(&argv(&["atl", "-v", "jira", "me"])),
            Some(2)
        );
    }

    #[test]
    fn find_sub_after_long_value_flag_separate() {
        assert_eq!(
            find_subcommand_index(&argv(&["atl", "--config", "x.toml", "jira"])),
            Some(3)
        );
    }

    #[test]
    fn find_sub_after_long_value_flag_equals() {
        assert_eq!(
            find_subcommand_index(&argv(&["atl", "--config=x.toml", "jira"])),
            Some(2)
        );
    }

    #[test]
    fn find_sub_after_short_value_flag_separate() {
        assert_eq!(
            find_subcommand_index(&argv(&["atl", "-F", "json", "jira"])),
            Some(3)
        );
    }

    #[test]
    fn find_sub_after_short_value_flag_equals() {
        assert_eq!(
            find_subcommand_index(&argv(&["atl", "-F=json", "jira"])),
            Some(2)
        );
    }

    #[test]
    fn find_sub_after_multiple_global_flags() {
        assert_eq!(
            find_subcommand_index(&argv(&[
                "atl",
                "-v",
                "--no-color",
                "-F",
                "json",
                "jira",
                "me",
            ])),
            Some(5)
        );
    }

    #[test]
    fn find_sub_empty_argv() {
        assert_eq!(find_subcommand_index(&argv(&["atl"])), None);
    }

    #[test]
    fn find_sub_only_flags() {
        assert_eq!(
            find_subcommand_index(&argv(&["atl", "-v", "--no-color"])),
            None
        );
    }

    // --- expand_with_aliases ---------------------------------------------------

    fn make_aliases<'a, I: IntoIterator<Item = (&'a str, &'a str)>>(
        pairs: I,
    ) -> HashMap<String, String> {
        pairs
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn expand_no_aliases_returns_unchanged() {
        let a = make_aliases([]);
        let input = argv(&["atl", "myq", "arg"]);
        assert_eq!(expand_with_aliases(input.clone(), &a), input);
    }

    #[test]
    fn expand_simple_alias() {
        let a = make_aliases([("myq", "jira me")]);
        let out = expand_with_aliases(argv(&["atl", "myq"]), &a);
        assert_eq!(out, argv(&["atl", "jira", "me"]));
    }

    #[test]
    fn expand_alias_with_quoted_args() {
        let a = make_aliases([("myq", r#"jira search "project=FOO""#)]);
        let out = expand_with_aliases(argv(&["atl", "myq"]), &a);
        assert_eq!(out, argv(&["atl", "jira", "search", "project=FOO"]));
    }

    #[test]
    fn expand_alias_preserves_trailing_args() {
        let a = make_aliases([("myq", "jira view")]);
        let out = expand_with_aliases(argv(&["atl", "myq", "PROJ-123"]), &a);
        assert_eq!(out, argv(&["atl", "jira", "view", "PROJ-123"]));
    }

    #[test]
    fn expand_builtin_name_never_expanded() {
        // Even if the user manages to sneak a builtin name into aliases
        // (shouldn't be possible through `set`, but be defensive) the builtin
        // wins.
        let a = make_aliases([("jira", "confluence read 123")]);
        let input = argv(&["atl", "jira", "me"]);
        assert_eq!(expand_with_aliases(input.clone(), &a), input);
    }

    #[test]
    fn expand_after_boolean_global_flag() {
        let a = make_aliases([("myq", "jira me")]);
        let out = expand_with_aliases(argv(&["atl", "-v", "myq"]), &a);
        assert_eq!(out, argv(&["atl", "-v", "jira", "me"]));
    }

    #[test]
    fn expand_after_value_taking_global_flag() {
        let a = make_aliases([("myq", "jira me")]);
        let out = expand_with_aliases(argv(&["atl", "-F", "json", "myq", "arg"]), &a);
        assert_eq!(out, argv(&["atl", "-F", "json", "jira", "me", "arg"]));
    }

    #[test]
    fn expand_unknown_command_without_matching_alias_unchanged() {
        let a = make_aliases([("other", "jira me")]);
        let input = argv(&["atl", "myq", "arg"]);
        assert_eq!(expand_with_aliases(input.clone(), &a), input);
    }

    #[test]
    fn expand_unparseable_shlex_returns_unchanged() {
        // An unterminated double-quote confuses shlex — we must not crash.
        let a = make_aliases([("myq", r#"jira search "unterminated"#)]);
        let input = argv(&["atl", "myq"]);
        assert_eq!(expand_with_aliases(input.clone(), &a), input);
    }

    #[test]
    fn expand_not_recursive() {
        // `foo -> bar`, `bar -> baz` — after expanding `foo` once we should
        // see `bar`, not `baz`. `bar` is not a builtin here so the key check
        // is that we don't re-run the expansion.
        let a = make_aliases([("foo", "bar"), ("bar", "baz")]);
        let out = expand_with_aliases(argv(&["atl", "foo"]), &a);
        assert_eq!(out, argv(&["atl", "bar"]));
    }

    // --- extract_config_path ---------------------------------------------------

    #[test]
    fn extract_config_separate_arg() {
        let out = extract_config_path(&argv(&["atl", "--config", "alt.toml", "myq"]));
        assert_eq!(out, Some(Utf8PathBuf::from("alt.toml")));
    }

    #[test]
    fn extract_config_equals_form() {
        let out = extract_config_path(&argv(&["atl", "--config=alt.toml", "myq"]));
        assert_eq!(out, Some(Utf8PathBuf::from("alt.toml")));
    }

    #[test]
    fn extract_config_none_when_absent() {
        let out = extract_config_path(&argv(&["atl", "myq", "arg"]));
        assert_eq!(out, None);
    }

    #[test]
    fn extract_config_after_other_global_flags() {
        // `-v`, `-F json`, then --config, then the subcommand.
        let out = extract_config_path(&argv(&[
            "atl", "-v", "-F", "json", "--config", "alt.toml", "myq",
        ]));
        assert_eq!(out, Some(Utf8PathBuf::from("alt.toml")));
    }

    #[test]
    fn extract_config_stops_at_subcommand() {
        // A `--config` that appears *after* the subcommand (as a subcommand
        // arg) must not be picked up as the global.
        let out = extract_config_path(&argv(&["atl", "myq", "--config", "not-a-global.toml"]));
        assert_eq!(out, None);
    }

    // --- validate_name ---------------------------------------------------------

    #[test]
    fn validate_name_empty_rejected() {
        assert!(validate_name("").is_err());
    }

    #[test]
    fn validate_name_builtin_rejected() {
        assert!(validate_name("jira").is_err());
        assert!(validate_name("confluence").is_err());
        assert!(validate_name("alias").is_err());
    }

    #[test]
    fn validate_name_flag_like_rejected() {
        assert!(validate_name("-x").is_err());
        assert!(validate_name("--foo").is_err());
    }

    #[test]
    fn validate_name_valid_ok() {
        assert!(validate_name("myq").is_ok());
        assert!(validate_name("pr-list").is_ok());
        assert!(validate_name("my_alias").is_ok());
    }
}
