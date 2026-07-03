//! Snapshot client (§12.1): the reconciler holds no GitHub credentials — all
//! repo content arrives as tarballs from the bot's WG-internal proxy.

use anyhow::{bail, Context, Result};
use std::collections::BTreeMap;

use crate::config::Config;

pub struct Snapshot {
    pub commit: String,
    pub files: BTreeMap<String, Vec<u8>>,
}

/// Fetch `<org>/<repo>@<branch>`. Ok(None) = branch/repo doesn't exist
/// (e.g. a project with no `env/production` yet).
pub async fn fetch(http: &reqwest::Client, config: &Config, org: &str, repo: &str, branch: &str) -> Result<Option<Snapshot>> {
    let url = format!("{}/api/snapshot/{org}/{repo}/{}", config.bot_url, urlencode(branch));
    let response = http.get(&url).send().await.context("bot snapshot API unreachable")?;
    match response.status() {
        s if s.is_success() => {}
        // The bot proxies GitHub errors as 502; a missing branch is expected
        // for classes a project hasn't opted into yet.
        s if s == reqwest::StatusCode::BAD_GATEWAY || s == reqwest::StatusCode::NOT_FOUND => return Ok(None),
        s => bail!("snapshot {org}/{repo}@{branch}: bot returned {s}"),
    }
    let commit = response
        .headers()
        .get("x-majnet-commit")
        .and_then(|v| v.to_str().ok())
        .context("snapshot response missing X-Majnet-Commit")?
        .to_string();
    let bytes = response.bytes().await?;
    let files = majnet_common::tarball::untar(&bytes)?;
    Ok(Some(Snapshot { commit, files }))
}

fn urlencode(s: &str) -> String {
    s.replace('/', "%2F")
}
