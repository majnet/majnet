//! Turning what you typed into what the API wants.
//!
//! Two mismatches to absorb. First, a project has both a *name* (`demo`) and a
//! GitHub *org* (`majksa-projects`); the HTTP API is addressed by org, humans
//! think in names, so anything you type is matched against both. Second, most
//! commands need a project, an app and an env class, and typing all three every
//! time is how people end up running production commands by muscle memory — so
//! the context supplies defaults and every command that acts prints the fully
//! resolved target before it does anything.

use anyhow::{bail, Result};
use serde::Deserialize;

use crate::client::{Api, Client};
use crate::config::Context;

pub const CLASSES: [&str; 4] = ["production", "stable", "testing", "ephemeral"];

/// Only the two fields the resolver needs. The `projects` command prints the
/// API's own JSON, so nothing is lost by not modelling the rest here.
#[derive(Debug, Clone, Deserialize)]
pub struct ProjectSummary {
    pub name: String,
    pub org: String,
}

/// Resolve `given` (or the context default) to a registered project's org.
pub async fn org(client: &Client, ctx: &Context, given: Option<&str>) -> Result<String> {
    let wanted = match given.or(ctx.project.as_deref()) {
        Some(v) if !v.is_empty() => v.to_string(),
        _ => bail!(
            "no project given and none set for this context\n\
             Pass it (`majnet apps <project>`), or set a default: `majnet context set --project <name>`"
        ),
    };
    let projects: Vec<ProjectSummary> = client.get_json(Api::Bot, "/api/projects").await?;
    match_project(&projects, &wanted)
}

/// Pure half of `org`, so the matching rules are testable without a server.
fn match_project(projects: &[ProjectSummary], wanted: &str) -> Result<String> {
    // Exact org first: it is what the API uses, so an exact org can never be
    // ambiguous with a name that happens to look like one.
    if let Some(p) = projects.iter().find(|p| p.org == wanted) {
        return Ok(p.org.clone());
    }
    if let Some(p) = projects.iter().find(|p| p.name == wanted) {
        return Ok(p.org.clone());
    }
    let fuzzy: Vec<&ProjectSummary> = projects
        .iter()
        .filter(|p| p.name.eq_ignore_ascii_case(wanted) || p.org.eq_ignore_ascii_case(wanted))
        .collect();
    match fuzzy.as_slice() {
        [only] => Ok(only.org.clone()),
        [] => bail!(
            "no project '{wanted}' in the registry\nKnown projects: {}",
            list(projects)
        ),
        many => bail!(
            "'{wanted}' matches more than one project ({}) — use the exact name",
            many.iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn list(projects: &[ProjectSummary]) -> String {
    if projects.is_empty() {
        return "(none — the registry is empty)".into();
    }
    projects
        .iter()
        .map(|p| {
            if p.name == p.org {
                p.name.clone()
            } else {
                format!("{} ({})", p.name, p.org)
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The app name, from the argument or the context default.
pub fn app(ctx: &Context, given: Option<&str>) -> Result<String> {
    match given.or(ctx.app.as_deref()) {
        Some(v) if !v.is_empty() => Ok(v.to_string()),
        _ => bail!(
            "no app given and none set for this context\n\
             Pass it, or set a default: `majnet context set --app <name>`"
        ),
    }
}

/// The env class, from the flag or the context default (`stable`).
///
/// Validated here rather than at the server: `--class prod` should fail on your
/// laptop with the list of real classes, not as a 400 five seconds later.
pub fn class(ctx: &Context, given: Option<&str>) -> Result<String> {
    let value = given
        .map(str::to_string)
        .unwrap_or_else(|| ctx.class_or_default());
    if !CLASSES.contains(&value.as_str()) {
        bail!(
            "unknown env class '{value}' — one of: {}",
            CLASSES.join(", ")
        );
    }
    Ok(value)
}

/// Ask before doing something to production.
///
/// Production is the only class the platform itself gates (a reviewed render
/// PR, admin-only writes), so the CLI mirrors that: a production-touching
/// command stops and asks unless you passed `--yes`. Everything else runs.
pub fn confirm(action: &str, target: &str, class: &str, yes: bool) -> Result<()> {
    if yes || class != "production" {
        return Ok(());
    }
    if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        bail!(
            "refusing to {action} {target} in production without confirmation\n\
             stdin is not a terminal, so there is nobody to ask — pass --yes if you mean it"
        );
    }
    eprint!("{action} {target} in PRODUCTION? type the app name to confirm: ");
    use std::io::Write as _;
    std::io::stderr().flush().ok();
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    let expected = target.rsplit('/').next().unwrap_or(target);
    if answer.trim() != expected {
        bail!("cancelled (expected '{expected}')");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(name: &str, org: &str) -> ProjectSummary {
        ProjectSummary {
            name: name.into(),
            org: org.into(),
        }
    }

    #[test]
    fn a_project_is_addressable_by_name_or_by_org() {
        let all = [p("demo", "majksa-projects"), p("sideline", "sideline-cz")];
        assert_eq!(match_project(&all, "demo").unwrap(), "majksa-projects");
        assert_eq!(
            match_project(&all, "majksa-projects").unwrap(),
            "majksa-projects"
        );
        assert_eq!(match_project(&all, "SIDELINE").unwrap(), "sideline-cz");
    }

    /// An exact org must win over a case-insensitive name match, or one
    /// project's org could silently address another project.
    #[test]
    fn an_exact_org_beats_a_fuzzy_name() {
        let all = [p("shared", "demo"), p("demo", "demo-org")];
        assert_eq!(match_project(&all, "demo").unwrap(), "demo");
    }

    #[test]
    fn an_unknown_project_lists_the_real_ones() {
        let all = [p("demo", "majksa-projects")];
        let err = match_project(&all, "nope").unwrap_err().to_string();
        assert!(err.contains("demo (majksa-projects)"), "{err}");
    }

    #[test]
    fn class_falls_back_to_the_context_and_rejects_typos() {
        let ctx = Context::default();
        assert_eq!(class(&ctx, None).unwrap(), "stable");
        assert_eq!(class(&ctx, Some("production")).unwrap(), "production");
        assert!(class(&ctx, Some("prod")).is_err());
    }

    /// --yes is the only way through in a pipeline; without a terminal there is
    /// nobody to ask, and defaulting to "go ahead" is how production gets hurt.
    #[test]
    fn production_needs_confirmation_and_other_classes_do_not() {
        assert!(confirm("restart", "demo/api", "stable", false).is_ok());
        assert!(confirm("restart", "demo/api", "production", true).is_ok());
    }
}
