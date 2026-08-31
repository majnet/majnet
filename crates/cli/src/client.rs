//! The HTTP layer, and the one rule it exists to enforce: **never hand back a
//! plausible wrong answer**.
//!
//! The dashboard serves its SPA for any `/api/...` it cannot authenticate — an
//! HTTP 200 carrying `text/html` instead of a 401. A naive client reads that as
//! success and shows an empty list. During one incident that fallthrough made a
//! probe look like it had worked when it had not, so every response here is
//! checked for shape before it is believed, and a mismatch is a loud error
//! naming the likely cause.
//!
//! The same rule covers the quieter version: a request that reaches the API but
//! carries no identity is answered as `infra` (§12.1), not refused. That is
//! correct for a node-local break-glass and wrong for a laptop, so `whoami`
//! reports which one you are rather than printing a name and hoping.

use anyhow::{bail, Context as _, Result};
use serde::de::DeserializeOwned;
use std::time::Duration;

use crate::config::Context;

pub struct Client {
    http: reqwest::Client,
    pub ctx: Context,
}

/// Which backend a path belongs to. The two listeners are different processes
/// with different credentials (§ credential isolation), and the CLI keeps that
/// visible rather than pretending there is one API.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Api {
    /// The bot: git-shaped state — projects, apps, releases, promote, rollback.
    Bot,
    /// The reconciler: what is actually running — events, logs, metrics, exec.
    Recon,
}

impl Client {
    pub fn new(ctx: Context, timeout: u64) -> Result<Self> {
        Ok(Self {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(timeout))
                .user_agent(concat!("majnet-cli/", env!("CARGO_PKG_VERSION")))
                .build()?,
            ctx,
        })
    }

    pub fn base(&self, api: Api) -> String {
        match api {
            Api::Bot => self.ctx.bot_base(),
            Api::Recon => self.ctx.recon_base(),
        }
    }

    fn url(&self, api: Api, path: &str) -> String {
        format!("{}{}", self.base(api), path)
    }

    pub async fn get_json<T: DeserializeOwned>(&self, api: Api, path: &str) -> Result<T> {
        let value = self.get_value(api, path).await?;
        serde_json::from_value(value).with_context(|| {
            format!(
                "{} returned JSON in an unexpected shape",
                self.url(api, path)
            )
        })
    }

    /// GET returning untyped JSON — for `--json` passthrough and for endpoints
    /// whose shape the CLI doesn't need to know.
    pub async fn get_value(&self, api: Api, path: &str) -> Result<serde_json::Value> {
        let url = self.url(api, path);
        let response = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| self.reach(&url, e))?;
        let body = self.body(&url, response).await?;
        parse_json(&url, &body)
    }

    /// GET an endpoint that answers in plain text (logs, `platform/version`).
    pub async fn get_text(&self, api: Api, path: &str) -> Result<String> {
        let url = self.url(api, path);
        let response = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| self.reach(&url, e))?;
        self.body(&url, response).await
    }

    pub async fn post_json<T: DeserializeOwned>(
        &self,
        api: Api,
        path: &str,
        json: &serde_json::Value,
    ) -> Result<T> {
        let url = self.url(api, path);
        let response = self
            .http
            .post(&url)
            .json(json)
            .send()
            .await
            .map_err(|e| self.reach(&url, e))?;
        let body = self.body(&url, response).await?;
        let value = parse_json(&url, &body)?;
        serde_json::from_value(value)
            .with_context(|| format!("{url} returned JSON in an unexpected shape"))
    }

    /// POST/PUT whose answer is a human-readable line (most write endpoints).
    pub async fn send_text(
        &self,
        api: Api,
        method: reqwest::Method,
        path: &str,
        json: Option<&serde_json::Value>,
    ) -> Result<String> {
        let url = self.url(api, path);
        let mut request = self.http.request(method, &url);
        if let Some(body) = json {
            request = request.json(body);
        }
        let response = request.send().await.map_err(|e| self.reach(&url, e))?;
        Ok(self.body(&url, response).await?.trim().to_string())
    }

    /// Read a response, converting a non-2xx or an SPA shell into an error that
    /// says what actually happened.
    async fn body(&self, url: &str, response: reqwest::Response) -> Result<String> {
        let status = response.status();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let body = response.text().await.context("reading response body")?;

        if !status.is_success() {
            // The control plane's errors are plain text and specific
            // ("bob is a developer on demo — admin required"); pass them
            // through instead of burying them under an HTTP code.
            bail!(
                "{url} → {status}\n{}{}",
                indent(&body),
                self.misroute_hint(status)
            );
        }
        if is_html(&content_type, &body) {
            bail!(
                "{url} → {status} {content_type}, which is the dashboard's SPA shell, not the API.\n\
                 An /api request whose Tailscale identity cannot be resolved falls through to the \
                 web app instead of returning 401 — so this is an authentication failure wearing a \
                 200. You are not talking to the API.\n\
                 Check that the URL is the dashboard origin (`majnet login` sets it), that this \
                 machine is on the tailnet (`tailscale status`), and that the front door is the one \
                 that injects identity."
            );
        }
        Ok(body)
    }

    /// A 404 in proxy mode usually means the URL is a *backend* rather than the
    /// dashboard: the `/api/bot` and `/api/recon` prefixes only exist on the
    /// dashboard's front door, so pointing at a WireGuard listener produces a
    /// route miss instead of anything that names the real mistake. (The
    /// phase-0 CLI's `MAJNET_URL` meant the bot listener, so an old export in a
    /// shell profile lands exactly here.)
    fn misroute_hint(&self, status: reqwest::StatusCode) -> &'static str {
        if status == reqwest::StatusCode::NOT_FOUND && !self.ctx.direct {
            "\n  Is that URL the dashboard origin? The WireGuard listeners are reached with              --direct (or MAJNET_BOT_URL / MAJNET_RECON_URL) instead."
        } else {
            ""
        }
    }

    fn reach(&self, url: &str, e: reqwest::Error) -> anyhow::Error {
        let hint = if self.ctx.direct {
            "\nThat is a WireGuard-internal listener: it is only routable from a node or an \
             enrolled WG peer. Drop --direct to go through the dashboard instead."
        } else {
            "\nCheck `tailscale status` — the control plane is reachable over the tailnet only."
        };
        anyhow::Error::new(e).context(format!("cannot reach {url}{hint}"))
    }
}

fn is_html(content_type: &str, body: &str) -> bool {
    content_type.contains("text/html") || body.trim_start().starts_with("<!")
}

fn parse_json(url: &str, body: &str) -> Result<serde_json::Value> {
    if body.trim().is_empty() {
        return Ok(serde_json::Value::Null);
    }
    serde_json::from_str(body).with_context(|| {
        format!(
            "{url} returned unparseable JSON:\n{}",
            indent(&snippet(body))
        )
    })
}

fn snippet(body: &str) -> String {
    let s: String = body.chars().take(400).collect();
    if body.chars().count() > 400 {
        format!("{s}…")
    } else {
        s
    }
}

fn indent(body: &str) -> String {
    let s = snippet(body.trim());
    if s.is_empty() {
        return "  (empty body)".into();
    }
    format!("  {}", s.replace('\n', "\n  "))
}

/// Percent-encode one path segment. Project, app and node names are tame, but a
/// branch or version can carry a `/`, and splicing that into a path silently
/// addresses a different route.
pub fn seg(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole reason this module exists: a 200 carrying the SPA shell is an
    /// authentication failure, and must never read as an empty result.
    #[test]
    fn the_spa_shell_is_recognised_by_type_or_by_body() {
        assert!(is_html("text/html; charset=utf-8", "{}"));
        assert!(is_html("application/octet-stream", "<!doctype html><html>"));
        assert!(!is_html("application/json", "{\"login\":\"majksa\"}"));
    }

    #[test]
    fn an_empty_body_is_null_not_a_parse_error() {
        assert_eq!(parse_json("u", "   ").unwrap(), serde_json::Value::Null);
        assert!(parse_json("u", "not json").is_err());
    }

    #[test]
    fn path_segments_are_encoded() {
        assert_eq!(seg("sideline-server"), "sideline-server");
        assert_eq!(seg("@scope/leaf@1.2.3"), "%40scope%2Fleaf%401.2.3");
    }
}
