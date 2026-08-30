//! Login, identity, and contexts.
//!
//! "Login" is a misnomer worth keeping, because it is what people look for —
//! but nothing is issued here. The command finds the control plane, asks it who
//! you are, and writes the URL down. Your credential is the Tailscale device
//! you are sitting at, which is why there is no `--token` and no keychain.
//!
//! The interesting work is in `whoami`, which has to distinguish three states
//! the API deliberately does not distinguish for itself:
//!
//! | what came back            | what it means                                    |
//! |---------------------------|--------------------------------------------------|
//! | `login: "someone"`        | you, as a named human, with your roles           |
//! | `login: null, admin:true` | `infra` — no identity reached the API at all      |
//! | HTML with a 200           | the SPA shell; authentication failed silently     |
//!
//! The middle row is the dangerous one: `admin: true` reads as "you are an
//! admin" and actually means "nobody checked". It is correct on a node (the WG
//! bind address *is* the credential, §12.1) and wrong on a laptop, so it is
//! reported as what it is rather than as a name.

use anyhow::{bail, Context as _, Result};
use clap::{Args, Subcommand};
use serde::Deserialize;

use crate::client::{Api, Client};
use crate::config::{Config, Context, DEFAULT_BOT_URL, DEFAULT_RECON_URL};
use crate::output::{cell, rows, Table};
use crate::App;

#[derive(Debug, Deserialize)]
pub struct WhoAmI {
    pub login: Option<String>,
    #[serde(default)]
    pub admin: bool,
}

#[derive(Args)]
pub struct LoginArgs {
    /// Control-plane origin, e.g. `http://majksa` or `https://dash.example.net`.
    /// Omitted, the tailnet is searched for one.
    #[arg(long, value_name = "URL")]
    pub url: Option<String>,
    /// Save under this context name (default: `default`).
    #[arg(long, value_name = "NAME", default_value = "default")]
    pub name: String,
    /// Configure the WireGuard-internal listeners instead of a dashboard URL.
    /// Only useful on a node or an enrolled peer; sends no identity.
    #[arg(long)]
    pub direct: bool,
    /// Default project for this context.
    #[arg(long)]
    pub project: Option<String>,
    /// Default env class for this context.
    #[arg(long)]
    pub class: Option<String>,
}

pub async fn login(
    mut config: Config,
    args: &LoginArgs,
    global_url: Option<&str>,
    timeout: u64,
) -> Result<()> {
    let mut ctx = config.contexts.get(&args.name).cloned().unwrap_or_default();
    ctx.project = args.project.clone().or(ctx.project);
    ctx.class = args.class.clone().or(ctx.class);

    if args.direct {
        ctx.direct = true;
        ctx.bot_url = Some(ctx.bot_url.unwrap_or_else(|| DEFAULT_BOT_URL.to_string()));
        ctx.recon_url = Some(
            ctx.recon_url
                .unwrap_or_else(|| DEFAULT_RECON_URL.to_string()),
        );
    } else {
        let url = match args.url.as_deref().or(global_url) {
            Some(url) => normalise(url),
            None => discover(timeout).await?,
        };
        ctx.url = Some(url);
        ctx.direct = false;
    }

    // Verify before saving. A config file pointing at something that isn't a
    // control plane is worse than no config file: every later command fails
    // somewhere confusing instead of here, where the URL is on screen.
    let client = Client::new(ctx.clone(), timeout)?;
    let who: WhoAmI = client
        .get_json(Api::Bot, "/api/whoami")
        .await
        .context("that URL did not answer as a MajNet control plane")?;

    config.contexts.insert(args.name.clone(), ctx.clone());
    config.current = args.name.clone();
    let path = config.save()?;

    println!("saved context '{}' → {}", args.name, describe(&ctx));
    println!("config: {}", path.display());
    report_identity(&who, &ctx);
    if who.login.is_some() {
        println!("\nTry: majnet status");
    }
    Ok(())
}

pub fn logout(mut config: Config, name: Option<&str>) -> Result<()> {
    let target = name.unwrap_or(&config.current).to_string();
    if config.contexts.remove(&target).is_none() {
        bail!("no context named '{target}'");
    }
    if config.current == target {
        config.current = config.contexts.keys().next().cloned().unwrap_or_default();
    }
    config.save()?;
    println!("removed context '{target}'");
    Ok(())
}

pub async fn whoami(app: &App) -> Result<()> {
    let who: WhoAmI = app.client.get_json(Api::Bot, "/api/whoami").await?;

    if !app.table() {
        // JSON mode adds the CLI's own reading of the answer without hiding the
        // API's, so a script can branch on `identity` instead of on `login ==
        // null`, which means two different things.
        let value = serde_json::json!({
            "login": who.login,
            "platform_admin": who.admin,
            "identity": if who.login.is_some() { "human" } else { "infra" },
            "endpoint": describe(&app.ctx),
        });
        println!("{}", serde_json::to_string_pretty(&value)?);
        return Ok(());
    }

    report_identity(&who, &app.ctx);

    // Per-project roles, which is what "what may I do" actually depends on.
    if let Some(login) = &who.login {
        let projects = app.client.get_value(Api::Bot, "/api/projects").await?;
        let mut table = Table::new(&["project", "org", "your role"])
            .empty_note("(no projects in the registry)");
        for project in rows(&projects) {
            let org = cell(&project, "org");
            let role = match app
                .client
                .get_value(
                    Api::Bot,
                    &format!("/api/members/{}", crate::client::seg(&org)),
                )
                .await
            {
                Ok(members) => rows(&members)
                    .into_iter()
                    .find(|m| cell(m, "user").eq_ignore_ascii_case(login))
                    .map(|m| cell(&m, "role"))
                    .unwrap_or_else(|| {
                        if who.admin {
                            "— (platform admin)".into()
                        } else {
                            "—".into()
                        }
                    }),
                // A project whose ops repo is unreachable must not sink the
                // whole table; say so in the cell.
                Err(_) => "?".into(),
            };
            table.push(vec![cell(&project, "name"), org, role]);
        }
        println!();
        table.print();
    }
    Ok(())
}

/// Print the identity verdict — the part that must never be optimistic.
fn report_identity(who: &WhoAmI, ctx: &Context) {
    match &who.login {
        Some(login) => {
            println!("you:      {login}");
            println!(
                "platform: {}",
                if who.admin {
                    "admin (may change platform config, terminal, control-plane pin)"
                } else {
                    "member (project roles decide the rest)"
                }
            );
            println!("endpoint: {}", describe(ctx));
        }
        None => {
            println!("you:      (unidentified — the control plane sees this call as `infra`)");
            println!("endpoint: {}", describe(ctx));
            if ctx.direct {
                println!(
                    "\nThat is expected with --direct: the WireGuard-internal listeners are trusted\n\
                     by bind address (§12.1), so no identity is sent and every role check passes.\n\
                     Actions are audited as `infra` rather than as you. Use the dashboard URL to\n\
                     act as yourself."
                );
            } else {
                println!(
                    "\nThis is NOT you. The URL answered, but no Tailscale identity reached the API,\n\
                     so anything you do would be recorded as `infra` and would bypass your project\n\
                     roles. Usual causes:\n\
                     \x20 · this machine is not on the tailnet — check `tailscale status`\n\
                     \x20 · the URL is not the identity-injecting front door (that is the dashboard\n\
                     \x20   origin, not a raw WireGuard address)\n\
                     \x20 · your Tailscale login has no entry in people.yaml — a platform admin adds\n\
                     \x20   it in the dashboard"
                );
            }
        }
    }
}

fn describe(ctx: &Context) -> String {
    if ctx.direct {
        format!(
            "{} + {} (direct, no identity)",
            ctx.bot_url.as_deref().unwrap_or(DEFAULT_BOT_URL),
            ctx.recon_url.as_deref().unwrap_or(DEFAULT_RECON_URL)
        )
    } else {
        ctx.url.clone().unwrap_or_default()
    }
}

// ── discovery ────────────────────────────────────────────────────────────────

/// Find the control plane on the tailnet.
///
/// `tailscale status --json` lists every peer this device can see, which is a
/// short list on a private tailnet. Probing each for `/api/bot/api/whoami` and
/// taking the first that answers as JSON beats asking the user for a URL they
/// would have to go look up — and it can only find hosts they can already
/// reach, so it discovers nothing their ACLs don't already allow.
async fn discover(timeout: u64) -> Result<String> {
    let hosts = tailnet_hosts()?;
    if hosts.is_empty() {
        bail!(
            "no tailnet peers to search — is this machine on the tailnet? (`tailscale status`)\n\
             If you know the URL, pass it: `majnet login --url http://<main-node>`"
        );
    }
    eprintln!(
        "searching {} tailnet host(s) for a control plane…",
        hosts.len()
    );

    for host in &hosts {
        for scheme in ["https", "http"] {
            let url = format!("{scheme}://{host}");
            let ctx = Context {
                url: Some(url.clone()),
                ..Default::default()
            };
            // Short per-probe timeout: most candidates are not it, and a slow
            // sweep is a worse experience than asking for the URL.
            let Ok(client) = Client::new(ctx, timeout.min(5)) else {
                continue;
            };
            if client
                .get_json::<WhoAmI>(Api::Bot, "/api/whoami")
                .await
                .is_ok()
            {
                eprintln!("found: {url}");
                return Ok(url);
            }
        }
    }
    bail!(
        "searched {} tailnet host(s) and none answered as a MajNet control plane\n\
         Pass the URL directly: `majnet login --url http://<main-node>`",
        hosts.len()
    )
}

/// DNS names of this tailnet's devices, self first.
fn tailnet_hosts() -> Result<Vec<String>> {
    let output = std::process::Command::new("tailscale")
        .args(["status", "--json"])
        .output()
        .context(
            "could not run `tailscale` — install the Tailscale CLI, or pass \
             `majnet login --url <url>`",
        )?;
    if !output.status.success() {
        bail!(
            "`tailscale status` failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let status: serde_json::Value = serde_json::from_slice(&output.stdout)
        .context("`tailscale status --json` returned something unexpected")?;

    let name = |device: &serde_json::Value| -> Option<String> {
        device
            .get("DNSName")
            .and_then(|v| v.as_str())
            .map(|d| d.trim_end_matches('.').to_string())
            .filter(|d| !d.is_empty())
    };
    let mut hosts: Vec<String> = Vec::new();
    if let Some(host) = status.get("Self").and_then(name) {
        hosts.push(host);
    }
    if let Some(peers) = status.get("Peer").and_then(|v| v.as_object()) {
        for peer in peers.values() {
            // An offline peer cannot be the control plane right now, and
            // probing it just burns the timeout.
            if peer.get("Online").and_then(serde_json::Value::as_bool) == Some(false) {
                continue;
            }
            if let Some(host) = name(peer) {
                hosts.push(host);
            }
        }
    }
    hosts.dedup();
    Ok(hosts)
}

fn normalise(url: &str) -> String {
    let trimmed = url.trim().trim_end_matches('/');
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else {
        // A bare hostname is what people type; assume the tailnet's plain-HTTP
        // `tailscale serve` front door rather than failing on a missing scheme.
        format!("http://{trimmed}")
    }
}

// ── contexts ─────────────────────────────────────────────────────────────────

#[derive(Subcommand)]
pub enum ContextCmd {
    /// List saved contexts.
    List,
    /// Switch the current context.
    Use { name: String },
    /// Change defaults on a context.
    Set {
        /// Context to change (default: the current one).
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        app: Option<String>,
        #[arg(long)]
        class: Option<String>,
    },
    /// Show the current context in full.
    Show,
}

pub fn context(mut config: Config, cmd: &ContextCmd) -> Result<()> {
    match cmd {
        ContextCmd::List => {
            let mut table = Table::new(&["current", "name", "endpoint", "project", "class"])
                .empty_note("(no contexts — run `majnet login`)");
            for (name, ctx) in &config.contexts {
                table.push(vec![
                    if *name == config.current {
                        "*".into()
                    } else {
                        String::new()
                    },
                    name.clone(),
                    describe(ctx),
                    ctx.project.clone().unwrap_or_default(),
                    ctx.class_or_default(),
                ]);
            }
            table.print();
        }
        ContextCmd::Use { name } => {
            if !config.contexts.contains_key(name) {
                bail!(
                    "no context named '{name}' (have: {})",
                    config.names().join(", ")
                );
            }
            config.current = name.clone();
            config.save()?;
            println!("using context '{name}'");
        }
        ContextCmd::Set {
            name,
            url,
            project,
            app,
            class,
        } => {
            let target = name.clone().unwrap_or_else(|| config.current.clone());
            let ctx = config
                .contexts
                .get_mut(&target)
                .with_context(|| format!("no context named '{target}'"))?;
            if let Some(v) = url {
                ctx.url = Some(normalise(v));
                ctx.direct = false;
            }
            if let Some(v) = project {
                ctx.project = Some(v.clone());
            }
            if let Some(v) = app {
                ctx.app = Some(v.clone());
            }
            if let Some(v) = class {
                crate::resolve::class(&Context::default(), Some(v))?;
                ctx.class = Some(v.clone());
            }
            let ctx = ctx.clone();
            config.save()?;
            println!("context '{target}': {}", describe(&ctx));
        }
        ContextCmd::Show => {
            let ctx = config.resolve(None)?;
            ctx.require_endpoint()?;
            print!("{}", serde_yaml::to_string(&ctx)?);
        }
    }
    Ok(())
}

/// Exposed so `status` can print the same identity line without re-deciding
/// what it means.
pub async fn identity(app: &App) -> Result<WhoAmI> {
    app.client.get_json(Api::Bot, "/api/whoami").await
}

/// Used by `main` only in table mode; kept here so the mapping lives with the
/// rest of the identity logic.
pub fn identity_line(who: &WhoAmI) -> String {
    match &who.login {
        Some(login) if who.admin => format!("{login} (platform admin)"),
        Some(login) => login.clone(),
        None => "infra — unidentified (see `majnet whoami`)".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::Format;

    #[test]
    fn a_bare_hostname_gets_a_scheme_and_loses_a_trailing_slash() {
        assert_eq!(normalise("majksa"), "http://majksa");
        assert_eq!(
            normalise("https://dash.example.net/"),
            "https://dash.example.net"
        );
        assert_eq!(normalise("  http://x  "), "http://x");
    }

    /// `login: null, admin: true` is the API saying "nobody checked". It must
    /// never render as an administrator.
    #[test]
    fn an_unidentified_caller_is_never_shown_as_an_admin() {
        let infra = WhoAmI {
            login: None,
            admin: true,
        };
        assert!(identity_line(&infra).contains("unidentified"));
        assert!(!identity_line(&infra).contains("admin)"));
        let human = WhoAmI {
            login: Some("majksa".into()),
            admin: true,
        };
        assert_eq!(identity_line(&human), "majksa (platform admin)");
    }

    #[test]
    fn describe_names_both_listeners_in_direct_mode() {
        let direct = Context {
            direct: true,
            ..Default::default()
        };
        assert!(describe(&direct).contains("no identity"));
    }

    /// Format is a `clap::ValueEnum`; a stale variant here would only show up
    /// as a runtime parse failure.
    #[test]
    fn formats_round_trip_through_clap() {
        use clap::ValueEnum;
        assert_eq!(Format::from_str("json", true).unwrap(), Format::Json);
    }
}
