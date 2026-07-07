//! Cloudflare API client (ADR 0007) — the bot's third external credential.
//!
//! Automates the edge wiring for custom domains: for a production hostname it
//! finds the delegated zone, points a **proxied** DNS record at the prod node,
//! and sets the zone to Full (strict). Origin CA certificate issuance is a
//! separate step (see `origin_cert`). The reconciler never sees the token —
//! credential isolation (§6) holds.

use anyhow::{bail, Context, Result};
use majnet_common::manifest::AppManifest;
use majnet_common::platform::NodesFile;
use serde::Deserialize;
use std::collections::BTreeMap;

use crate::AppState;

const API: &str = "https://api.cloudflare.com/client/v4";

/// Ensure the Cloudflare edge wiring (proxied DNS → prod, Full-strict) for
/// every production hostname in a freshly rendered `env/production` tree.
/// No-op without a token. Per-host failures are logged, not fatal — a domain
/// whose zone isn't delegated to Cloudflare simply isn't wired yet.
pub async fn ensure_domains(state: &AppState, rendered: &BTreeMap<String, String>) -> Result<()> {
    let Some(token) = state.config.cloudflare_token.clone() else {
        return Ok(());
    };
    let hosts = production_hosts(rendered);
    if hosts.is_empty() {
        return Ok(());
    }
    let prod_ip = prod_public_ip(state)
        .await
        .context("resolving prod public IP for Cloudflare DNS")?;
    let cf = Cloudflare::new(state.http.clone(), token);
    for host in hosts {
        match cf.zone_for(&host).await {
            Err(e) => tracing::warn!(host, error = format!("{e:#}"), "skipping (no Cloudflare zone)"),
            Ok(zone) => {
                if let Err(e) = cf.ensure_dns_a(&zone, &host, &prod_ip).await {
                    tracing::error!(host, error = format!("{e:#}"), "Cloudflare DNS ensure failed");
                } else if let Err(e) = cf.ensure_ssl_strict(&zone).await {
                    tracing::error!(zone = zone.name, error = format!("{e:#}"), "Cloudflare SSL mode failed");
                } else {
                    tracing::info!(host, ip = prod_ip, "Cloudflare edge ensured");
                }
            }
        }
    }
    Ok(())
}

/// Production hostnames declared by ingress across the rendered app manifests
/// (top-level `<app>.yaml`; skips `secrets/…`).
fn production_hosts(rendered: &BTreeMap<String, String>) -> Vec<String> {
    let mut hosts: Vec<String> = rendered
        .iter()
        .filter(|(path, _)| !path.contains('/') && path.ends_with(".yaml"))
        .filter_map(|(_, yaml)| AppManifest::parse(yaml).ok())
        .filter_map(|m| m.ingress)
        .flat_map(|ing| ing.hosts().into_iter().map(String::from).collect::<Vec<_>>())
        .collect();
    hosts.sort();
    hosts.dedup();
    hosts
}

/// The prod node's public IPv4, from the platform `nodes.yaml`.
async fn prod_public_ip(state: &AppState) -> Result<String> {
    let client = state.github.org_client(&state.config.root_org).await?;
    let yaml =
        crate::platform_api::read_platform_file(&client, &state.config.root_org, "nodes.yaml")
            .await?;
    let nodes = NodesFile::parse(yaml.as_bytes())?;
    let prod = nodes.by_role("prod").context("no prod node in nodes.yaml")?;
    let ip = prod
        .public_endpoint
        .rsplit_once(':')
        .map(|(ip, _)| ip)
        .unwrap_or(&prod.public_endpoint);
    anyhow::ensure!(!ip.is_empty(), "prod node has no public endpoint yet");
    Ok(ip.to_string())
}

pub struct Cloudflare {
    http: reqwest::Client,
    token: String,
}

#[derive(Debug, Deserialize)]
struct Envelope<T> {
    success: bool,
    #[serde(default)]
    errors: Vec<ApiError>,
    result: Option<T>,
}

#[derive(Debug, Deserialize)]
struct ApiError {
    code: i64,
    message: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Zone {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Deserialize)]
struct DnsRecord {
    id: String,
    content: String,
    #[serde(default)]
    proxied: bool,
}

impl Cloudflare {
    pub fn new(http: reqwest::Client, token: String) -> Self {
        Self { http, token }
    }

    /// The zone that owns `host` — the registrable zone whose name equals or is
    /// a dotted suffix of `host`, longest match first. Errors if the host's
    /// domain isn't delegated to this Cloudflare account (the one thing the
    /// bot can't fix — nameserver delegation is manual).
    pub async fn zone_for(&self, host: &str) -> Result<Zone> {
        let zones: Vec<Zone> = self
            .get("/zones?per_page=50&status=active")
            .await
            .context("listing Cloudflare zones")?;
        select_zone(host, &zones).cloned().with_context(|| {
            format!(
                "no Cloudflare zone for '{host}' — is the domain added to this account and its \
                 nameservers delegated to Cloudflare? (zone creation/delegation is manual)"
            )
        })
    }

    /// Ensure a **proxied** A record `name → ip` exists (create or update).
    pub async fn ensure_dns_a(&self, zone: &Zone, name: &str, ip: &str) -> Result<()> {
        let existing: Vec<DnsRecord> = self
            .get(&format!(
                "/zones/{}/dns_records?type=A&name={name}",
                zone.id
            ))
            .await
            .context("listing DNS records")?;
        let body = serde_json::json!({
            "type": "A", "name": name, "content": ip, "proxied": true, "ttl": 1
        });
        match existing.first() {
            Some(rec) if rec.content == ip && rec.proxied => Ok(()),
            Some(rec) => self
                .send(
                    reqwest::Method::PATCH,
                    &format!("/zones/{}/dns_records/{}", zone.id, rec.id),
                    Some(body),
                )
                .await
                .with_context(|| format!("updating DNS record for {name}")),
            None => self
                .send(
                    reqwest::Method::POST,
                    &format!("/zones/{}/dns_records", zone.id),
                    Some(body),
                )
                .await
                .with_context(|| format!("creating DNS record for {name}")),
        }
    }

    /// Set the zone's SSL/TLS mode to Full (strict).
    pub async fn ensure_ssl_strict(&self, zone: &Zone) -> Result<()> {
        self.send(
            reqwest::Method::PATCH,
            &format!("/zones/{}/settings/ssl", zone.id),
            Some(serde_json::json!({ "value": "strict" })),
        )
        .await
        .context("setting SSL mode to Full (strict)")
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T> {
        let resp = self
            .http
            .get(format!("{API}{path}"))
            .bearer_auth(&self.token)
            .send()
            .await?;
        unwrap_envelope(resp).await
    }

    /// Send a mutating request and discard the (unmodelled) result body.
    async fn send(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> Result<()> {
        let mut req = self
            .http
            .request(method, format!("{API}{path}"))
            .bearer_auth(&self.token);
        if let Some(body) = body {
            req = req.json(&body);
        }
        let _: serde_json::Value = unwrap_envelope(req.send().await?).await?;
        Ok(())
    }
}

async fn unwrap_envelope<T: serde::de::DeserializeOwned>(resp: reqwest::Response) -> Result<T> {
    let status = resp.status();
    let env: Envelope<T> = resp
        .json()
        .await
        .with_context(|| format!("decoding Cloudflare response (HTTP {status})"))?;
    if !env.success {
        let msg = env
            .errors
            .iter()
            .map(|e| format!("[{}] {}", e.code, e.message))
            .collect::<Vec<_>>()
            .join("; ");
        bail!("Cloudflare API error (HTTP {status}): {msg}");
    }
    env.result
        .context("Cloudflare response had success=true but no result")
}

/// Longest-suffix zone match for a hostname. `app.majksa.cz` → `majksa.cz`.
fn select_zone<'a>(host: &str, zones: &'a [Zone]) -> Option<&'a Zone> {
    zones
        .iter()
        .filter(|z| host == z.name || host.ends_with(&format!(".{}", z.name)))
        .max_by_key(|z| z.name.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zones() -> Vec<Zone> {
        ["majksa.net", "majksa.cz", "sub.majksa.net"]
            .iter()
            .map(|n| Zone {
                id: format!("id-{n}"),
                name: n.to_string(),
            })
            .collect()
    }

    #[test]
    fn picks_longest_matching_zone() {
        let z = zones();
        assert_eq!(select_zone("app.majksa.cz", &z).unwrap().name, "majksa.cz");
        assert_eq!(select_zone("majksa.net", &z).unwrap().name, "majksa.net");
        // Longest suffix wins over the parent zone.
        assert_eq!(
            select_zone("a.sub.majksa.net", &z).unwrap().name,
            "sub.majksa.net"
        );
        assert!(select_zone("app.example.org", &z).is_none());
        // Not a real suffix (no dot boundary).
        assert!(select_zone("notmajksa.net", &z).is_none());
    }
}
