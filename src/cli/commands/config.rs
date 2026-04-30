use camino::Utf8Path;

use crate::cli::args::{ConfigSetDefaultsArgs, ConfigSubcommand};
use crate::config::ConfigLoader;
use crate::io::IoStreams;
use crate::output::{OutputFormat, Transforms, write_output};

pub fn run(
    cmd: &ConfigSubcommand,
    config_path: Option<&Utf8Path>,
    format: &OutputFormat,
    io: &mut IoStreams,
    transforms: &Transforms<'_>,
) -> anyhow::Result<()> {
    match cmd {
        ConfigSubcommand::List => list(config_path, format, io, transforms),
        ConfigSubcommand::Show(args) => {
            show(config_path, args.name.as_deref(), format, io, transforms)
        }
        ConfigSubcommand::Delete(args) => delete(config_path, &args.name, format, io, transforms),
        ConfigSubcommand::SetDefault(args) => {
            set_default(config_path, &args.name, format, io, transforms)
        }
        ConfigSubcommand::SetDefaults(args) => {
            set_defaults(config_path, args, format, io, transforms)
        }
    }
}

fn list(
    config_path: Option<&Utf8Path>,
    format: &OutputFormat,
    io: &mut IoStreams,
    transforms: &Transforms<'_>,
) -> anyhow::Result<()> {
    let config = ConfigLoader::load(config_path)?
        .ok_or_else(|| anyhow::anyhow!("no config file found; run `atl init` first"))?;

    let mut names: Vec<_> = config.profiles.keys().collect();
    names.sort();

    let items: Vec<serde_json::Value> = names
        .iter()
        .map(|name| {
            serde_json::json!({
                "name": name,
                "is_default": *name == &config.default_profile,
            })
        })
        .collect();
    let value = serde_json::Value::Array(items);

    write_output(value, format, io, transforms)
}

fn show(
    config_path: Option<&Utf8Path>,
    name: Option<&str>,
    format: &OutputFormat,
    io: &mut IoStreams,
    transforms: &Transforms<'_>,
) -> anyhow::Result<()> {
    let config = ConfigLoader::load(config_path)?
        .ok_or_else(|| anyhow::anyhow!("no config file found; run `atl init` first"))?;

    let profile_name = name.unwrap_or(&config.default_profile);
    let profile = config
        .profiles
        .get(profile_name)
        .ok_or_else(|| anyhow::anyhow!("profile '{profile_name}' not found"))?;

    let value = serde_json::json!({
        "name": profile_name,
        "profile": serde_json::to_value(profile)?,
    });

    write_output(value, format, io, transforms)
}

fn delete(
    config_path: Option<&Utf8Path>,
    name: &str,
    format: &OutputFormat,
    io: &mut IoStreams,
    transforms: &Transforms<'_>,
) -> anyhow::Result<()> {
    let path = match config_path {
        Some(p) => p.to_path_buf(),
        None => ConfigLoader::resolve_config_path()?,
    };

    let mut config = ConfigLoader::load(Some(path.as_path()))?
        .ok_or_else(|| anyhow::anyhow!("no config file found"))?;

    if !config.profiles.contains_key(name) {
        anyhow::bail!("profile '{name}' not found");
    }

    if name == config.default_profile {
        anyhow::bail!("cannot delete the default profile '{name}'; change default_profile first");
    }

    config.profiles.remove(name);

    let content = toml::to_string_pretty(&config)?;
    std::fs::write(path.as_std_path(), content)?;

    let value = serde_json::json!({
        "action": "deleted",
        "profile": name,
    });

    write_output(value, format, io, transforms)
}

fn set_default(
    config_path: Option<&Utf8Path>,
    name: &str,
    format: &OutputFormat,
    io: &mut IoStreams,
    transforms: &Transforms<'_>,
) -> anyhow::Result<()> {
    let path = match config_path {
        Some(p) => p.to_path_buf(),
        None => ConfigLoader::resolve_config_path()?,
    };

    let mut config = ConfigLoader::load(Some(path.as_path()))?
        .ok_or_else(|| anyhow::anyhow!("no config file found"))?;

    if !config.profiles.contains_key(name) {
        anyhow::bail!("profile '{name}' not found");
    }

    config.default_profile = name.to_string();

    let content = toml::to_string_pretty(&config)?;
    std::fs::write(path.as_std_path(), content)?;

    let value = serde_json::json!({
        "action": "set_default",
        "profile": name,
    });

    write_output(value, format, io, transforms)
}

fn set_defaults(
    config_path: Option<&Utf8Path>,
    args: &ConfigSetDefaultsArgs,
    format: &OutputFormat,
    io: &mut IoStreams,
    transforms: &Transforms<'_>,
) -> anyhow::Result<()> {
    let path = match config_path {
        Some(p) => p.to_path_buf(),
        None => ConfigLoader::resolve_config_path()?,
    };

    let mut config = ConfigLoader::load(Some(path.as_path()))?
        .ok_or_else(|| anyhow::anyhow!("no config file found"))?;

    let profile_name = args.profile.as_deref().unwrap_or(&config.default_profile);
    let profile_name_owned = profile_name.to_string();
    let profile = config
        .profiles
        .get_mut(&profile_name_owned)
        .ok_or_else(|| anyhow::anyhow!("profile '{profile_name_owned}' not found"))?;

    if let Some(proj) = &args.project {
        profile.default_project = Some(proj.clone());
    }
    if let Some(space) = &args.space {
        profile.default_space = Some(space.clone());
    }
    let project = profile.default_project.clone();
    let space = profile.default_space.clone();

    let content = toml::to_string_pretty(&config)?;
    std::fs::write(path.as_std_path(), content)?;

    let value = serde_json::json!({
        "action": "updated",
        "profile": profile_name_owned,
        "project": project,
        "space": space,
    });

    write_output(value, format, io, transforms)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use camino::Utf8PathBuf;
    use tempfile::TempDir;

    use super::*;
    use crate::config::Config;

    // -- helpers ---------------------------------------------------------

    fn write_config(dir: &TempDir, contents: &str) -> Utf8PathBuf {
        let path = Utf8PathBuf::from_path_buf(dir.path().join("atl.toml"))
            .expect("temp dir path is valid utf8");
        std::fs::write(&path, contents).expect("write config");
        path
    }

    fn missing_path(dir: &TempDir) -> Utf8PathBuf {
        Utf8PathBuf::from_path_buf(dir.path().join("does-not-exist.toml"))
            .expect("temp dir path is valid utf8")
    }

    fn json_stdout(io: &IoStreams) -> serde_json::Value {
        let raw = io.stdout_as_string();
        serde_json::from_str(&raw)
            .unwrap_or_else(|e| panic!("stdout is not valid JSON: {e}\n--- stdout ---\n{raw}"))
    }

    fn sample_two_profile_config() -> &'static str {
        r#"
default_profile = "work"

[profiles.work]
default_project = "WRK"

[profiles.work.jira]
domain = "work.atlassian.net"
email = "alice@work.com"

[profiles.personal]
default_project = "HOME"

[profiles.personal.jira]
domain = "personal.atlassian.net"
"#
    }

    fn fmt() -> OutputFormat {
        OutputFormat::Json
    }

    fn tx() -> Transforms<'static> {
        Transforms::none()
    }

    fn set_defaults_args(
        profile: Option<&str>,
        project: Option<&str>,
        space: Option<&str>,
    ) -> ConfigSetDefaultsArgs {
        ConfigSetDefaultsArgs {
            profile: profile.map(String::from),
            project: project.map(String::from),
            space: space.map(String::from),
        }
    }

    // -- run() dispatcher ------------------------------------------------

    #[test]
    fn run_dispatches_to_list() {
        let dir = TempDir::new().unwrap();
        let path = write_config(&dir, sample_two_profile_config());
        let mut io = IoStreams::test();

        run(
            &ConfigSubcommand::List,
            Some(path.as_path()),
            &fmt(),
            &mut io,
            &tx(),
        )
        .unwrap();

        let parsed = json_stdout(&io);
        let arr = parsed.as_array().expect("list returns a JSON array");
        assert_eq!(arr.len(), 2, "expected 2 profiles, got {arr:?}");
    }

    #[test]
    fn run_dispatches_to_show() {
        let dir = TempDir::new().unwrap();
        let path = write_config(&dir, sample_two_profile_config());
        let mut io = IoStreams::test();

        run(
            &ConfigSubcommand::Show(crate::cli::args::ConfigShowArgs { name: None }),
            Some(path.as_path()),
            &fmt(),
            &mut io,
            &tx(),
        )
        .unwrap();

        let parsed = json_stdout(&io);
        assert_eq!(parsed["name"], serde_json::json!("work"));
    }

    // -- list ------------------------------------------------------------

    #[test]
    fn list_returns_profiles_with_default_marker() {
        let dir = TempDir::new().unwrap();
        let path = write_config(&dir, sample_two_profile_config());
        let mut io = IoStreams::test();

        list(Some(path.as_path()), &fmt(), &mut io, &tx()).unwrap();

        let parsed = json_stdout(&io);
        let arr = parsed.as_array().expect("array");
        // Sorted alphabetically: personal, work.
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["name"], serde_json::json!("personal"));
        assert_eq!(arr[0]["is_default"], serde_json::json!(false));
        assert_eq!(arr[1]["name"], serde_json::json!("work"));
        assert_eq!(arr[1]["is_default"], serde_json::json!(true));
    }

    #[test]
    fn list_sorts_profiles_alphabetically_for_determinism() {
        // Insert profiles in non-alphabetical order to prove the sort.
        let dir = TempDir::new().unwrap();
        let path = write_config(
            &dir,
            r#"
default_profile = "alpha"

[profiles.zulu]
[profiles.alpha]
[profiles.mike]
"#,
        );
        let mut io = IoStreams::test();

        list(Some(path.as_path()), &fmt(), &mut io, &tx()).unwrap();

        let parsed = json_stdout(&io);
        let names: Vec<&str> = parsed
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, vec!["alpha", "mike", "zulu"]);
    }

    #[test]
    fn list_errors_when_no_config_file() {
        let dir = TempDir::new().unwrap();
        let missing = missing_path(&dir);
        let mut io = IoStreams::test();

        let err = list(Some(missing.as_path()), &fmt(), &mut io, &tx()).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("not found") || msg.contains("no config file"),
            "expected 'not found' or 'no config file' in error, got: {msg}"
        );
    }

    // -- show ------------------------------------------------------------

    #[test]
    fn show_uses_default_profile_when_name_is_none() {
        let dir = TempDir::new().unwrap();
        let path = write_config(&dir, sample_two_profile_config());
        let mut io = IoStreams::test();

        show(Some(path.as_path()), None, &fmt(), &mut io, &tx()).unwrap();

        let parsed = json_stdout(&io);
        assert_eq!(parsed["name"], serde_json::json!("work"));
        assert_eq!(
            parsed["profile"]["default_project"],
            serde_json::json!("WRK")
        );
        assert_eq!(
            parsed["profile"]["jira"]["domain"],
            serde_json::json!("work.atlassian.net")
        );
    }

    #[test]
    fn show_returns_named_profile() {
        let dir = TempDir::new().unwrap();
        let path = write_config(&dir, sample_two_profile_config());
        let mut io = IoStreams::test();

        show(
            Some(path.as_path()),
            Some("personal"),
            &fmt(),
            &mut io,
            &tx(),
        )
        .unwrap();

        let parsed = json_stdout(&io);
        assert_eq!(parsed["name"], serde_json::json!("personal"));
        assert_eq!(
            parsed["profile"]["jira"]["domain"],
            serde_json::json!("personal.atlassian.net")
        );
    }

    #[test]
    fn show_errors_when_profile_missing() {
        let dir = TempDir::new().unwrap();
        let path = write_config(&dir, sample_two_profile_config());
        let mut io = IoStreams::test();

        let err = show(Some(path.as_path()), Some("ghost"), &fmt(), &mut io, &tx()).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("'ghost'") && msg.contains("not found"),
            "expected error to mention 'ghost' and 'not found', got: {msg}"
        );
    }

    #[test]
    fn show_errors_when_no_config_file() {
        let dir = TempDir::new().unwrap();
        let missing = missing_path(&dir);
        let mut io = IoStreams::test();

        let err = show(Some(missing.as_path()), None, &fmt(), &mut io, &tx()).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("not found") || msg.contains("no config file"),
            "expected 'not found' / 'no config file' in error, got: {msg}"
        );
    }

    // -- delete ----------------------------------------------------------

    #[test]
    fn delete_removes_non_default_profile_and_persists() {
        let dir = TempDir::new().unwrap();
        let path = write_config(&dir, sample_two_profile_config());
        let mut io = IoStreams::test();

        delete(Some(path.as_path()), "personal", &fmt(), &mut io, &tx()).unwrap();

        let parsed = json_stdout(&io);
        assert_eq!(parsed["action"], serde_json::json!("deleted"));
        assert_eq!(parsed["profile"], serde_json::json!("personal"));

        // Reload from disk and verify the profile is gone.
        let reloaded = ConfigLoader::load(Some(path.as_path()))
            .unwrap()
            .expect("config still present");
        assert!(!reloaded.profiles.contains_key("personal"));
        assert!(reloaded.profiles.contains_key("work"));
        assert_eq!(reloaded.default_profile, "work");
    }

    #[test]
    fn delete_errors_when_profile_missing() {
        let dir = TempDir::new().unwrap();
        let path = write_config(&dir, sample_two_profile_config());
        let mut io = IoStreams::test();

        let err = delete(Some(path.as_path()), "ghost", &fmt(), &mut io, &tx()).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("'ghost'") && msg.contains("not found"),
            "expected 'ghost' + 'not found' in error, got: {msg}"
        );
    }

    #[test]
    fn delete_refuses_to_remove_default_profile() {
        let dir = TempDir::new().unwrap();
        let path = write_config(&dir, sample_two_profile_config());
        let mut io = IoStreams::test();

        let err = delete(Some(path.as_path()), "work", &fmt(), &mut io, &tx()).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("cannot delete the default profile"),
            "expected default-profile guard, got: {msg}"
        );

        // Profile should still be on disk.
        let reloaded = ConfigLoader::load(Some(path.as_path()))
            .unwrap()
            .expect("config still present");
        assert!(reloaded.profiles.contains_key("work"));
    }

    #[test]
    fn delete_errors_when_no_config_file() {
        let dir = TempDir::new().unwrap();
        let missing = missing_path(&dir);
        let mut io = IoStreams::test();

        let err = delete(Some(missing.as_path()), "work", &fmt(), &mut io, &tx()).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("not found") || msg.contains("no config file"),
            "expected 'not found' / 'no config file' in error, got: {msg}"
        );
    }

    // -- set_default -----------------------------------------------------

    #[test]
    fn set_default_updates_default_and_persists() {
        let dir = TempDir::new().unwrap();
        let path = write_config(&dir, sample_two_profile_config());
        let mut io = IoStreams::test();

        set_default(Some(path.as_path()), "personal", &fmt(), &mut io, &tx()).unwrap();

        let parsed = json_stdout(&io);
        assert_eq!(parsed["action"], serde_json::json!("set_default"));
        assert_eq!(parsed["profile"], serde_json::json!("personal"));

        let reloaded = ConfigLoader::load(Some(path.as_path()))
            .unwrap()
            .expect("config still present");
        assert_eq!(reloaded.default_profile, "personal");
    }

    #[test]
    fn set_default_errors_when_profile_missing() {
        let dir = TempDir::new().unwrap();
        let path = write_config(&dir, sample_two_profile_config());
        let mut io = IoStreams::test();

        let err = set_default(Some(path.as_path()), "ghost", &fmt(), &mut io, &tx()).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("'ghost'") && msg.contains("not found"),
            "expected 'ghost' + 'not found' in error, got: {msg}"
        );

        // Default must not have changed.
        let reloaded = ConfigLoader::load(Some(path.as_path()))
            .unwrap()
            .expect("config still present");
        assert_eq!(reloaded.default_profile, "work");
    }

    #[test]
    fn set_default_errors_when_no_config_file() {
        let dir = TempDir::new().unwrap();
        let missing = missing_path(&dir);
        let mut io = IoStreams::test();

        let err = set_default(Some(missing.as_path()), "work", &fmt(), &mut io, &tx()).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("not found") || msg.contains("no config file"),
            "expected 'not found' / 'no config file' in error, got: {msg}"
        );
    }

    // -- set_defaults ----------------------------------------------------

    #[test]
    fn set_defaults_updates_project_on_default_profile() {
        let dir = TempDir::new().unwrap();
        let path = write_config(&dir, sample_two_profile_config());
        let mut io = IoStreams::test();
        let args = set_defaults_args(None, Some("NEW"), None);

        set_defaults(Some(path.as_path()), &args, &fmt(), &mut io, &tx()).unwrap();

        let parsed = json_stdout(&io);
        assert_eq!(parsed["action"], serde_json::json!("updated"));
        assert_eq!(parsed["profile"], serde_json::json!("work"));
        assert_eq!(parsed["project"], serde_json::json!("NEW"));
        // The work profile in the fixture has no default_space, so output is null.
        assert_eq!(parsed["space"], serde_json::Value::Null);

        let reloaded = ConfigLoader::load(Some(path.as_path()))
            .unwrap()
            .expect("config still present");
        let profile = reloaded.profiles.get("work").expect("work profile");
        assert_eq!(profile.default_project.as_deref(), Some("NEW"));
        assert_eq!(profile.default_space, None);
    }

    #[test]
    fn set_defaults_updates_space_on_default_profile() {
        let dir = TempDir::new().unwrap();
        let path = write_config(&dir, sample_two_profile_config());
        let mut io = IoStreams::test();
        let args = set_defaults_args(None, None, Some("DOCS"));

        set_defaults(Some(path.as_path()), &args, &fmt(), &mut io, &tx()).unwrap();

        let parsed = json_stdout(&io);
        assert_eq!(parsed["space"], serde_json::json!("DOCS"));
        // Existing default_project must be preserved.
        assert_eq!(parsed["project"], serde_json::json!("WRK"));

        let reloaded = ConfigLoader::load(Some(path.as_path()))
            .unwrap()
            .expect("config still present");
        let profile = reloaded.profiles.get("work").expect("work profile");
        assert_eq!(profile.default_space.as_deref(), Some("DOCS"));
        assert_eq!(profile.default_project.as_deref(), Some("WRK"));
    }

    #[test]
    fn set_defaults_updates_named_profile() {
        let dir = TempDir::new().unwrap();
        let path = write_config(&dir, sample_two_profile_config());
        let mut io = IoStreams::test();
        let args = set_defaults_args(Some("personal"), Some("PNEW"), Some("PSP"));

        set_defaults(Some(path.as_path()), &args, &fmt(), &mut io, &tx()).unwrap();

        let parsed = json_stdout(&io);
        assert_eq!(parsed["profile"], serde_json::json!("personal"));
        assert_eq!(parsed["project"], serde_json::json!("PNEW"));
        assert_eq!(parsed["space"], serde_json::json!("PSP"));

        let reloaded = ConfigLoader::load(Some(path.as_path()))
            .unwrap()
            .expect("config still present");
        let profile = reloaded.profiles.get("personal").expect("personal profile");
        assert_eq!(profile.default_project.as_deref(), Some("PNEW"));
        assert_eq!(profile.default_space.as_deref(), Some("PSP"));
        // Default profile should not have been touched.
        let work = reloaded.profiles.get("work").expect("work profile");
        assert_eq!(work.default_project.as_deref(), Some("WRK"));
        assert_eq!(work.default_space, None);
    }

    #[test]
    fn set_defaults_errors_when_profile_missing() {
        let dir = TempDir::new().unwrap();
        let path = write_config(&dir, sample_two_profile_config());
        let mut io = IoStreams::test();
        let args = set_defaults_args(Some("ghost"), Some("X"), None);

        let err = set_defaults(Some(path.as_path()), &args, &fmt(), &mut io, &tx()).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("'ghost'") && msg.contains("not found"),
            "expected 'ghost' + 'not found' in error, got: {msg}"
        );
    }

    #[test]
    fn set_defaults_errors_when_no_config_file() {
        let dir = TempDir::new().unwrap();
        let missing = missing_path(&dir);
        let mut io = IoStreams::test();
        let args = set_defaults_args(None, Some("X"), None);

        let err = set_defaults(Some(missing.as_path()), &args, &fmt(), &mut io, &tx()).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("not found") || msg.contains("no config file"),
            "expected 'not found' / 'no config file' in error, got: {msg}"
        );
    }

    #[test]
    fn set_defaults_with_no_field_args_is_a_noop_round_trip() {
        // The clap-level ArgGroup forces the user to pass at least one of
        // --project / --space, so this case never occurs at runtime. Calling
        // the handler directly with both fields None proves that, when nothing
        // is provided, the existing profile fields are preserved verbatim and
        // the action still reports "updated" (no separate guard exists in the
        // handler — that contract lives in clap).
        let dir = TempDir::new().unwrap();
        let path = write_config(&dir, sample_two_profile_config());
        let mut io = IoStreams::test();
        let args = set_defaults_args(None, None, None);

        set_defaults(Some(path.as_path()), &args, &fmt(), &mut io, &tx()).unwrap();

        let parsed = json_stdout(&io);
        assert_eq!(parsed["action"], serde_json::json!("updated"));
        assert_eq!(parsed["profile"], serde_json::json!("work"));
        assert_eq!(parsed["project"], serde_json::json!("WRK"));
        assert_eq!(parsed["space"], serde_json::Value::Null);

        // On-disk profile fields are unchanged.
        let reloaded = ConfigLoader::load(Some(path.as_path()))
            .unwrap()
            .expect("config still present");
        let profile = reloaded.profiles.get("work").expect("work profile");
        assert_eq!(profile.default_project.as_deref(), Some("WRK"));
        assert_eq!(profile.default_space, None);
    }

    // -- guard against accidental Config::default() drift ----------------

    #[test]
    fn helper_sample_config_parses_to_expected_shape() {
        // Sanity check that the fixture string we pass into all the tests
        // really does deserialize as expected — keeps the tests honest if the
        // Profile struct ever grows new required fields.
        let dir = TempDir::new().unwrap();
        let path = write_config(&dir, sample_two_profile_config());
        let cfg = ConfigLoader::load(Some(path.as_path()))
            .unwrap()
            .expect("fixture loads");
        assert_eq!(cfg.default_profile, "work");
        assert_eq!(cfg.profiles.len(), 2);
        let names: Vec<&String> = {
            let mut v: Vec<&String> = cfg.profiles.keys().collect();
            v.sort();
            v
        };
        assert_eq!(names, vec!["personal", "work"]);

        // Suppress the unused-import warning for HashMap / Config when the
        // test below is the only consumer. Cheap proof that Config builds.
        let _: Config = Config {
            default_profile: "x".to_string(),
            profiles: HashMap::new(),
            aliases: HashMap::new(),
        };
    }
}
