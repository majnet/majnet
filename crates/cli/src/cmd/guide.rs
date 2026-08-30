//! Documentation the machine can read.
//!
//! Two audiences that a `--help` page serves badly. Shell completion wants a
//! generated script; an AI agent wants the part `--help` leaves out — which
//! commands mutate, which need which role, what the JSON looks like, and what
//! the confusing answers mean. `agent-guide` prints that, and `--install`
//! drops it in as a Claude Code skill so an agent working in a repo picks it
//! up without being told.
//!
//! The guide is compiled in (`include_str!`), so the binary and its
//! documentation can never drift apart across an upgrade.

use anyhow::{Context as _, Result};
use clap::{Args, CommandFactory};
use std::path::PathBuf;

/// The agent-facing reference. Also the source for `docs/cli-for-agents.md`.
const GUIDE: &str = include_str!("../../docs/agent-guide.md");

pub fn completions(shell: clap_complete::Shell) -> Result<()> {
    // `Cli` lives in main.rs; ask clap for the same command tree the binary
    // parses with, so completions cannot describe a stale surface.
    let mut command = crate::Cli::command();
    let name = command.get_name().to_string();
    clap_complete::generate(shell, &mut command, name, &mut std::io::stdout());
    Ok(())
}

#[derive(Args)]
pub struct GuideArgs {
    /// Write the guide into the current repo as a Claude Code skill
    /// (`.claude/skills/majnet/SKILL.md`) instead of printing it.
    #[arg(long)]
    pub install: bool,
    /// Directory to install into (default: the current directory).
    #[arg(long, value_name = "DIR", requires = "install")]
    pub path: Option<PathBuf>,
}

pub fn agent_guide(args: &GuideArgs) -> Result<()> {
    if !args.install {
        print!("{GUIDE}");
        return Ok(());
    }
    let root = args.path.clone().unwrap_or_else(|| PathBuf::from("."));
    let dir = root.join(".claude").join("skills").join("majnet");
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let path = dir.join("SKILL.md");
    std::fs::write(&path, skill()).with_context(|| format!("writing {}", path.display()))?;
    println!("installed {}", path.display());
    println!(
        "An agent in this repo will now load the guide when a task involves the MajNet \
         platform. Commit it if the whole team should get it."
    );
    Ok(())
}

/// The guide with the frontmatter a Claude Code skill needs. The description is
/// what decides whether an agent loads it, so it names the verbs someone would
/// actually type rather than describing the tool in the abstract.
fn skill() -> String {
    format!(
        "---\n\
         name: majnet\n\
         description: >-\n\
         \x20 Operate a MajNet self-hosted PaaS from the command line with the `majnet` CLI:\n\
         \x20 check fleet or app status, read container logs, run a command inside a running\n\
         \x20 app, query an app's managed database with SQL, promote or roll back a deploy,\n\
         \x20 cut a release, and inspect nodes, manifests, secrets and members. Use whenever\n\
         \x20 a task mentions MajNet, an app's environment (production/stable/testing/\n\
         \x20 ephemeral), a deploy, a render PR, or `majnet`.\n\
         ---\n\n\
         {GUIDE}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The guide ships inside the binary; an empty or truncated file would only
    /// show up when somebody asked for help.
    #[test]
    fn the_guide_is_compiled_in_and_substantial() {
        assert!(
            GUIDE.len() > 2000,
            "guide looks truncated ({} bytes)",
            GUIDE.len()
        );
        assert!(GUIDE.contains("majnet exec"));
        assert!(GUIDE.contains("majnet sql"));
    }

    /// Frontmatter has to parse as YAML or the skill is silently ignored.
    #[test]
    fn the_skill_has_well_formed_frontmatter() {
        let text = skill();
        let body = text.strip_prefix("---\n").expect("starts with frontmatter");
        let (front, rest) = body.split_once("\n---\n").expect("frontmatter terminator");
        let parsed: serde_yaml::Value = serde_yaml::from_str(front).expect("valid YAML");
        assert_eq!(parsed["name"], serde_yaml::Value::from("majnet"));
        assert!(parsed["description"].as_str().unwrap().contains("MajNet"));
        assert!(rest.contains("majnet exec"));
    }
}
