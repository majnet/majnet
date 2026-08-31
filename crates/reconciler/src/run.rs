//! Imperative run surface (ADR 0029): one-shot `exec` inside an app container,
//! and `sql` against an app's managed database. Both exist for the `majnet`
//! CLI — the dashboard's terminal (ADR 0016) covers interactive work, but an
//! operator scripting a fix, and an agent diagnosing one, need a request that
//! returns a value and an exit code instead of a PTY.
//!
//! Like restart (§16) these are *imperative*: they change nothing git owns, so
//! they cannot be a commit. That makes the gate the only thing standing between
//! a caller and a production container, so it is the same gate `logs` uses —
//! production requires a project **admin**, every other class a **developer** —
//! and every call writes an audit event naming the caller and what they ran.
//!
//! Note the deliberate difference from the ADR 0016 terminal, which is
//! platform-admin only: that one hands out a *host* root shell across the whole
//! fleet. These two are scoped to one app the caller already administers, which
//! is the same blast radius they get from `restart` and the manifest editor.
//!
//! ## What the timeout does and does not do
//!
//! Docker exposes no way to cancel a running exec. `RUN_TIMEOUT` therefore
//! abandons *reading* the output; the command itself keeps running in the
//! container until it exits or the container is replaced. So the timeout bounds
//! this API's latency, not the work it started — which is why a timed-out call
//! is still audited.
//!
//! ## The read-only guard is a seatbelt, not a sandbox
//!
//! `sql` runs the engine's own client inside the engine container as the app's
//! derived role (§15) — never as the superuser — so a query has exactly the
//! privileges the app itself has. Without `write=true` the statement runs in a
//! read-only transaction, which catches a mistyped `UPDATE`. It does **not**
//! contain someone who is trying: anyone able to run SQL at all can start
//! another transaction. The boundary is the role check and the audit row, and
//! the docs say so rather than implying a sandbox that isn't there.

use anyhow::{Context, Result};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use bollard::models::ExecConfig;
use bollard::Docker;
use futures_util::StreamExt;
use majnet_common::manifest::{AppManifest, DbEngine};
use majnet_common::platform::{NodesFile, ProjectsFile};
use majnet_common::project::Role;
use majnet_common::EnvClass;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncWriteExt;

use crate::AppState;

type ApiError = (StatusCode, String);

/// Cap on captured output. A runaway command can stream gigabytes; a truncated
/// answer that says it was truncated beats an OOM or a hung CLI.
const MAX_OUTPUT: usize = 1 << 20;
/// Default row cap for `sql` (override with `?limit=`).
const DEFAULT_ROWS: usize = 200;
/// Hard ceiling on one command or query.
const RUN_TIMEOUT: Duration = Duration::from_secs(120);

// ── shared plumbing ──────────────────────────────────────────────────────────

/// Production is admin-only; every other class is developer-and-up. Same rule
/// as `logs`/`containers` — one table of who-may-touch-what, not three.
fn min_role(class: EnvClass) -> Role {
    if class == EnvClass::Production {
        Role::Admin
    } else {
        Role::Developer
    }
}

fn parse_class(raw: &str) -> Result<EnvClass, ApiError> {
    serde_yaml::from_str(raw).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "class must be production|stable|testing|ephemeral".into(),
        )
    })
}

fn forbidden(e: anyhow::Error) -> ApiError {
    (StatusCode::FORBIDDEN, format!("{e:#}"))
}

fn upstream(e: anyhow::Error) -> ApiError {
    (StatusCode::BAD_GATEWAY, format!("{e:#}"))
}

/// The platform snapshot plus the two things every handler here derives from
/// it: the class's node and a Docker client for it.
struct Placement {
    docker: Docker,
    node: String,
    /// The project *name* (container labels and DB names use it; callers
    /// address projects by org, like the dashboard does).
    project_name: String,
    platform: crate::snapshot::Snapshot,
}

async fn placement(state: &AppState, org: &str, class: EnvClass) -> Result<Placement> {
    let platform = crate::snapshot::fetch(
        &state.http,
        &state.config,
        &state.config.root_org,
        "platform",
        "main",
    )
    .await?
    .context("platform snapshot unavailable")?;
    let nodes = NodesFile::parse(platform.files.get("nodes.yaml").context("no nodes.yaml")?)?;
    let node = nodes
        .by_role(class.node_role())
        .with_context(|| format!("no node with role '{}' in nodes.yaml", class.node_role()))?;
    let docker = state.nodes(&nodes).client_for(node).await?;
    let project_name = platform
        .files
        .get("projects.yaml")
        .and_then(|b| ProjectsFile::parse(b).ok())
        .and_then(|pf| pf.projects.into_iter().find(|p| p.org == org))
        .map(|p| p.name)
        .unwrap_or_else(|| org.to_string());
    Ok(Placement {
        docker,
        node: node.name.clone(),
        project_name,
        platform,
    })
}

/// What one command produced. `exit_code` is the container's, not the API's —
/// a command that fails is a successful *request* carrying a non-zero code, so
/// a caller can tell "I could not run it" from "it ran and said no".
#[derive(Debug, Serialize)]
pub struct RunOutput {
    pub exit_code: i64,
    pub stdout: String,
    pub stderr: String,
    /// Output hit `MAX_OUTPUT` and was cut.
    pub truncated: bool,
}

/// Run `cmd` in `container`, feeding `stdin` and capturing both streams.
///
/// `tty: false` deliberately — a TTY merges stderr into stdout, and the whole
/// point of this path (unlike the terminal) is that a caller can tell them
/// apart and parse one of them.
async fn exec_capture(
    docker: &Docker,
    container: &str,
    cmd: Vec<String>,
    env: Option<Vec<String>>,
    stdin: Option<&str>,
    workdir: Option<String>,
) -> Result<RunOutput> {
    let exec = docker
        .create_exec(
            container,
            ExecConfig {
                cmd: Some(cmd),
                env,
                working_dir: workdir,
                attach_stdin: Some(stdin.is_some()),
                attach_stdout: Some(true),
                attach_stderr: Some(true),
                tty: Some(false),
                ..Default::default()
            },
        )
        .await
        .context("creating exec")?;

    let started = docker
        .start_exec(&exec.id, None::<bollard::exec::StartExecOptions>)
        .await
        .context("starting exec")?;

    let bollard::exec::StartExecResults::Attached {
        output: mut stream,
        mut input,
    } = started
    else {
        anyhow::bail!("exec detached unexpectedly");
    };

    if let Some(data) = stdin {
        input
            .write_all(data.as_bytes())
            .await
            .context("writing exec stdin")?;
        input.flush().await.ok();
        // Close stdin so clients that read to EOF (psql -f -, mysql) terminate.
        input.shutdown().await.ok();
    }
    drop(input);

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut truncated = false;
    let collect = async {
        while let Some(chunk) = stream.next().await {
            use bollard::container::LogOutput;
            let (buf, bytes) = match chunk.context("reading exec output")? {
                LogOutput::StdOut { message } | LogOutput::Console { message } => {
                    (&mut stdout, message)
                }
                LogOutput::StdErr { message } => (&mut stderr, message),
                LogOutput::StdIn { .. } => continue,
            };
            if buf.len() >= MAX_OUTPUT {
                truncated = true;
                continue;
            }
            let room = MAX_OUTPUT - buf.len();
            if bytes.len() > room {
                truncated = true;
                buf.extend_from_slice(&bytes[..room]);
            } else {
                buf.extend_from_slice(&bytes);
            }
        }
        Ok::<(), anyhow::Error>(())
    };
    tokio::time::timeout(RUN_TIMEOUT, collect)
        .await
        .map_err(|_| anyhow::anyhow!("timed out after {}s", RUN_TIMEOUT.as_secs()))??;

    let exit_code = docker
        .inspect_exec(&exec.id)
        .await
        .context("inspecting exec")?
        .exit_code
        .unwrap_or(-1);

    Ok(RunOutput {
        exit_code,
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
        truncated,
    })
}

// ── POST /api/exec/{project}/{class}/{app} ───────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ExecBody {
    /// argv, run directly (no shell). Use `["sh","-c","…"]` for a pipeline —
    /// explicit, so nobody is surprised by shell expansion they didn't ask for.
    pub cmd: Vec<String>,
    pub stdin: Option<String>,
    pub workdir: Option<String>,
}

// There is deliberately no `user` field. Docker's exec API would happily take
// one, which would let a project *developer* run as root inside the app
// container — reading root-owned paths, writing protected ones, and reading the
// secrets tmpfs even where the app's own user cannot. That is escalation beyond
// what the app itself runs as, for a convenience nobody asked for. The command
// runs as the image's user; a platform admin who genuinely needs root has the
// ADR 0016 terminal.

#[derive(Debug, Serialize)]
pub struct ExecResult {
    #[serde(flatten)]
    pub output: RunOutput,
    pub container: String,
    pub node: String,
}

pub async fn exec_post(
    State(state): State<Arc<AppState>>,
    Path((project, class, app)): Path<(String, String, String)>,
    headers: HeaderMap,
    Json(body): Json<ExecBody>,
) -> Result<Json<ExecResult>, ApiError> {
    let class = parse_class(&class)?;
    let actor = crate::authz::require(&state, &headers, &project, min_role(class))
        .await
        .map_err(forbidden)?;
    if body.cmd.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "cmd must not be empty".into()));
    }

    let placed = placement(&state, &project, class).await.map_err(upstream)?;
    let container = crate::terminal::find_app_container(
        &placed.docker,
        &placed.platform,
        &project,
        &app,
        class.as_str(),
    )
    .await
    .map_err(upstream)?;

    let line = body.cmd.join(" ");
    let attempt = exec_capture(
        &placed.docker,
        &container,
        body.cmd,
        None,
        body.stdin.as_deref(),
        body.workdir,
    )
    .await;

    // Audit BOTH outcomes. Recording only on success left a hole big enough to
    // matter: the timeout abandons *reading* the output, but Docker has no way
    // to cancel an exec, so the command is still running in the container — a
    // caller could run something long and leave no trace at all. A failed
    // attempt is exactly the one worth having on record.
    let outcome = match &attempt {
        Ok(o) => format!("exit {}", o.exit_code),
        Err(e) => format!("FAILED: {e:#}"),
    };
    let _ = state.store.record(
        "imperative",
        &placed.project_name,
        &placed.node,
        &format!("exec {app} ({})", class.as_str()),
        &format!("by {actor}: {line} → {outcome}"),
    );
    tracing::info!(%actor, project = %placed.project_name, app, class = class.as_str(), cmd = %line, outcome = %outcome, "cli exec");

    let output = attempt.map_err(upstream)?;
    Ok(Json(ExecResult {
        output,
        container,
        node: placed.node,
    }))
}

// ── managed database: metadata + queries ─────────────────────────────────────

/// Where an app's managed database lives and what speaks to it. No password —
/// this endpoint exists so a caller can *describe* the database, not connect to
/// it out of band.
#[derive(Debug, Serialize)]
pub struct DbInfo {
    pub engine: String,
    pub database: String,
    pub user: String,
    pub container: String,
    pub node: String,
    pub class: String,
    /// The engine speaks SQL and `sql` returns rows; false for valkey/mongodb,
    /// where the same endpoint returns the client's raw output instead.
    pub tabular: bool,
}

/// Everything needed to run one statement, resolved from the rendered manifest
/// plus the stateless credential derivation (§15).
struct DbTarget {
    docker: Docker,
    engine: DbEngine,
    database: String,
    user: String,
    password: String,
    container: String,
    node: String,
    project_name: String,
}

/// The credential half of a `DbTarget` — everything `client_invocation` needs,
/// and nothing that requires a live Docker connection. Split out so the
/// invocation logic (including the read-only guard) is unit-testable without a
/// daemon on the machine running the tests.
struct DbCreds {
    engine: DbEngine,
    database: String,
    user: String,
    password: String,
}

impl DbTarget {
    fn creds(&self) -> DbCreds {
        DbCreds {
            engine: self.engine,
            database: self.database.clone(),
            user: self.user.clone(),
            password: self.password.clone(),
        }
    }
}

async fn db_target(state: &AppState, org: &str, class: EnvClass, app: &str) -> Result<DbTarget> {
    let placed = placement(state, org, class).await?;

    // The engine is whatever the *rendered* manifest declares — the same file
    // the reconciler provisioned from, so the credentials below are the ones
    // that actually exist.
    let rendered =
        crate::snapshot::fetch(&state.http, &state.config, org, "ops", &class.env_branch())
            .await?
            .with_context(|| format!("{org}/ops has no {} branch yet", class.env_branch()))?;
    let yaml = rendered
        .files
        .get(&format!("{app}.yaml"))
        .with_context(|| {
            format!(
                "no rendered manifest for {app} in {} — is the app deployed to this class?",
                class.env_branch()
            )
        })?;
    let manifest = AppManifest::parse(std::str::from_utf8(yaml).context("manifest is not UTF-8")?)
        .context("rendered manifest failed validation")?;
    let engine = manifest
        .database
        .map(|d| d.engine)
        .with_context(|| format!("{app} ({}) declares no database", class.as_str()))?;

    let (database, password) =
        crate::db::app_credentials(&state.config, engine, &placed.project_name, app, class)?;
    Ok(DbTarget {
        docker: placed.docker,
        engine,
        user: database.clone(),
        database,
        password,
        container: crate::db::engine_container(engine).to_string(),
        node: placed.node,
        project_name: placed.project_name,
    })
}

/// SQL engines return rows; the key/document stores return their client's raw
/// output, because inventing a table shape for `INFO` or `db.stats()` would be
/// a lie about what came back.
fn tabular(engine: DbEngine) -> bool {
    matches!(engine, DbEngine::Postgres | DbEngine::Mariadb)
}

pub async fn db_get(
    State(state): State<Arc<AppState>>,
    Path((project, class, app)): Path<(String, String, String)>,
    headers: HeaderMap,
) -> Result<Json<DbInfo>, ApiError> {
    let class = parse_class(&class)?;
    crate::authz::require(&state, &headers, &project, min_role(class))
        .await
        .map_err(forbidden)?;
    let t = db_target(&state, &project, class, &app)
        .await
        .map_err(upstream)?;
    Ok(Json(DbInfo {
        engine: format!("{:?}", t.engine).to_lowercase(),
        database: t.database,
        user: t.user,
        container: t.container,
        node: t.node,
        class: class.as_str().to_string(),
        tabular: tabular(t.engine),
    }))
}

#[derive(Debug, Deserialize)]
pub struct SqlQuery {
    /// Allow statements that write. Default false → the statement runs in a
    /// read-only transaction (see the module docs on what that is and isn't).
    #[serde(default)]
    pub write: bool,
    /// Row cap on the response (default 200).
    pub limit: Option<usize>,
    /// Ask the server for a catalog listing instead of running `sql`:
    /// `tables`, or `columns` with `table=`.
    pub meta: Option<String>,
    pub table: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SqlBody {
    pub sql: String,
}

#[derive(Debug, Serialize)]
pub struct SqlResult {
    pub engine: String,
    pub database: String,
    pub read_only: bool,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub row_count: usize,
    /// Rows beyond `limit` were dropped from the response (the statement still
    /// ran in full — this is a display cap, not a `LIMIT`).
    pub truncated: bool,
    /// Non-tabular engines (valkey, mongodb) report here instead of `rows`.
    pub raw: Option<String>,
    /// Anything the client wrote to stderr that wasn't fatal (notices, warnings).
    pub notice: Option<String>,
}

pub async fn sql_post(
    State(state): State<Arc<AppState>>,
    Path((project, class, app)): Path<(String, String, String)>,
    Query(q): Query<SqlQuery>,
    headers: HeaderMap,
    Json(body): Json<SqlBody>,
) -> Result<Json<SqlResult>, ApiError> {
    let class = parse_class(&class)?;
    // A write needs project-admin everywhere, not just in production: "read the
    // stable database" and "mutate it" are different asks, and the role model
    // has no third tier to express that, so writes take the higher one.
    let needed = if q.write {
        Role::Admin
    } else {
        min_role(class)
    };
    let actor = crate::authz::require(&state, &headers, &project, needed)
        .await
        .map_err(forbidden)?;

    let target = db_target(&state, &project, class, &app)
        .await
        .map_err(upstream)?;

    let sql = match q.meta.as_deref() {
        None => body.sql.trim().to_string(),
        Some(kind) => catalog_query(target.engine, kind, q.table.as_deref())
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("{e:#}")))?,
    };
    if sql.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "sql must not be empty".into()));
    }
    // Catalog listings are reads by construction; never let `write=true` on a
    // `meta=` call open a read-write transaction nobody asked for.
    let write = q.write && q.meta.is_none();

    let attempt = run_sql(&target, &sql, write, q.limit.unwrap_or(DEFAULT_ROWS)).await;

    // Audited either way — a statement the engine *rejected* is the one most
    // worth keeping, since that is what probing looks like.
    let outcome = match &attempt {
        Ok(r) => format!("{} row(s)", r.row_count),
        Err(e) => format!("FAILED: {}", one_line(&format!("{e:#}"), 200)),
    };
    let _ = state.store.record(
        "imperative",
        &target.project_name,
        &target.node,
        &format!("sql {app} ({})", class.as_str()),
        &format!(
            "by {actor} [{}]: {} → {outcome}",
            if write { "write" } else { "read-only" },
            one_line(&sql, 300),
        ),
    );
    tracing::info!(%actor, project = %target.project_name, app, class = class.as_str(), write, outcome = %outcome, "cli sql");

    Ok(Json(attempt.map_err(upstream)?))
}

/// Collapse a statement to one audit-log line without losing the shape of it.
fn one_line(sql: &str, max: usize) -> String {
    let flat = sql.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        return flat;
    }
    flat.chars().take(max.saturating_sub(1)).collect::<String>() + "…"
}

/// The engine-specific "what's in here" queries, so a caller (or an agent) can
/// orient without knowing each engine's catalog dialect.
fn catalog_query(engine: DbEngine, kind: &str, table: Option<&str>) -> Result<String> {
    let q = match (engine, kind) {
        (DbEngine::Postgres, "tables") => "SELECT table_schema, table_name \
             FROM information_schema.tables \
             WHERE table_schema NOT IN ('pg_catalog','information_schema') \
             ORDER BY 1, 2"
            .to_string(),
        (DbEngine::Postgres, "columns") => {
            let t = table.context("meta=columns needs table=")?;
            format!(
                "SELECT column_name, data_type, is_nullable, column_default \
                 FROM information_schema.columns \
                 WHERE table_name = {} ORDER BY ordinal_position",
                sql_literal(t)
            )
        }
        (DbEngine::Mariadb, "tables") => "SELECT table_name, table_rows \
             FROM information_schema.tables WHERE table_schema = DATABASE() ORDER BY 1"
            .to_string(),
        (DbEngine::Mariadb, "columns") => {
            let t = table.context("meta=columns needs table=")?;
            format!(
                "SELECT column_name, column_type, is_nullable, column_default \
                 FROM information_schema.columns \
                 WHERE table_schema = DATABASE() AND table_name = {} \
                 ORDER BY ordinal_position",
                sql_literal(t)
            )
        }
        (DbEngine::Valkey, "tables") => "INFO keyspace".to_string(),
        (DbEngine::Mongodb, "tables") => "db.getCollectionNames()".to_string(),
        (DbEngine::Mongodb, "columns") => {
            let t = table.context("meta=columns needs table=")?;
            // Mongo has no schema; the closest honest answer is one document.
            format!("db.getCollection({}).findOne()", json_literal(t))
        }
        (engine, kind) => anyhow::bail!("meta={kind} is not supported for {engine:?}"),
    };
    Ok(q)
}

/// Single-quoted SQL literal with quotes doubled. Only ever wraps a caller's
/// table name for the catalog queries above — user SQL is passed through
/// verbatim, since running arbitrary SQL is the whole feature.
fn sql_literal(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

fn json_literal(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

async fn run_sql(target: &DbTarget, sql: &str, write: bool, limit: usize) -> Result<SqlResult> {
    let (cmd, env, stdin) = client_invocation(&target.creds(), sql, write);
    let out = exec_capture(
        &target.docker,
        &target.container,
        cmd,
        Some(env),
        stdin.as_deref(),
        None,
    )
    .await?;

    let engine_name = format!("{:?}", target.engine).to_lowercase();
    if out.exit_code != 0 {
        // The engine's own error is far more useful than ours — surface it.
        let msg = if out.stderr.trim().is_empty() {
            out.stdout.trim().to_string()
        } else {
            out.stderr.trim().to_string()
        };
        anyhow::bail!(
            "{engine_name} rejected the statement (exit {}): {msg}",
            out.exit_code
        );
    }

    let notice = Some(out.stderr.trim().to_string()).filter(|s| !s.is_empty());
    if !tabular(target.engine) {
        return Ok(SqlResult {
            engine: engine_name,
            database: target.database.clone(),
            read_only: !write,
            columns: Vec::new(),
            rows: Vec::new(),
            row_count: 0,
            truncated: out.truncated,
            raw: Some(out.stdout),
            notice,
        });
    }

    let (columns, mut rows) = match target.engine {
        DbEngine::Postgres => parse_csv(&out.stdout),
        _ => parse_tsv(&out.stdout),
    };
    let row_count = rows.len();
    let over = row_count > limit;
    if over {
        rows.truncate(limit);
    }
    Ok(SqlResult {
        engine: engine_name,
        database: target.database.clone(),
        read_only: !write,
        columns,
        rows,
        row_count,
        truncated: over || out.truncated,
        raw: None,
        notice,
    })
}

/// (argv, env, stdin) for one statement against one engine.
///
/// SQL travels on **stdin**, never inside an argv string: there is no shell in
/// the exec, but a statement containing a newline or a quote still has no
/// business being spliced into a command line.
fn client_invocation(
    target: &DbCreds,
    sql: &str,
    write: bool,
) -> (Vec<String>, Vec<String>, Option<String>) {
    let s = |v: &str| v.to_string();
    match target.engine {
        DbEngine::Postgres => {
            // Read-only is set on the *connection* (`PGOPTIONS`), not as a
            // statement in the script. Sending `SET TRANSACTION READ ONLY;`
            // ahead of the query made psql print its `SET` command tag, and
            // `--csv` puts that on stdout — so the parser read "SET" as the
            // header row and every column name shifted down into the data.
            // A connection-level setting produces no output at all.
            //
            // `-q` suppresses the remaining status tags (`CREATE TABLE`, …) for
            // the same reason: only result rows should reach the parser.
            let mut env = vec![format!("PGPASSWORD={}", target.password)];
            if !write {
                env.push("PGOPTIONS=-c default_transaction_read_only=on".into());
            }
            (
                vec![
                    s("psql"),
                    s("--csv"),
                    s("-q"),
                    s("--no-psqlrc"),
                    s("-v"),
                    s("ON_ERROR_STOP=1"),
                    s("--single-transaction"),
                    s("-U"),
                    target.user.clone(),
                    s("-d"),
                    target.database.clone(),
                    s("-f"),
                    s("-"),
                ],
                env,
                Some(sql.to_string()),
            )
        }
        DbEngine::Mariadb => {
            // Same reasoning as postgres: the guard runs at connect time
            // (`--init-command`) rather than as a statement in the script, so
            // nothing of ours can end up in the output the parser reads.
            let guard = if write {
                Vec::new()
            } else {
                vec![s("--init-command"), s("SET SESSION TRANSACTION READ ONLY")]
            };
            // The image ships `mariadb`; older tags only `mysql`. Pick at run
            // time rather than pinning a name that silently changed under us.
            let mut cmd =
                vec![
                s("sh"),
                s("-c"),
                s("if command -v mariadb >/dev/null 2>&1; then bin=mariadb; else bin=mysql; fi; \
                   user=$1; db=$2; shift 2; \
                   exec \"$bin\" --batch --raw -u \"$user\" -D \"$db\" \"$@\""),
                s("majnet-sql"),
                target.user.clone(),
                target.database.clone(),
            ];
            cmd.extend(guard);
            (
                cmd,
                vec![format!("MYSQL_PWD={}", target.password)],
                Some(sql.to_string()),
            )
        }
        DbEngine::Valkey => (
            vec![
                s("valkey-cli"),
                s("--user"),
                target.user.clone(),
                s("--pass"),
                target.password.clone(),
                s("--no-auth-warning"),
            ],
            Vec::new(),
            Some(format!("{sql}\n")),
        ),
        DbEngine::Mongodb => (
            vec![
                s("mongosh"),
                format!(
                    "mongodb://{}:{}@localhost:27017/{}?authSource={}",
                    target.user, target.password, target.database, target.database
                ),
                s("--quiet"),
                s("--eval"),
                sql.to_string(),
            ],
            Vec::new(),
            None,
        ),
    }
}

// ── output parsing ───────────────────────────────────────────────────────────

/// RFC 4180 CSV (what `psql --csv` emits), first record = header.
///
/// Hand-rolled rather than pulling a CSV crate into the control plane for one
/// call site: quoted fields, doubled quotes, embedded newlines and commas. A
/// statement that returns nothing yields no header and no rows.
fn parse_csv(text: &str) -> (Vec<String>, Vec<Vec<String>>) {
    let mut records: Vec<Vec<String>> = Vec::new();
    let mut record: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    let mut any = false;

    while let Some(c) = chars.next() {
        any = true;
        match (quoted, c) {
            (true, '"') => {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    quoted = false;
                }
            }
            (true, c) => field.push(c),
            (false, '"') if field.is_empty() => quoted = true,
            (false, ',') => record.push(std::mem::take(&mut field)),
            (false, '\r') => {}
            (false, '\n') => {
                record.push(std::mem::take(&mut field));
                records.push(std::mem::take(&mut record));
            }
            (false, c) => field.push(c),
        }
    }
    if !field.is_empty() || !record.is_empty() {
        record.push(field);
        records.push(record);
    }
    if !any || records.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let header = records.remove(0);
    (header, records)
}

/// `mysql --batch` output: tab-separated, first line = header, `\t`/`\n`/`\\`
/// backslash-escaped inside values (that's what `--raw` still leaves in).
fn parse_tsv(text: &str) -> (Vec<String>, Vec<Vec<String>>) {
    let mut lines = text.lines();
    let Some(header) = lines.next() else {
        return (Vec::new(), Vec::new());
    };
    let split = |line: &str| -> Vec<String> {
        line.split('\t')
            .map(|f| {
                f.replace("\\n", "\n")
                    .replace("\\t", "\t")
                    .replace("\\\\", "\\")
            })
            .collect()
    };
    (split(header), lines.map(split).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_handles_quotes_commas_and_newlines() {
        let (cols, rows) = parse_csv("id,name\n1,\"Smith, John\"\n2,\"line\nbreak\"\n");
        assert_eq!(cols, ["id", "name"]);
        assert_eq!(rows, [["1", "Smith, John"], ["2", "line\nbreak"]]);
    }

    #[test]
    fn csv_doubles_are_one_quote_and_empty_input_is_empty() {
        let (_, rows) = parse_csv("v\n\"say \"\"hi\"\"\"\n");
        assert_eq!(rows, [["say \"hi\""]]);
        assert_eq!(parse_csv(""), (Vec::new(), Vec::new()));
    }

    /// A statement returning no rows (an UPDATE, a SET) must not invent one.
    #[test]
    fn csv_header_only_yields_no_rows() {
        let (cols, rows) = parse_csv("id,name\n");
        assert_eq!(cols, ["id", "name"]);
        assert!(rows.is_empty());
    }

    #[test]
    fn tsv_unescapes_mysql_batch_output() {
        let (cols, rows) = parse_tsv("id\tnote\n1\ta\\nb\n");
        assert_eq!(cols, ["id", "note"]);
        assert_eq!(rows, [["1", "a\nb"]]);
    }

    /// The read-only guard must be on the connection, and the script must be
    /// **only** the caller's SQL.
    ///
    /// Both halves matter. Without the guard a read is silently read-write;
    /// with the guard in the *script*, psql prints a `SET` command tag that
    /// `--csv` writes to stdout, and the parser reads it as the header row —
    /// which shifted every column name down into the data. The smoke test
    /// caught that; this pins it.
    #[test]
    fn the_read_only_guard_rides_on_the_connection_not_the_script() {
        let t = DbCreds {
            engine: DbEngine::Postgres,
            database: "demo_api_stable".into(),
            user: "demo_api_stable".into(),
            password: "pw".into(),
        };
        let (cmd, env, stdin) = client_invocation(&t, "SELECT 1", false);
        assert_eq!(
            stdin.unwrap(),
            "SELECT 1",
            "nothing of ours may reach stdout"
        );
        assert!(env
            .iter()
            .any(|e| e == "PGOPTIONS=-c default_transaction_read_only=on"));
        assert!(env.iter().any(|e| e.starts_with("PGPASSWORD=")));
        // `-q` keeps command tags (`CREATE TABLE`) out of the parsed output too.
        assert!(cmd.contains(&"-q".to_string()));

        let (_, env, stdin) = client_invocation(&t, "INSERT INTO t VALUES (1)", true);
        assert_eq!(stdin.unwrap(), "INSERT INTO t VALUES (1)");
        assert!(
            !env.iter().any(|e| e.contains("read_only")),
            "write must not be guarded"
        );
    }

    /// MariaDB takes the same shape: guard at connect, script untouched.
    #[test]
    fn mariadb_guards_at_connect_time() {
        let t = DbCreds {
            engine: DbEngine::Mariadb,
            database: "demo_api_stable".into(),
            user: "demo_api_stable".into(),
            password: "pw".into(),
        };
        let (cmd, _, stdin) = client_invocation(&t, "SELECT 1", false);
        assert_eq!(stdin.unwrap(), "SELECT 1");
        assert!(cmd.iter().any(|a| a == "SET SESSION TRANSACTION READ ONLY"));
        let (cmd, _, _) = client_invocation(&t, "SELECT 1", true);
        assert!(!cmd.iter().any(|a| a.contains("READ ONLY")));
    }

    /// Production is the line where a read stops being a developer's business.
    #[test]
    fn production_needs_admin_other_classes_do_not() {
        assert_eq!(min_role(EnvClass::Production), Role::Admin);
        assert_eq!(min_role(EnvClass::Stable), Role::Developer);
        assert_eq!(min_role(EnvClass::Ephemeral), Role::Developer);
    }

    #[test]
    fn catalog_queries_are_engine_specific_and_refuse_what_they_cannot_do() {
        assert!(catalog_query(DbEngine::Postgres, "tables", None)
            .unwrap()
            .contains("information_schema.tables"));
        assert!(catalog_query(DbEngine::Postgres, "columns", None).is_err());
        assert!(catalog_query(DbEngine::Valkey, "columns", Some("x")).is_err());
        // A table name with a quote must not break out of the literal.
        let q = catalog_query(DbEngine::Postgres, "columns", Some("o'brien")).unwrap();
        assert!(q.contains("'o''brien'"));
    }

    #[test]
    fn audit_line_is_flattened_and_bounded() {
        assert_eq!(one_line("SELECT\n  1", 100), "SELECT 1");
        assert_eq!(one_line(&"x".repeat(400), 10).chars().count(), 10);
    }
}
