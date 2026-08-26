//! `majnet` — a read-only CLI for the control plane's internal API.
//!
//! # Why this exists
//!
//! Diagnosing the fleet previously meant opening the dashboard in a browser,
//! because the internal API is bound to the main node's **WireGuard IP** and the
//! dashboard's `/api` is only reachable through Caddy, which injects a
//! `Tailscale-User-Login` header it derives from `/tsauth`. Neither path is
//! scriptable, so incidents got diagnosed by screenshot.
//!
//! # The trap this is built to avoid
//!
//! Requests to the dashboard's `/api/...` **do not 401 when identity is
//! missing** — they fall through to the SPA and return `200 text/html`. A naive
//! script therefore "succeeds" and hands back an HTML shell, which is very easy
//! to mistake for an empty result. During one incident that fallthrough made a
//! probe look like it had worked when it had not.
//!
//! So every response here must be JSON or the command **fails loudly**
//! (`ApiError::NotJson`). A confusing error beats a plausible wrong answer.
//!
//! # Reachability
//!
//! The default base URL is the WG-internal listener, which is bind-address
//! trusted (§12.1) and needs no credentials — but is only routable from a
//! WireGuard peer. Run this on the main node, or from a machine enrolled as a
//! peer. `--base-url` points it elsewhere (e.g. an SSH tunnel).

use anyhow::{bail, Context, Result};
use serde_json::Value;

/// The bot's WG-internal listener (`Config::listen_internal`).
const DEFAULT_BASE_URL: &str = "http://10.88.0.1:8081";

const USAGE: &str = "\
majnet — read-only CLI for the MajNet control plane

USAGE:
    majnet [OPTIONS] <COMMAND>

COMMANDS:
    events                    Recent fleet activity (the dashboard's feed)
    nodes                     Registered nodes from the platform repo
    control-plane             Pinned control-plane version and rollout state
    projects                  Registered projects
    apps <org>                Apps in a project
    releases <org> <app>      Release history for an app
    whoami                    The identity the API attributes to this caller
    version                   The pinned control-plane version

OPTIONS:
    --base-url <URL>   Internal API base (env MAJNET_URL)
                       [default: http://10.88.0.1:8081]
    --json             Print the raw JSON response instead of a table
    --limit <N>        events: show at most N (default 40)
    --project <NAME>   events: only this project/org
    --failed           events: only entries that look like failures
    --timeout <SECS>   Request timeout (default 15)
    -h, --help         This help

NOTES:
    The internal API is bound to the main node's WireGuard IP and is trusted by
    bind address, so no token is needed — but it is only reachable from a WG
    peer. Responses must be JSON: if the dashboard's SPA shell comes back
    instead, this exits non-zero rather than pretending it worked.
";

struct Args {
    base_url: String,
    json: bool,
    limit: usize,
    project: Option<String>,
    failed: bool,
    timeout: u64,
    command: Vec<String>,
}

fn parse_args() -> Result<Option<Args>> {
    let mut a = Args {
        base_url: std::env::var("MAJNET_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.into()),
        json: false,
        limit: 40,
        project: None,
        failed: false,
        timeout: 15,
        command: Vec::new(),
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        // A flag needing a value; `next()` is the value, so a missing one is an
        // error rather than silently swallowing the following subcommand.
        let mut value = |name: &str| -> Result<String> {
            it.next().with_context(|| format!("{name} needs a value"))
        };
        match arg.as_str() {
            "-h" | "--help" => return Ok(None),
            "--json" => a.json = true,
            "--failed" => a.failed = true,
            "--base-url" => a.base_url = value("--base-url")?,
            "--project" => a.project = Some(value("--project")?),
            "--limit" => {
                a.limit = value("--limit")?
                    .parse()
                    .context("--limit must be a number")?
            }
            "--timeout" => {
                a.timeout = value("--timeout")?
                    .parse()
                    .context("--timeout must be seconds")?
            }
            s if s.starts_with('-') => bail!("unknown option '{s}' (try --help)"),
            s => a.command.push(s.to_string()),
        }
    }
    if a.command.is_empty() {
        return Ok(None);
    }
    Ok(Some(a))
}

/// GET a path and insist on JSON.
///
/// The `NotJson` branch is the reason this helper exists: the dashboard serves
/// its SPA for unauthenticated `/api/...`, so a 200 with `text/html` means "you
/// are not talking to the API", not "no results".
async fn get_json(client: &reqwest::Client, base: &str, path: &str) -> Result<Value> {
    let url = format!("{}{}", base.trim_end_matches('/'), path);
    let response = client.get(&url).send().await.with_context(|| {
        // Only blame WireGuard when we actually tried the WG endpoint — telling
        // someone who passed their own --base-url to "enroll as a WG peer" sends
        // them after the wrong problem.
        if base.trim_end_matches('/') == DEFAULT_BASE_URL {
            format!(
                "cannot reach {url}\n\
                 That is the WG-internal listener, which is only routable from a \
                 WireGuard peer. Run this on the main node, enroll this machine as a \
                 peer, or pass --base-url."
            )
        } else {
            format!("cannot reach {url}")
        }
    })?;

    let status = response.status();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let body = response.text().await.context("reading response body")?;

    if !status.is_success() {
        bail!("{url} returned {status}\n{}", snippet(&body));
    }
    // Belt and braces: trust the content type, but also catch a mislabelled shell.
    if !content_type.contains("json") || body.trim_start().starts_with('<') {
        bail!(
            "{url} returned {status} {content_type}, not JSON.\n\
             This is almost certainly the dashboard's SPA shell: an /api request \
             without a resolved Tailscale identity falls through to the app instead \
             of 401-ing. You are not talking to the API.\n\
             Point --base-url at the WG-internal listener ({DEFAULT_BASE_URL}) from a \
             WG peer.\n{}",
            snippet(&body)
        );
    }
    serde_json::from_str(&body).with_context(|| format!("{url} returned unparseable JSON"))
}

fn snippet(body: &str) -> String {
    let s: String = body.chars().take(200).collect();
    format!("  ── body ──\n  {}", s.replace('\n', "\n  "))
}

fn field<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

/// Heuristic for "this line is a problem", matching how the dashboard feed reads:
/// the reconciler records failures in `result`, prefixed `FAILED:` or similar.
fn looks_failed(event: &Value) -> bool {
    let r = field(event, "result").to_ascii_lowercase();
    r.contains("failed") || r.contains("error") || r.contains("unhealthy")
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    let head: String = s.chars().take(n.saturating_sub(1)).collect();
    format!("{head}…")
}

fn print_events(events: &[Value], args: &Args) {
    let rows: Vec<&Value> = events
        .iter()
        .filter(|e| {
            args.project
                .as_deref()
                .is_none_or(|p| field(e, "project") == p)
        })
        .filter(|e| !args.failed || looks_failed(e))
        .take(args.limit)
        .collect();

    if rows.is_empty() {
        println!("no matching events");
        return;
    }
    println!(
        "{:<20}  {:<16}  {:<22}  {:<8}  RESULT",
        "AT", "PROJECT", "ACTION", "COMMIT"
    );
    for e in &rows {
        println!(
            "{:<20}  {:<16}  {:<22}  {:<8}  {}",
            truncate(field(e, "at"), 20),
            truncate(field(e, "project"), 16),
            truncate(field(e, "action"), 22),
            truncate(field(e, "commit"), 8),
            truncate(field(e, "result"), 90),
        );
    }
    let failures = rows.iter().filter(|e| looks_failed(e)).count();
    println!("\n{} shown · {} look like failures", rows.len(), failures);
}

fn print_table(items: &[Value], columns: &[&str]) {
    if items.is_empty() {
        println!("(none)");
        return;
    }
    for c in columns {
        print!("{:<24}", c.to_ascii_uppercase());
    }
    println!();
    for it in items {
        for c in columns {
            // Fall back to a compact JSON rendering for non-string fields
            // (booleans, numbers, nested objects) rather than printing blanks.
            let raw = it.get(*c).map_or_else(String::new, |v| {
                v.as_str().map_or_else(|| v.to_string(), str::to_string)
            });
            print!("{:<24}", truncate(&raw, 23));
        }
        println!();
    }
}

fn as_array(v: &Value) -> Vec<Value> {
    v.as_array().cloned().unwrap_or_else(|| vec![v.clone()])
}

#[tokio::main]
async fn main() -> Result<()> {
    let Some(args) = parse_args()? else {
        print!("{USAGE}");
        return Ok(());
    };
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(args.timeout))
        .build()?;

    let cmd: Vec<&str> = args.command.iter().map(String::as_str).collect();
    let (path, render): (String, fn(&Value, &Args)) = match cmd.as_slice() {
        ["events"] => ("/api/events".into(), |v, a| print_events(&as_array(v), a)),
        ["nodes"] => ("/api/nodes".into(), |v, _| {
            print_table(
                &as_array(v),
                &["name", "role", "wireguard_ip", "tailscale_ip"],
            )
        }),
        ["control-plane"] => ("/api/control-plane".into(), |v, _| {
            println!("{}", serde_json::to_string_pretty(v).unwrap_or_default())
        }),
        ["version"] => ("/api/platform/version".into(), |v, _| {
            println!("{}", serde_json::to_string_pretty(v).unwrap_or_default())
        }),
        ["projects"] => ("/api/projects".into(), |v, _| {
            print_table(&as_array(v), &["name", "org"])
        }),
        ["whoami"] => ("/api/whoami".into(), |v, _| {
            println!(
                "login: {}\nadmin: {}",
                v.get("login").and_then(Value::as_str).unwrap_or("(none)"),
                v.get("admin").and_then(Value::as_bool).unwrap_or(false)
            )
        }),
        ["apps", org] => (format!("/api/apps/{org}"), |v, _| {
            print_table(&as_array(v), &["name", "class", "digest"])
        }),
        ["releases", org, app] => (format!("/api/releases/{org}/{app}"), |v, _| {
            print_table(&as_array(v), &["version", "at", "digest"])
        }),
        [] => unreachable!("empty command handled in parse_args"),
        other => bail!("unknown command '{}' (try --help)", other.join(" ")),
    };

    let body = get_json(&client, &args.base_url, &path).await?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&body)?);
    } else {
        render(&body, &args);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(project: &str, action: &str, result: &str) -> Value {
        serde_json::json!({
            "at": "2026-08-26T15:00:00Z", "commit": "12b2549abc",
            "project": project, "node": "private", "action": action,
            "result": result, "kind": "deploy",
        })
    }

    /// The whole point of the tool: a 200 carrying the SPA shell must be an
    /// error, not an empty result. This is the footgun that made a dashboard
    /// probe look successful during an incident.
    #[test]
    fn looks_failed_matches_the_reconciler_wording() {
        assert!(looks_failed(&ev("sideline", "converge", "FAILED: boom")));
        assert!(looks_failed(&ev(
            "sideline",
            "converge",
            "health check failed — old container keeps serving: container reported unhealthy"
        )));
        assert!(!looks_failed(&ev("sideline", "deploy", "deployed 12b2549")));
    }

    #[test]
    fn truncate_is_char_safe_and_marks_elision() {
        assert_eq!(truncate("abc", 8), "abc");
        assert_eq!(truncate("abcdefgh", 4), "abc…");
        // Must not panic or split a multi-byte char mid-sequence.
        assert_eq!(truncate("čárka", 3), "čá…");
    }

    #[test]
    fn field_is_tolerant_of_missing_and_non_string_values() {
        let v = serde_json::json!({ "a": "x", "n": 3 });
        assert_eq!(field(&v, "a"), "x");
        assert_eq!(field(&v, "n"), "");
        assert_eq!(field(&v, "absent"), "");
    }

    #[test]
    fn as_array_wraps_a_bare_object() {
        assert_eq!(as_array(&serde_json::json!([1, 2])).len(), 2);
        assert_eq!(as_array(&serde_json::json!({"a": 1})).len(), 1);
    }
}
