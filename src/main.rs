use anyhow::Result;
use clap::Parser;
use tracing::Level;
use tracing_subscriber::EnvFilter;

use atl::cli::args::{Cli, Command, SelfSubcommand};
use atl::cli::commands;
use atl::error::exit_code_for_error;
use atl::io::IoStreams;
use atl::output::Transforms;

fn main() {
    reset_sigpipe();
    if let Err(e) = run() {
        eprintln!("Error: {e:#}");
        std::process::exit(exit_code_for_error(&e));
    }
}

/// Restore the default `SIGPIPE` disposition on Unix so that piping `atl`
/// output into commands like `head` results in a clean exit on broken pipe
/// rather than a Rust panic.
#[cfg(unix)]
fn reset_sigpipe() {
    // SAFETY: setting a signal disposition before any threads are spawned
    // is sound; we run this as the very first thing in `main()`.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

#[cfg(not(unix))]
fn reset_sigpipe() {}

fn run() -> Result<()> {
    // Expand user-defined aliases (if any) before clap parses argv. This
    // must happen before `Cli::parse_from` so the resulting argv looks like
    // a normal invocation to clap. Best-effort: if config loading fails we
    // fall through with the original argv.
    let raw_args: Vec<String> = std::env::args().collect();
    let args = commands::alias::expand_aliases(raw_args);
    let cli = Cli::parse_from(args);

    init_logging(&cli);

    let mut io = IoStreams::system(&cli)?;

    let result = dispatch(&cli, &mut io);

    // Drop's safety net would also call this, but doing it explicitly here
    // means errors flushing the pager surface as a normal Result rather than
    // being silently swallowed.
    let stop_res = io.stop_pager();

    // After a successful command, best-effort check for a newer release and
    // print a notice to stderr. Suppressed for `atl self check` / `self
    // update` so we don't double-print during an explicit update workflow.
    // Runs only on the success path — if `dispatch` failed we propagate the
    // error without nagging the user about updates.
    if result.is_ok() {
        let is_self_cmd = matches!(&cli.command, Command::Self_(_));
        commands::updater_notifier::maybe_print_notice(&mut io, is_self_cmd);
    }

    // If dispatch succeeded but the pager couldn't be flushed, surface the
    // pager error so the user sees truncated output as a failure. If
    // dispatch failed, preserve the original error (it is more important
    // than any downstream pager shutdown hiccup).
    match result {
        Ok(()) => stop_res,
        Err(e) => Err(e),
    }
}

fn dispatch(cli: &Cli, io: &mut IoStreams) -> Result<()> {
    let transforms = Transforms {
        jq: cli.jq.as_deref(),
        template: cli.template.as_deref(),
    };

    match &cli.command {
        Command::Init => {
            let prompter = atl::auth::InquirePrompter;
            commands::init::run_init(io, &prompter)
        }
        Command::Config(cmd) => commands::config::run(
            &cmd.command,
            cli.config.as_deref(),
            &cli.format,
            io,
            &transforms,
        ),
        Command::Completions(args) => {
            use clap::CommandFactory;
            let mut cmd = Cli::command();
            let mut stdout = io.stdout();
            clap_complete::generate(args.shell, &mut cmd, "atl", &mut stdout);
            Ok(())
        }
        Command::Self_(cmd) => match &cmd.command {
            SelfSubcommand::Check => commands::updater::run_check(&cli.format, io, &transforms),
            SelfSubcommand::Update(args) => {
                commands::updater::run_update(args, &cli.format, io, &transforms)
            }
        },
        Command::Confluence(cmd) => run_async(commands::confluence::run(
            &cmd.command,
            cli.config.as_deref(),
            cli.profile.as_deref(),
            cli.retries,
            &cli.format,
            io,
            &transforms,
        )),
        Command::Jira(cmd) => run_async(commands::jira::run(
            &cmd.command,
            cli.config.as_deref(),
            cli.profile.as_deref(),
            cli.retries,
            &cli.format,
            io,
            &transforms,
        )),
        Command::Api(args) => run_async(commands::api::run(
            args,
            cli.config.as_deref(),
            cli.profile.as_deref(),
            cli.retries,
            &cli.format,
            io,
            &transforms,
        )),
        Command::Browse(args) => run_async(commands::browse::run(
            args,
            cli.config.as_deref(),
            cli.profile.as_deref(),
            cli.retries,
            io,
        )),
        Command::Alias(cmd) => {
            commands::alias::run(cmd, cli.config.as_deref(), &cli.format, io, &transforms)
        }
        Command::Auth(cmd) => {
            let store = atl::auth::SystemKeyring;
            let prompter = atl::auth::InquirePrompter;
            run_async(commands::auth::run(
                &cmd.command,
                cli.config.as_deref(),
                cli.profile.as_deref(),
                io,
                &store,
                &prompter,
                cli.retries,
            ))
        }
        Command::GenerateDocs(args) => commands::docs::run(args, io),
    }
}

fn run_async<F: std::future::Future<Output = Result<()>>>(fut: F) -> Result<()> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(fut)
}

fn init_logging(cli: &Cli) {
    let level = if cli.quiet {
        Level::ERROR
    } else {
        match cli.verbose {
            0 => Level::WARN,
            1 => Level::INFO,
            2 => Level::DEBUG,
            _ => Level::TRACE,
        }
    };

    let filter = EnvFilter::from_default_env().add_directive(level.into());

    // Use try_init so callers (e.g. integration tests that invoke `run`
    // multiple times in-process) don't panic on second initialization.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_ansi(!cli.no_color)
        .try_init();
}
