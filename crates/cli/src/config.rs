//! Where the CLI keeps its (credential-free) state: `~/.config/majnet/config.yaml`.
//!
//! There is deliberately **no token in this file**. Identity is your Tailscale
//! device: the dashboard's front door (`tailscale serve`, or the Caddy edge)
//! resolves the calling tailnet IP to a login and injects it as a header the
//! control plane trusts (§16). So "logging in" is `tailscale up` plus knowing
//! the URL — and there is nothing here worth stealing, which is the point.
//!
//! A *context* bundles that URL with the defaults you'd otherwise retype on
//! every command (project, app, env class). Several contexts let one laptop
//! address more than one platform.

use anyhow::{bail, Context as _, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// The default env class when neither the command line nor the context says.
/// `stable`, not `production`: the cheap default must be the safe one.
pub const DEFAULT_CLASS: &str = "stable";

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Name of the context commands use unless `--context` overrides it.
    #[serde(default)]
    pub current: String,
    #[serde(default)]
    pub contexts: BTreeMap<String, Context>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Context {
    /// The dashboard origin — the front door that injects your identity.
    /// Unset in `direct` mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Talk to the WG-internal listeners instead of the dashboard. Then there
    /// is no identity header, so the control plane sees `infra` (§12.1). Only
    /// from a node or a WireGuard peer.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub direct: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bot_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recon_url: Option<String>,
    /// Defaults filled in when a command omits them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
}

/// The bot's WG-internal listener (`Config::listen_internal`) — the `direct`
/// default, reachable only from a WireGuard peer.
pub const DEFAULT_BOT_URL: &str = "http://10.88.0.1:8081";
/// The reconciler's WG-internal listener (`MAJNET_LISTEN`).
pub const DEFAULT_RECON_URL: &str = "http://10.88.0.1:9090";

pub fn config_path() -> Result<PathBuf> {
    if let Ok(explicit) = std::env::var("MAJNET_CONFIG") {
        return Ok(PathBuf::from(explicit));
    }
    let base = match std::env::var("XDG_CONFIG_HOME") {
        Ok(v) if !v.is_empty() => PathBuf::from(v),
        _ => {
            PathBuf::from(std::env::var("HOME").context("neither HOME nor XDG_CONFIG_HOME is set")?)
                .join(".config")
        }
    };
    Ok(base.join("majnet").join("config.yaml"))
}

impl Config {
    pub fn load() -> Result<Self> {
        let path = config_path()?;
        match std::fs::read_to_string(&path) {
            Ok(text) => serde_yaml::from_str(&text)
                .with_context(|| format!("parsing {} (delete it to start over)", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    pub fn save(&self) -> Result<PathBuf> {
        let path = config_path()?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        std::fs::write(&path, serde_yaml::to_string(self)?)
            .with_context(|| format!("writing {}", path.display()))?;
        Ok(path)
    }

    /// The context `name` (or the current one), with environment overrides
    /// applied. `MAJNET_URL` alone is enough to run without ever writing a
    /// config file — handy in CI and in a container on a node.
    pub fn resolve(&self, name: Option<&str>) -> Result<Context> {
        let mut ctx = match name
            .or(Some(self.current.as_str()))
            .filter(|n| !n.is_empty())
        {
            Some(n) => match self.contexts.get(n) {
                Some(c) => c.clone(),
                // An explicitly requested context that doesn't exist is an
                // error; a missing *default* just means "not set up yet", which
                // the env vars below may still satisfy.
                None if name.is_some() => {
                    bail!("no context named '{n}' (have: {})", self.names().join(", "))
                }
                None => Context::default(),
            },
            None => Context::default(),
        };
        if let Ok(v) = std::env::var("MAJNET_URL") {
            if !v.is_empty() {
                ctx.url = Some(v);
                ctx.direct = false;
            }
        }
        if let Ok(v) = std::env::var("MAJNET_BOT_URL") {
            if !v.is_empty() {
                ctx.bot_url = Some(v);
                ctx.direct = true;
            }
        }
        if let Ok(v) = std::env::var("MAJNET_RECON_URL") {
            if !v.is_empty() {
                ctx.recon_url = Some(v);
                ctx.direct = true;
            }
        }
        if let Ok(v) = std::env::var("MAJNET_PROJECT") {
            if !v.is_empty() {
                ctx.project = Some(v);
            }
        }
        // Deliberately *not* checked here: the caller still has `--url` and
        // `--direct` to apply on top, and rejecting an unconfigured context
        // before those are folded in made `majnet --url … nodes` fail on a
        // machine that had never run `majnet login`. See `require_endpoint`.
        Ok(ctx)
    }

    pub fn names(&self) -> Vec<&str> {
        self.contexts.keys().map(String::as_str).collect()
    }
}

impl Context {
    /// Fail if this context still names no control plane — called after every
    /// command-line override has been applied, never before.
    pub fn require_endpoint(&self) -> Result<()> {
        if self.url.is_none() && !self.direct {
            bail!(
                "no control plane configured — run `majnet login`, pass --url, or set \
                 MAJNET_URL to the dashboard origin (the front door that resolves your \
                 Tailscale identity)"
            );
        }
        Ok(())
    }

    pub fn bot_base(&self) -> String {
        if self.direct {
            return trim(self.bot_url.as_deref().unwrap_or(DEFAULT_BOT_URL));
        }
        format!("{}/api/bot", trim(self.url.as_deref().unwrap_or_default()))
    }

    pub fn recon_base(&self) -> String {
        if self.direct {
            return trim(self.recon_url.as_deref().unwrap_or(DEFAULT_RECON_URL));
        }
        format!(
            "{}/api/recon",
            trim(self.url.as_deref().unwrap_or_default())
        )
    }

    /// WebSocket URL for the reconciler's terminal, mirroring the base scheme.
    pub fn terminal_ws(&self, query: &str) -> String {
        let base = self.recon_base();
        let ws = if let Some(rest) = base.strip_prefix("https://") {
            format!("wss://{rest}")
        } else if let Some(rest) = base.strip_prefix("http://") {
            format!("ws://{rest}")
        } else {
            base
        };
        format!("{ws}/api/terminal?{query}")
    }

    pub fn class_or_default(&self) -> String {
        self.class
            .clone()
            .unwrap_or_else(|| DEFAULT_CLASS.to_string())
    }
}

fn trim(url: &str) -> String {
    url.trim_end_matches('/').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_mode_prefixes_both_backends_and_strips_trailing_slash() {
        let ctx = Context {
            url: Some("http://majksa/".into()),
            ..Default::default()
        };
        assert_eq!(ctx.bot_base(), "http://majksa/api/bot");
        assert_eq!(ctx.recon_base(), "http://majksa/api/recon");
    }

    #[test]
    fn direct_mode_talks_to_the_wg_listeners_unprefixed() {
        let ctx = Context {
            direct: true,
            ..Default::default()
        };
        assert_eq!(ctx.bot_base(), DEFAULT_BOT_URL);
        assert_eq!(ctx.recon_base(), DEFAULT_RECON_URL);
    }

    /// A https dashboard must produce `wss://`, or the shell silently fails to
    /// upgrade behind the Caddy edge.
    #[test]
    fn terminal_ws_follows_the_scheme() {
        let http = Context {
            url: Some("http://majksa".into()),
            ..Default::default()
        };
        assert!(http
            .terminal_ws("mode=host")
            .starts_with("ws://majksa/api/recon/api/terminal?"));
        let https = Context {
            url: Some("https://dash.example.net".into()),
            ..Default::default()
        };
        assert!(https
            .terminal_ws("mode=host")
            .starts_with("wss://dash.example.net/"));
    }

    #[test]
    fn an_unknown_named_context_is_an_error_but_an_unset_default_is_not() {
        // `resolve` folds in environment overrides; clear them so the test
        // describes the code and not the shell it happens to run in.
        for k in ["MAJNET_URL", "MAJNET_BOT_URL", "MAJNET_RECON_URL"] {
            std::env::remove_var(k);
        }
        let cfg = Config::default();
        assert!(cfg.resolve(Some("nope")).is_err());
        // No context and no env resolves fine — `--url` may still supply the
        // endpoint. The complaint comes later, from `require_endpoint`.
        let ctx = cfg.resolve(None).expect("an unset default is not an error");
        let err = ctx.require_endpoint().unwrap_err().to_string();
        assert!(err.contains("majnet login"), "{err}");
        // …and an endpoint from any source satisfies it.
        assert!(Context {
            url: Some("http://x".into()),
            ..Default::default()
        }
        .require_endpoint()
        .is_ok());
        assert!(Context {
            direct: true,
            ..Default::default()
        }
        .require_endpoint()
        .is_ok());
    }

    #[test]
    fn the_default_class_is_the_safe_one() {
        assert_eq!(Context::default().class_or_default(), "stable");
    }
}
