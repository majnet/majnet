//! Platform repo seeding: read `platform-seed/` from the install checkout,
//! render what we already know (the main node's entry in nodes.yaml), and
//! hand the tree to the bot to commit (writes-through-git, ADR 0004).

use anyhow::{Context, Result};
use majnet_common::platform::{NodesFile, VersionFile};
use std::collections::BTreeMap;
use std::path::Path;

use crate::state::SetupState;

/// The seed tree, with `nodes.yaml` rendered from wizard state.
pub fn build_tree(seed_dir: &Path, state: &SetupState) -> Result<BTreeMap<String, String>> {
    let mut files = BTreeMap::new();
    walk(seed_dir, seed_dir, &mut files)?;
    anyhow::ensure!(
        files.contains_key("nodes.yaml"),
        "platform-seed has no nodes.yaml at {}",
        seed_dir.display()
    );
    let rendered = render_nodes(&files["nodes.yaml"], state)?;
    files.insert("nodes.yaml".into(), rendered);
    anyhow::ensure!(
        files.contains_key("version.yaml"),
        "platform-seed has no version.yaml at {}",
        seed_dir.display()
    );
    match checkout_head(seed_dir) {
        Some(sha) => {
            let pinned = render_version(&files["version.yaml"], &sha)?;
            files.insert("version.yaml".into(), pinned);
        }
        // No git checkout around the seed (dev fixtures) — the seed's own
        // `ref` stands; a real install always pins the built commit.
        None => tracing::warn!("seed dir is not inside a git checkout — version.yaml not pinned"),
    }
    Ok(files)
}

/// The commit the installer checked out — `platform-seed/` sits at the repo
/// root, so its parent is the checkout.
fn checkout_head(seed_dir: &Path) -> Option<String> {
    let repo = seed_dir.parent()?;
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    let sha = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (out.status.success() && !sha.is_empty()).then_some(sha)
}

/// Pin the control plane to the exact commit the installer built (ADR 0005).
fn render_version(seed_yaml: &str, sha: &str) -> Result<String> {
    let mut pin = VersionFile::parse(seed_yaml.as_bytes()).context("parsing seed version.yaml")?;
    pin.control_plane.git_ref = sha.to_string();
    Ok(format!(
        "# Control-plane version pin (ADR 0005) — converged by majnet-update.\n\
         # Branch, tag, or full commit SHA; bump via commit, rollback = pin an older ref.\n{}",
        serde_yaml::to_string(&pin)?
    ))
}

fn walk(root: &Path, dir: &Path, files: &mut BTreeMap<String, String>) -> Result<()> {
    for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let path = entry?.path();
        if path.is_dir() {
            walk(root, &path, files)?;
        } else {
            let rel = path
                .strip_prefix(root)
                .expect("under root")
                .to_string_lossy()
                .replace('\\', "/");
            let content = std::fs::read_to_string(&path)
                .with_context(|| format!("reading {} (seed files must be text)", path.display()))?;
            files.insert(rel, content);
        }
    }
    Ok(())
}

/// Fill in every node the wizard already knows (at seed time: main).
fn render_nodes(seed_yaml: &str, state: &SetupState) -> Result<String> {
    let mut nodes = NodesFile::parse(seed_yaml.as_bytes()).context("parsing seed nodes.yaml")?;
    for node in &mut nodes.nodes {
        if let Some(known) = state.nodes.get(&node.name) {
            node.public_endpoint = known.public_endpoint.clone();
            node.wireguard_pubkey = known.wireguard_pubkey.clone();
        }
    }
    Ok(format!(
        "# Managed by the platform — updated via node enrollment (ADR 0004).\n{}",
        serde_yaml::to_string(&nodes)?
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::NodeEntry;

    #[test]
    fn renders_known_nodes_into_seed() {
        let seed = "wireguard_subnet: 10.88.0.0/24\ndocker_api_port: 2376\nnodes:\n  - name: main\n    role: main\n    wireguard_ip: 10.88.0.1\n  - name: prod\n    role: prod\n    wireguard_ip: 10.88.0.2\n";
        let mut state = SetupState::default();
        state.nodes.insert(
            "main".into(),
            NodeEntry {
                role: "main".into(),
                ssh_host: String::new(),
                wireguard_ip: "10.88.0.1".into(),
                public_endpoint: "203.0.113.1:51820".into(),
                wireguard_pubkey: "PUBKEY".into(),
            },
        );
        let out = render_nodes(seed, &state).unwrap();
        let parsed = NodesFile::parse(out.as_bytes()).unwrap();
        assert_eq!(parsed.nodes[0].wireguard_pubkey, "PUBKEY");
        assert_eq!(parsed.nodes[0].public_endpoint, "203.0.113.1:51820");
        assert_eq!(parsed.nodes[1].wireguard_pubkey, ""); // prod not enrolled yet
    }

    #[test]
    fn pins_version_to_the_built_commit() {
        let out = render_version("control_plane:\n  ref: main\n", "0123abcd").unwrap();
        let parsed = VersionFile::parse(out.as_bytes()).unwrap();
        assert_eq!(parsed.control_plane.git_ref, "0123abcd");
    }
}
