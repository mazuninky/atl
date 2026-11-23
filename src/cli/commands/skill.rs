//! `atl skill` — install, update, inspect and remove the agent skill.
//!
//! Every subcommand reports one object through [`write_output`], with every
//! key always there and none null, so that each output format can hold it:
//! what does not apply is an empty string or list.
//!
//! # Exit codes
//!
//! 0 — done, or nothing installed to work on; 5 — refused
//! ([`Refusal`] as `Error::InvalidInput`, nothing written); 1 — a file could
//! not be read or written, or another atl held the lock for longer than
//! 10 seconds.

use serde_json::{Value, json};

use crate::cli::args::{SkillCommand, SkillSubcommand};
use crate::error::Error;
use crate::io::IoStreams;
use crate::output::{OutputFormat, Transforms, write_output};
use crate::skill::{
    self, Edition, Environment, LinkState, Made, Refusal, RemoveReport, SkillError, Status, Target,
    WriteReport, one_line,
};

/// Run `atl skill` in `env`, with the skill of `edition` (the embedded one,
/// but in a test).
///
/// # Errors
///
/// [`Error::InvalidInput`] for a refusal; [`skill::FileError`] when a file
/// cannot be read or written; any error of the output.
pub fn run(
    cmd: &SkillCommand,
    env: &Environment,
    edition: &Edition,
    format: &OutputFormat,
    io: &mut IoStreams,
    transforms: &Transforms<'_>,
) -> anyhow::Result<()> {
    let value = value(&cmd.command, env, edition).map_err(into_anyhow)?;
    write_output(value, format, io, transforms)
}

/// What the subcommand did, as the object it reports.
fn value(
    command: &SkillSubcommand,
    env: &Environment,
    edition: &Edition,
) -> Result<Value, SkillError> {
    Ok(match command {
        SkillSubcommand::Install(args) => {
            let report = skill::install(&target(env, args.project)?, edition, args.force)?;
            write_value(&report)
        }
        SkillSubcommand::Update(args) => {
            let target = target(env, args.project)?;
            match skill::update(&target, edition, args.force)? {
                Some(report) => write_value(&report),
                None => not_installed(&target),
            }
        }
        SkillSubcommand::Remove(args) => {
            let report = skill::remove(&target(env, args.project)?, edition, args.force)?;
            remove_value(&report)
        }
        SkillSubcommand::Status(args) => status_value(
            &skill::status(&target(env, args.project)?, edition)?,
            edition,
        ),
    })
}

/// A refusal is invalid input (exit 5), in one line; anything else is a
/// runtime error (exit 1).
fn into_anyhow(err: SkillError) -> anyhow::Error {
    match err {
        SkillError::Refused(refusal) => Error::InvalidInput(one_line(&refusal.to_string())).into(),
        SkillError::File(err) => err.into(),
    }
}

/// The skill the command is about.
fn target(env: &Environment, project: bool) -> Result<Target, Refusal> {
    if project { env.project() } else { env.global() }
}

/// What `install` and `update` report.
///
/// `action` is `installed` (there was none of atl's), `updated` (from
/// another version), `restored` (the same version, with files put back) or
/// `up_to_date` (nothing changed); `link_action` is `created`, `replaced`,
/// `unchanged`, or empty where no link is wanted or `update` ran.
fn write_value(report: &WriteReport) -> Value {
    let action = match &report.previous {
        _ if !report.changed => "up_to_date",
        None => "installed",
        Some(previous) if *previous == report.version => "restored",
        Some(_) => "updated",
    };
    let (link, link_action) = match &report.link {
        Some((place, made)) => (
            place.as_str(),
            match made {
                Made::Nothing => "unchanged",
                Made::Created => "created",
                Made::Replaced => "replaced",
            },
        ),
        None => ("", ""),
    };
    json!({
        "path": report.dir.as_str(),
        "action": action,
        "version": report.version,
        "previous": report.previous.as_deref().unwrap_or_default(),
        "kept": report.kept,
        "link": link,
        "link_action": link_action,
    })
}

/// What `update` reports where atl installed no skill.
fn not_installed(target: &Target) -> Value {
    json!({
        "path": target.dir.as_str(),
        "action": "not_installed",
        "version": "",
        "previous": "",
        "kept": [],
        "link": "",
        "link_action": "",
    })
}

/// What `remove` reports: `action` is `removed` or `not_installed`; `link`
/// the link it removed; `left` the files that are not atl's when the
/// directory stays, `other` whatever else keeps it (empty directories, what
/// a system or a write that was cut short leaves) when there are none.
fn remove_value(report: &RemoveReport) -> Value {
    json!({
        "path": report.dir.as_str(),
        "action": if report.version.is_some() { "removed" } else { "not_installed" },
        "version": report.version.as_deref().unwrap_or_default(),
        "link": report.link.as_ref().map(|link| link.as_str()).unwrap_or_default(),
        "dir_kept": report.dir_kept,
        "left": report.left,
        "other": report.other,
    })
}

/// The object `atl skill status` reports.
fn status_value(status: &Status, edition: &Edition) -> Value {
    let (place, link) = match &status.link {
        Some((place, state)) => (place.as_str(), link_name(state)),
        None => ("", "not_wanted"),
    };
    json!({
        "path": status.dir.as_str(),
        "installed": status.state.is_installed(),
        "state": status.state.name(),
        "version": status.version.as_deref().unwrap_or_default(),
        "atl_version": edition.version,
        "edited": status.edited,
        "missing": status.missing,
        "colliding": status.colliding,
        "in_the_way": status.in_the_way,
        "unlisted": status.unlisted,
        "link": place,
        "link_state": link,
    })
}

/// The name of a link's state in the output of `atl skill status`.
fn link_name(state: &LinkState) -> &'static str {
    match state {
        LinkState::Ours | LinkState::Alike | LinkState::Shared => "ok",
        LinkState::Missing => "missing",
        LinkState::Elsewhere(_) => "elsewhere",
        LinkState::Taken | LinkState::Blocked(_) => "taken",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::args::{Cli, Command};
    use crate::error::{exit_code, exit_code_for_error};
    use clap::Parser;
    use pretty_assertions::assert_eq;

    /// `atl skill ARGS -F json` in `env`: the object it reports.
    fn atl(args: &[&str], env: &Environment) -> anyhow::Result<Value> {
        let argv = ["atl", "-F", "json", "skill"].iter().chain(args);
        let cli = Cli::try_parse_from(argv).expect("arguments");
        let Command::Skill(cmd) = &cli.command else {
            panic!("not `atl skill`");
        };
        let mut io = IoStreams::test();
        run(
            cmd,
            env,
            &Edition::embedded(),
            &cli.format,
            &mut io,
            &Transforms::none(),
        )?;
        Ok(serde_json::from_str(&io.stdout_as_string()).expect("JSON"))
    }

    fn home() -> (tempfile::TempDir, camino::Utf8PathBuf, Environment) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let home = camino::Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).expect("UTF-8");
        let env = Environment {
            home: Some(home.as_str().into()),
            current_dir: Some(home.join("project")),
            ..Environment::default()
        };
        (tmp, home, env)
    }

    #[test]
    fn every_subcommand_reports_an_object_of_a_fixed_shape() {
        let (_tmp, home, env) = home();
        let dir = home.join(".agents/skills/atl");
        let version = env!("CARGO_PKG_VERSION");
        let status = |state: &str, installed: bool, v: &str| {
            json!({
                "path": dir.as_str(), "installed": installed, "state": state, "version": v,
                "atl_version": version, "edited": [], "missing": [], "colliding": [],
                "in_the_way": [], "unlisted": [], "link": "", "link_state": "not_wanted",
            })
        };
        assert_eq!(
            atl(&["status"], &env).expect("status"),
            status("not_installed", false, "")
        );
        let write = |action: &str, v: &str| {
            json!({
                "path": dir.as_str(), "action": action, "version": v, "previous": "",
                "kept": [], "link": "", "link_action": "",
            })
        };
        assert_eq!(
            atl(&["update"], &env).expect("update"),
            write("not_installed", "")
        );
        assert_eq!(
            atl(&["install"], &env).expect("install"),
            write("installed", version)
        );
        assert_eq!(
            atl(&["status"], &env).expect("status"),
            status("up_to_date", true, version)
        );
        let mut again = write("up_to_date", version);
        again["previous"] = json!(version);
        assert_eq!(atl(&["update"], &env).expect("update"), again);
        assert_eq!(
            atl(&["remove"], &env).expect("remove"),
            json!({
                "path": dir.as_str(), "action": "removed", "version": version, "link": "",
                "dir_kept": false, "left": [], "other": [],
            })
        );
        assert!(!dir.exists());
        assert_eq!(
            atl(&["remove"], &env).expect("remove")["action"],
            "not_installed"
        );
    }

    #[test]
    fn a_refusal_is_invalid_input_in_one_line() {
        let (_tmp, home, env) = home();
        let dir = home.join(".agents/skills/atl");
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(dir.join("notes\n.md"), "mine").expect("write");
        let err = atl(&["install"], &env).expect_err("refused");
        assert_eq!(exit_code_for_error(&err), exit_code::INPUT_ERROR);
        assert_eq!(
            err.to_string(),
            format!(
                "invalid input: {dir} was not installed by atl (notes\\x0a.md): --force writes \
                 atl's files into it and keeps the rest"
            )
        );
        atl(&["install", "--force"], &env).expect("forced");
        let err = atl(&["status"], &Environment::default()).expect_err("no home");
        assert_eq!(exit_code_for_error(&err), exit_code::INPUT_ERROR);
        let project = atl(&["status", "--project"], &env).expect("project");
        assert_eq!(
            project["path"],
            json!(home.join("project/.agents/skills/atl").as_str())
        );
    }

    /// A file that cannot be read is a runtime error, not a refusal.
    #[test]
    #[cfg(unix)]
    fn a_file_that_cannot_be_read_is_a_runtime_error() {
        use std::os::unix::fs::PermissionsExt as _;
        let (_tmp, home, env) = home();
        atl(&["install"], &env).expect("install");
        let skill_md = home.join(".agents/skills/atl/SKILL.md");
        let chmod = |mode| {
            std::fs::set_permissions(&skill_md, std::fs::Permissions::from_mode(mode))
                .expect("chmod");
        };
        chmod(0o000);
        let result = atl(&["update"], &env);
        chmod(0o644);
        if let Err(err) = result {
            assert_eq!(exit_code_for_error(&err), exit_code::RUNTIME_ERROR, "{err}");
            assert!(err.to_string().starts_with("cannot read "), "{err}");
        } // else root, which reads it anyway
    }

    /// As root, in a home directory of another user's: nothing is written,
    /// `--force` or not, and `status` works.
    #[test]
    #[cfg(unix)]
    fn root_is_refused_in_another_users_home() {
        let (tmp, home, mut env) = home();
        let owner = skill::testing::not_roots(tmp.path());
        atl(&["install"], &env).expect("install");
        env.euid = Some(0);
        for command in ["install", "update", "remove"] {
            for command in [&[command][..], &[command, "--force"]] {
                let err = atl(command, &env).expect_err("refused");
                assert_eq!(
                    exit_code_for_error(&err),
                    exit_code::INPUT_ERROR,
                    "{command:?}"
                );
                assert_eq!(
                    err.to_string(),
                    format!(
                        "invalid input: {home} belongs to the user with id {owner}, and atl runs \
                         as root: run it without sudo, or what it writes there would be root's"
                    )
                );
            }
        }
        assert!(home.join(".agents/skills/atl/SKILL.md").is_file());
        assert_eq!(
            atl(&["status"], &env).expect("status")["state"],
            "other_owner"
        );
    }

    /// A skill directory of another user's, without root: refused (exit 5)
    /// `--force` or not, and `status` says whose it is not.
    #[test]
    #[cfg(unix)]
    fn a_skill_directory_of_another_user_is_refused() {
        use std::os::unix::fs::MetadataExt as _;
        let (_tmp, home, mut env) = home();
        atl(&["install"], &env).expect("install");
        let dir = home.join(".agents/skills/atl");
        let owner = std::fs::metadata(&dir).expect("stat").uid();
        let other = if owner == 4242 { 4243 } else { 4242 };
        env.euid = Some(other);
        for command in ["install", "update", "remove"] {
            for command in [&[command][..], &[command, "--force"]] {
                let err = atl(command, &env).expect_err("refused");
                assert_eq!(
                    (exit_code_for_error(&err), err.to_string()),
                    (
                        exit_code::INPUT_ERROR,
                        format!(
                            "invalid input: {dir} belongs to the user with id {owner}, and atl \
                             runs as the user with id {other}: atl does not touch the skill there"
                        )
                    ),
                    "{command:?}"
                );
            }
        }
        let status = atl(&["status"], &env).expect("status");
        assert_eq!(
            (&status["state"], &status["installed"]),
            (&json!("other_owner"), &json!(false))
        );
        assert!(dir.join("SKILL.md").is_file());
    }

    /// Where Claude Code is, the link is wanted where there are links: on
    /// Windows there are none to make, and the status says `not_wanted`.
    #[test]
    fn the_link_is_wanted_only_where_there_are_links() {
        let (_tmp, home, env) = home();
        std::fs::create_dir(home.join(".claude")).expect("mkdir");
        let place = home.join(".claude/skills/atl");
        let (link, state, made) = if cfg!(unix) {
            (place.as_str(), "missing", "created")
        } else {
            ("", "not_wanted", "")
        };
        let status = atl(&["status"], &env).expect("status");
        assert_eq!(
            (&status["link"], &status["link_state"]),
            (&json!(link), &json!(state))
        );
        let installed = atl(&["install"], &env).expect("install");
        assert_eq!(
            (&installed["link"], &installed["link_action"]),
            (&json!(link), &json!(made))
        );
    }
}
