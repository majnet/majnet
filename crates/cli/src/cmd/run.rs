//! Running things: a command inside an app's container, and SQL against its
//! managed database.
//!
//! Both go through the reconciler (the only component allowed to touch node
//! Docker APIs) and are gated by your project role — production needs admin,
//! everything else a developer, and a SQL *write* needs admin in every class.
//! Both are audited under your name.
//!
//! Two details that matter when scripting this:
//!
//!   * `exec` exits with the **container's** exit code, so `majnet exec … --
//!     test -f /app/config` works in an `if`. A transport failure exits 1 with
//!     a message on stderr, which is distinguishable because stdout is empty.
//!   * `sql` is **read-only unless you pass `--write`**. That guard is a
//!     transaction mode, not a sandbox — it stops a mistyped `UPDATE`, not a
//!     determined one. The real boundary is the role check.

use anyhow::{bail, Context as _, Result};
use clap::Args;
use serde_json::{json, Value};
use std::io::{IsTerminal, Read, Write};

use crate::client::{seg, Api};
use crate::output::{elide, emit, Table};
use crate::resolve;
use crate::App;

use super::read::{target, TargetArgs};

// ── exec ─────────────────────────────────────────────────────────────────────

#[derive(Args)]
pub struct ExecArgs {
    #[command(flatten)]
    pub target: TargetArgs,
    /// Run the command through `sh -c` instead of executing argv directly.
    #[arg(long)]
    pub shell: bool,
    /// Working directory inside the container.
    #[arg(long)]
    pub workdir: Option<String>,
    /// Feed the command stdin; `-` reads this process's stdin.
    #[arg(long, value_name = "TEXT")]
    pub stdin: Option<String>,
    /// The command, after `--`. e.g. `majnet exec demo api -- ls -la /app`
    ///
    /// Not marked `required`: clap forbids a required positional after the
    /// optional `project`/`app`, and an empty command is checked below with a
    /// message that shows the `--` people forget.
    #[arg(last = true, value_name = "CMD")]
    pub cmd: Vec<String>,
}

pub async fn exec(app: &App, args: &ExecArgs) -> Result<()> {
    if args.cmd.is_empty() {
        bail!(
            "no command given — it goes after `--`, e.g.\n               majnet exec demo api -c stable -- ls -la /app"
        );
    }
    let (org, name, class) = target(app, &args.target).await?;
    let line = args.cmd.join(" ");
    resolve::confirm(
        "run a command in",
        &format!("{org}/{name}"),
        &class,
        app.yes,
    )?;

    let cmd = if args.shell {
        vec!["sh".to_string(), "-c".to_string(), line.clone()]
    } else {
        args.cmd.clone()
    };
    let stdin = match args.stdin.as_deref() {
        Some("-") => {
            let mut buffer = String::new();
            std::io::stdin()
                .read_to_string(&mut buffer)
                .context("reading stdin")?;
            Some(buffer)
        }
        other => other.map(str::to_string),
    };

    // No `user`: the command runs as the image's user. Letting a caller pick
    // one would let a project developer run as root in the app container, which
    // is escalation past what the app itself runs as (see reconciler run.rs).
    let body = json!({
        "cmd": cmd,
        "stdin": stdin,
        "workdir": args.workdir,
    });
    let result: Value = app
        .patient_client()?
        .post_json(
            Api::Recon,
            &format!("/api/exec/{}/{}/{}", seg(&org), seg(&class), seg(&name)),
            &body,
        )
        .await?;

    if !app.table() {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        // Pass the streams through as the command wrote them — no framing, so
        // the output can be piped into whatever expected it.
        print!("{}", result["stdout"].as_str().unwrap_or_default());
        eprint!("{}", result["stderr"].as_str().unwrap_or_default());
        if result["truncated"].as_bool().unwrap_or(false) {
            eprintln!("majnet: output truncated at 1 MiB");
        }
        std::io::stdout().flush().ok();
    }

    // The container's verdict becomes ours, so `majnet exec` composes in shell
    // conditionals instead of always looking like it succeeded.
    let code = result["exit_code"].as_i64().unwrap_or(-1);
    if code != 0 {
        std::process::exit(code.clamp(1, 255) as i32);
    }
    Ok(())
}

// ── sql ──────────────────────────────────────────────────────────────────────

#[derive(Args)]
pub struct SqlArgs {
    #[command(flatten)]
    pub target: TargetArgs,
    /// The statement. Omit it to read from stdin, or to start a small REPL when
    /// stdin is a terminal.
    pub sql: Option<String>,
    /// Read the statement from a file.
    #[arg(long, value_name = "PATH", conflicts_with = "sql")]
    pub file: Option<String>,
    /// Allow the statement to write. Requires project admin in every class.
    #[arg(long)]
    pub write: bool,
    /// Rows to print (the statement still runs in full).
    #[arg(long, default_value_t = 200)]
    pub limit: usize,
    /// List the tables instead of running a statement.
    #[arg(long, conflicts_with_all = ["sql", "file"])]
    pub tables: bool,
    /// Describe one table's columns.
    #[arg(long, value_name = "TABLE", conflicts_with_all = ["sql", "file", "tables"])]
    pub columns: Option<String>,
}

pub async fn sql(app: &App, args: &SqlArgs) -> Result<()> {
    let (org, name, class) = target(app, &args.target).await?;

    let statement = match (&args.sql, &args.file, args.tables, &args.columns) {
        (_, _, true, _) | (_, _, _, Some(_)) => None,
        (Some(sql), _, _, _) => Some(sql.clone()),
        (_, Some(path), _, _) => {
            Some(std::fs::read_to_string(path).with_context(|| format!("reading {path}"))?)
        }
        // No statement given: a pipe means "the SQL is on stdin"; a terminal
        // means the person wants to poke around, so open the REPL.
        _ if !std::io::stdin().is_terminal() => {
            let mut buffer = String::new();
            std::io::stdin().read_to_string(&mut buffer)?;
            Some(buffer)
        }
        _ => return repl(app, &org, &name, &class, args).await,
    };

    if args.write {
        resolve::confirm(
            "run a WRITE statement against",
            &format!("{org}/{name}"),
            &class,
            app.yes,
        )?;
    }
    let meta = if args.tables {
        Some(("tables".to_string(), None))
    } else {
        args.columns
            .clone()
            .map(|t| ("columns".to_string(), Some(t)))
    };

    let result = query(
        app,
        &org,
        &name,
        &class,
        statement.as_deref(),
        args.write,
        args.limit,
        meta.as_ref(),
    )
    .await?;
    emit(app.format, &result, print_result)
}

/// One statement against one app's database.
#[allow(clippy::too_many_arguments)]
async fn query(
    app: &App,
    org: &str,
    name: &str,
    class: &str,
    statement: Option<&str>,
    write: bool,
    limit: usize,
    meta: Option<&(String, Option<String>)>,
) -> Result<Value> {
    let mut path = format!(
        "/api/sql/{}/{}/{}?limit={limit}",
        seg(org),
        seg(class),
        seg(name)
    );
    if write {
        path.push_str("&write=true");
    }
    if let Some((kind, table)) = meta {
        path.push_str(&format!("&meta={}", seg(kind)));
        if let Some(t) = table {
            path.push_str(&format!("&table={}", seg(t)));
        }
    }
    let sql = statement.unwrap_or("").to_string();
    if meta.is_none() && sql.trim().is_empty() {
        bail!("no statement given");
    }
    app.patient_client()?
        .post_json(Api::Recon, &path, &json!({ "sql": sql }))
        .await
}

fn print_result(value: &Value) {
    if let Some(raw) = value.get("raw").and_then(Value::as_str) {
        print!("{raw}");
        if !raw.ends_with('\n') {
            println!();
        }
        return;
    }
    let columns: Vec<String> = value
        .get("columns")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|c| c.as_str().unwrap_or_default().to_string())
                .collect()
        })
        .unwrap_or_default();
    if columns.is_empty() {
        // A statement that returns no result set (a write, a SET) is not an
        // empty table — say what happened instead of printing nothing.
        println!("ok — no rows returned");
    } else {
        let mut table = Table::new(&columns).empty_note("(0 rows)");
        for row in value
            .get("rows")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            table.push(
                row.as_array()
                    .map(|cells| {
                        cells
                            .iter()
                            .map(|c| c.as_str().unwrap_or_default().to_string())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default(),
            );
        }
        table.print();
        let count = value.get("row_count").and_then(Value::as_u64).unwrap_or(0);
        let shown = table.len();
        if count as usize != shown {
            println!("\n{count} row(s), {shown} shown — raise --limit for the rest");
        } else {
            println!("\n{count} row(s)");
        }
    }
    if let Some(notice) = value.get("notice").and_then(Value::as_str) {
        eprintln!("{}", notice.trim_end());
    }
}

/// A deliberately small REPL: enough to look around a database without
/// installing a client or opening a tunnel, and no more. Statements end at a
/// `;`, `\dt` lists tables, `\d <table>` describes one, `\q` leaves.
async fn repl(app: &App, org: &str, name: &str, class: &str, args: &SqlArgs) -> Result<()> {
    let info = app
        .client
        .get_value(
            Api::Recon,
            &format!("/api/db/{}/{}/{}", seg(org), seg(class), seg(name)),
        )
        .await?;
    let database = info["database"].as_str().unwrap_or(name).to_string();
    println!(
        "{} · {} ({class}) on {}",
        info["engine"].as_str().unwrap_or("?"),
        database,
        info["node"].as_str().unwrap_or("?")
    );
    println!(
        "read-only{}. \\dt tables · \\d <table> columns · \\q quit",
        if args.write { " OFF (--write)" } else { "" }
    );

    let mut buffer = String::new();
    loop {
        print!(
            "{}",
            if buffer.is_empty() {
                format!("{database}=> ")
            } else {
                "     -> ".into()
            }
        );
        std::io::stdout().flush().ok();
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line)? == 0 {
            println!();
            return Ok(());
        }
        let trimmed = line.trim();
        if buffer.is_empty() {
            match trimmed {
                "" => continue,
                "\\q" | "quit" | "exit" => return Ok(()),
                "\\dt" => {
                    show(
                        app,
                        org,
                        name,
                        class,
                        args,
                        None,
                        Some(("tables".into(), None)),
                    )
                    .await;
                    continue;
                }
                other if other.starts_with("\\d ") => {
                    let table = other[3..].trim().to_string();
                    show(
                        app,
                        org,
                        name,
                        class,
                        args,
                        None,
                        Some(("columns".into(), Some(table))),
                    )
                    .await;
                    continue;
                }
                _ => {}
            }
        }
        buffer.push_str(&line);
        // Wait for the terminator, so multi-line statements paste in cleanly.
        if !buffer.trim_end().ends_with(';') {
            continue;
        }
        let statement = std::mem::take(&mut buffer);
        show(app, org, name, class, args, Some(statement), None).await;
    }
}

/// Run and print inside the REPL — an error ends the statement, not the session.
async fn show(
    app: &App,
    org: &str,
    name: &str,
    class: &str,
    args: &SqlArgs,
    statement: Option<String>,
    meta: Option<(String, Option<String>)>,
) {
    match query(
        app,
        org,
        name,
        class,
        statement.as_deref(),
        args.write,
        args.limit,
        meta.as_ref(),
    )
    .await
    {
        Ok(value) => print_result(&value),
        Err(e) => eprintln!("{}", elide(&format!("{e:#}"), 2000)),
    }
}
