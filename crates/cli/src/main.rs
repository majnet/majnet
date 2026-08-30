//! `majnet` — the command-line client for a MajNet control plane.
//!
//! # What this is
//!
//! Everything the dashboard can do, from a laptop, in a form a script (or an
//! agent) can drive: fleet status, logs, deploys, releases, a shell in a
//! container, and SQL against an app's managed database.
//!
//! # How you are authenticated
//!
//! There is no token. Your identity is your **Tailscale device**: the control
//! plane sits behind a front door (`tailscale serve`, or the Caddy edge) that
//! resolves the calling tailnet IP to a login and injects it as a header the
//! backends trust (§16, ADR 0016). `people.yaml` maps that login to a GitHub
//! user and the platform-admin flag; each project's `project.yaml` carries the
//! per-project role. Both are edited in the dashboard — so permissions granted
//! in the UI are exactly the permissions you have here, with nothing to issue,
//! copy or revoke.
//!
//! The corollary is the failure mode this CLI works hardest to make obvious:
//! a request that arrives *without* a resolved identity is not refused, it is
//! answered as `infra` (the WG-mesh break-glass, §12.1) or falls through to the
//! dashboard's SPA with a 200. Both look like success. `majnet whoami` says
//! which of the three you are, and `client.rs` turns the SPA fallthrough into a
//! loud error instead of an empty list.
//!
//! # Two ways in
//!
//! - **through the dashboard** (default) — `--url http://<main-node>`; your
//!   identity is resolved, your project roles apply.
//! - **`--direct`** — straight at the WireGuard-internal listeners from a node
//!   or an enrolled peer. No identity header, so the backends see `infra` and
//!   every role check passes. That is break-glass, and `whoami` says so.

mod client;
mod cmd;
mod config;
mod output;
mod resolve;

use anyhow::Result;
use clap::{Parser, Subcommand};

use client::Client;
use config::{Config, Context};
use output::Format;

#[derive(Parser)]
#[command(
    name = "majnet",
    version,
    about = "Command-line client for a MajNet control plane",
    long_about = "Command-line client for a MajNet control plane.\n\n\
                  Authentication is your Tailscale identity — there is no token. Run \
                  `majnet login` once, then `majnet whoami` to see who the platform thinks \
                  you are and what you may do.\n\n\
                  Machine-readable documentation for scripts and AI agents: `majnet agent-guide`."
    // No `propagate_version`: it puts a `--version` flag on every subcommand,
    // which collides with `release promote <VERSION>`. `majnet --version` is
    // the only place anyone looks for it anyway.
)]
pub(crate) struct Cli {
    /// Output format (default: table on a terminal, json otherwise).
    #[arg(short, long, global = true, value_enum)]
    output: Option<Format>,

    /// Use a named context from the config file instead of the current one.
    #[arg(long, global = true, value_name = "NAME")]
    context: Option<String>,

    /// Control-plane origin for this invocation (overrides the context).
    #[arg(long, global = true, value_name = "URL", env = "MAJNET_URL")]
    url: Option<String>,

    /// Talk to the WireGuard-internal listeners directly. No identity is sent,
    /// so the control plane treats the call as `infra` break-glass.
    #[arg(long, global = true)]
    direct: bool,

    /// Skip the production confirmation prompt.
    #[arg(short = 'y', long, global = true)]
    yes: bool,

    /// Request timeout in seconds.
    #[arg(long, global = true, default_value_t = 30, value_name = "SECS")]
    timeout: u64,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Point this machine at a control plane and verify who it thinks you are.
    Login(cmd::auth::LoginArgs),
    /// Forget a context.
    Logout {
        /// Context to remove (default: the current one).
        name: Option<String>,
    },
    /// Who the control plane says you are, and what that lets you do.
    Whoami,
    /// Manage saved contexts.
    #[command(subcommand)]
    Context(cmd::auth::ContextCmd),

    /// One-screen fleet summary: nodes, deploys in flight, recent failures.
    Status,
    /// Recent fleet activity (the dashboard's feed).
    Events(cmd::read::EventArgs),
    /// Registered nodes.
    Nodes,
    /// Live per-node CPU, memory and container counts.
    Metrics {
        /// Only this node.
        #[arg(long)]
        node: Option<String>,
    },
    /// Registered projects.
    Projects,
    /// Apps in a project.
    Apps {
        /// Project name or GitHub org.
        project: Option<String>,
    },
    /// Everything about one app: environments, image, containers, build info.
    App {
        project: Option<String>,
        app: Option<String>,
    },
    /// Containers backing an app in one environment.
    Ps(cmd::read::TargetArgs),
    /// Container logs for an app in one environment.
    Logs(cmd::read::LogArgs),
    /// Build metadata each environment reported at `/info`.
    Info {
        project: Option<String>,
        app: Option<String>,
    },
    /// The app's manifest files as committed on ops `main`.
    Manifest {
        project: Option<String>,
        app: Option<String>,
    },
    /// Project members and their roles.
    Members { project: Option<String> },
    /// Secret names for an app+environment (values only with --reveal).
    Secrets(cmd::read::SecretArgs),

    /// Deploys: promote, restart, roll back, and the render PRs in flight.
    #[command(subcommand)]
    Deploy(cmd::deploy::DeployCmd),
    /// Releases: cut a version, review a draft, promote one.
    #[command(subcommand)]
    Release(cmd::release::ReleaseCmd),

    /// Run one command inside an app's container and print what it said.
    Exec(cmd::run::ExecArgs),
    /// Open an interactive shell — in an app container, or on a node.
    Shell(cmd::shell::ShellArgs),
    /// Run SQL against an app's managed database.
    Sql(cmd::run::SqlArgs),
    /// Describe an app's managed database (engine, name, where it lives).
    Db(cmd::read::TargetArgs),

    /// Control-plane version: what is pinned, what is running.
    #[command(subcommand)]
    ControlPlane(cmd::admin::ControlPlaneCmd),
    /// The platform version pinned in the platform repo.
    Version,

    /// Print a shell completion script.
    Completions {
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
    /// Print the reference another AI agent needs to drive this CLI safely.
    AgentGuide(cmd::guide::GuideArgs),
}

/// Everything a command needs: a configured client, the resolved defaults, and
/// how to print.
pub struct App {
    pub client: Client,
    pub ctx: Context,
    pub format: Format,
    pub yes: bool,
    /// The `--timeout` value, so a command that legitimately takes longer than
    /// a status query can build itself a more patient client.
    pub timeout: u64,
}

impl App {
    pub fn table(&self) -> bool {
        self.format == Format::Table
    }

    /// A client for the long calls. `exec` and `sql` are allowed 120s by the
    /// reconciler, so the default 30s request timeout would abandon a query the
    /// server is still happily running — and the caller would have no way to
    /// tell that from a failure. Never *shortens* an explicit `--timeout`.
    pub fn patient_client(&self) -> Result<Client> {
        Client::new(self.ctx.clone(), self.timeout.max(150))
    }
}

#[tokio::main]
async fn main() {
    // Rust ignores SIGPIPE, so a write to a closed pipe returns EPIPE and
    // `println!` panics — which turns `majnet events | head` into a stack
    // trace. Restore the default so the process just dies quietly, the way
    // every other command in a pipeline does.
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };

    if let Err(e) = run().await {
        // `{e:#}` keeps the anyhow context chain — the layer that failed *and*
        // what it was doing. Losing that is how "cannot reach" stops being
        // actionable.
        eprintln!("majnet: {e:#}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let cli = Cli::parse();

    // Two commands need no control plane at all; resolving a context first
    // would make `majnet completions` fail on a machine that isn't set up.
    match &cli.command {
        Command::Completions { shell } => return cmd::guide::completions(*shell),
        Command::AgentGuide(args) => return cmd::guide::agent_guide(args),
        _ => {}
    }

    let config = Config::load()?;
    if let Command::Login(args) = &cli.command {
        return cmd::auth::login(config, args, cli.url.as_deref(), cli.timeout).await;
    }
    if let Command::Logout { name } = &cli.command {
        return cmd::auth::logout(config, name.as_deref());
    }
    if let Command::Context(sub) = &cli.command {
        return cmd::auth::context(config, sub);
    }

    let mut ctx = config.resolve(cli.context.as_deref())?;
    if let Some(url) = &cli.url {
        ctx.url = Some(url.clone());
        ctx.direct = false;
    }
    if cli.direct {
        ctx.direct = true;
    }
    // After the overrides, not before: `--url` on a machine with no config file
    // is a perfectly good way to run this.
    ctx.require_endpoint()?;

    let format = cli.output.unwrap_or({
        // A pipe gets JSON: a caller redirecting output wants data, and column
        // alignment is not data.
        if std::io::IsTerminal::is_terminal(&std::io::stdout()) {
            Format::Table
        } else {
            Format::Json
        }
    });

    let app = App {
        client: Client::new(ctx.clone(), cli.timeout)?,
        ctx,
        format,
        yes: cli.yes,
        timeout: cli.timeout,
    };

    match cli.command {
        Command::Whoami => cmd::auth::whoami(&app).await,
        Command::Status => cmd::read::status(&app).await,
        Command::Events(args) => cmd::read::events(&app, &args).await,
        Command::Nodes => cmd::read::nodes(&app).await,
        Command::Metrics { node } => cmd::read::metrics(&app, node.as_deref()).await,
        Command::Projects => cmd::read::projects(&app).await,
        Command::Apps { project } => cmd::read::apps(&app, project.as_deref()).await,
        Command::App { project, app: name } => {
            cmd::read::app_detail(&app, project.as_deref(), name.as_deref()).await
        }
        Command::Ps(args) => cmd::read::ps(&app, &args).await,
        Command::Logs(args) => cmd::read::logs(&app, &args).await,
        Command::Info { project, app: name } => {
            cmd::read::info(&app, project.as_deref(), name.as_deref()).await
        }
        Command::Manifest { project, app: name } => {
            cmd::read::manifest(&app, project.as_deref(), name.as_deref()).await
        }
        Command::Members { project } => cmd::read::members(&app, project.as_deref()).await,
        Command::Secrets(args) => cmd::read::secrets(&app, &args).await,
        Command::Deploy(sub) => cmd::deploy::run(&app, sub).await,
        Command::Release(sub) => cmd::release::run(&app, sub).await,
        Command::Exec(args) => cmd::run::exec(&app, &args).await,
        Command::Shell(args) => cmd::shell::shell(&app, &args).await,
        Command::Sql(args) => cmd::run::sql(&app, &args).await,
        Command::Db(args) => cmd::read::db(&app, &args).await,
        Command::ControlPlane(sub) => cmd::admin::control_plane(&app, sub).await,
        Command::Version => cmd::admin::version(&app).await,
        // Handled above, before a control plane was required.
        Command::Login(_)
        | Command::Logout { .. }
        | Command::Context(_)
        | Command::Completions { .. }
        | Command::AgentGuide(_) => unreachable!(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    /// clap panics at runtime on a malformed command tree (duplicate flags, an
    /// optional positional ahead of a required one). `debug_assert` catches
    /// most of it; generating completions walks every subcommand, which is how
    /// the rest surfaces — and it is a real command, so it must not panic.
    #[test]
    fn the_command_tree_is_well_formed() {
        Cli::command().debug_assert();
        for shell in [
            clap_complete::Shell::Bash,
            clap_complete::Shell::Zsh,
            clap_complete::Shell::Fish,
        ] {
            let mut command = Cli::command();
            let mut sink = Vec::new();
            clap_complete::generate(shell, &mut command, "majnet", &mut sink);
            assert!(!sink.is_empty(), "{shell} completions came out empty");
        }
    }
}
