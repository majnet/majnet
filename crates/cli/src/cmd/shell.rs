//! An interactive shell, over the reconciler's terminal WebSocket (ADR 0016).
//!
//! Same endpoint the dashboard's terminal uses, so the same rules apply and are
//! worth stating plainly: it is **platform-admin only, and named** — the
//! header-less `infra` bypass is refused, because every session is recorded.
//! The full transcript is written on the node and the open/close are audit
//! events. `--direct` therefore cannot open a shell at all, which is correct.
//!
//! Two modes: a container shell in an app, or a root shell in a node's host
//! namespaces (a privileged helper plus `nsenter`). Sessions close themselves
//! after 15 minutes idle or 4 hours total.
//!
//! Locally this puts the terminal in raw mode so keystrokes reach the far end
//! unbuffered — including ctrl-c, which interrupts the *remote* command, not
//! this process. The mode is restored by a `Drop` guard, so it comes back even
//! if the connection dies mid-session; a shell that leaves your terminal
//! unusable is a worse bug than one that fails to connect.

use anyhow::{bail, Context as _, Result};
use clap::Args;
use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tungstenite::tungstenite::Message;

use crate::client::seg;
use crate::resolve;
use crate::App;

#[derive(Args)]
pub struct ShellArgs {
    /// Project name or GitHub org (container mode).
    pub project: Option<String>,
    /// App name (container mode).
    pub app: Option<String>,
    /// Environment class for the container.
    #[arg(short, long)]
    pub class: Option<String>,
    /// Open a root shell on a node's host instead of in a container.
    #[arg(long, conflicts_with_all = ["project", "app", "class"], value_name = "NODE")]
    pub node: Option<String>,
}

pub async fn shell(app: &App, args: &ShellArgs) -> Result<()> {
    if app.ctx.direct {
        bail!(
            "a terminal session must be attributable to a named platform admin, and --direct \
             sends no identity (the WG listeners are trusted by bind address, §12.1).\n\
             Connect through the dashboard URL instead: `majnet login`."
        );
    }
    if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        bail!(
            "`majnet shell` needs a terminal. For a scripted command use \
             `majnet exec … -- <cmd>`, which returns output and an exit code."
        );
    }

    let (query, what) = match &args.node {
        Some(node) => (
            format!("mode=host&node={}", seg(node)),
            format!("host shell on {node}"),
        ),
        None => {
            let org = resolve::org(&app.client, &app.ctx, args.project.as_deref()).await?;
            let name = resolve::app(&app.ctx, args.app.as_deref())?;
            let class = resolve::class(&app.ctx, args.class.as_deref())?;
            resolve::confirm("open a shell in", &format!("{org}/{name}"), &class, app.yes)?;
            (
                format!(
                    "mode=container&project={}&app={}&class={}",
                    seg(&org),
                    seg(&name),
                    seg(&class)
                ),
                format!("{org}/{name} ({class})"),
            )
        }
    };

    let url = app.ctx.terminal_ws(&query);
    eprintln!("majnet: connecting to {what} — this session is recorded");
    let (socket, _) = tokio_tungstenite::connect_async(&url)
        .await
        .map_err(|e| handshake_error(e, &url))?;
    let (mut tx, mut rx) = socket.split();

    // Raw mode from here on; the guard restores it however we leave.
    let _raw = RawMode::enable()?;
    if let Some((cols, rows)) = window_size() {
        let _ = tx.send(Message::Text(resize(cols, rows).into())).await;
    }

    let mut winch = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::window_change())
        .context("watching for terminal resizes")?;
    let mut stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    let mut buffer = [0u8; 4096];

    loop {
        tokio::select! {
            message = rx.next() => match message {
                Some(Ok(Message::Binary(bytes))) => {
                    stdout.write_all(&bytes).await?;
                    stdout.flush().await?;
                }
                Some(Ok(Message::Text(text))) => {
                    stdout.write_all(text.as_bytes()).await?;
                    stdout.flush().await?;
                }
                Some(Ok(Message::Close(_))) | None => break,
                Some(Ok(_)) => {}
                Some(Err(e)) => {
                    // Restore the terminal before complaining, or the message
                    // lands in a screen with no line discipline.
                    drop(_raw);
                    bail!("terminal connection lost: {e}");
                }
            },
            read = stdin.read(&mut buffer) => match read {
                Ok(0) => break,
                Ok(n) => {
                    if tx.send(Message::Binary(buffer[..n].to_vec().into())).await.is_err() {
                        break;
                    }
                }
                Err(e) => bail!("reading stdin: {e}"),
            },
            _ = winch.recv() => {
                if let Some((cols, rows)) = window_size() {
                    let _ = tx.send(Message::Text(resize(cols, rows).into())).await;
                }
            }
        }
    }
    let _ = tx.send(Message::Close(None)).await;
    Ok(())
}

/// The reconciler refuses a non-admin with a 403 and a sentence explaining why;
/// tungstenite hides that inside the handshake error, so dig it out.
fn handshake_error(e: tokio_tungstenite::tungstenite::Error, url: &str) -> anyhow::Error {
    use tokio_tungstenite::tungstenite::Error;
    if let Error::Http(response) = &e {
        let status = response.status();
        let body = response
            .body()
            .as_ref()
            .map(|b| String::from_utf8_lossy(b).trim().to_string())
            .unwrap_or_default();
        let hint = if status == 403 {
            "\nThe terminal is platform-admin only, and refuses an unidentified caller outright \
             — every session has to be attributable. `majnet whoami` shows what the control \
             plane thinks you are."
        } else {
            ""
        };
        return anyhow::anyhow!("terminal refused ({status}): {body}{hint}");
    }
    anyhow::Error::new(e).context(format!("opening {url}"))
}

fn resize(cols: u16, rows: u16) -> String {
    format!(r#"{{"resize":{{"cols":{cols},"rows":{rows}}}}}"#)
}

/// Current terminal size via `TIOCGWINSZ`. `None` when stdout isn't a tty, in
/// which case the far end keeps its 80×24 default.
fn window_size() -> Option<(u16, u16)> {
    let mut size: libc::winsize = unsafe { std::mem::zeroed() };
    let ok = unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut size) } == 0;
    (ok && size.ws_col > 0).then_some((size.ws_col, size.ws_row))
}

/// Puts the terminal in raw mode and restores the previous settings on drop.
struct RawMode(libc::termios);

impl RawMode {
    fn enable() -> Result<Self> {
        let mut original: libc::termios = unsafe { std::mem::zeroed() };
        if unsafe { libc::tcgetattr(libc::STDIN_FILENO, &mut original) } != 0 {
            bail!("cannot read terminal settings (is stdin a terminal?)");
        }
        let mut raw = original;
        unsafe { libc::cfmakeraw(&mut raw) };
        if unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &raw) } != 0 {
            bail!("cannot put the terminal into raw mode");
        }
        Ok(Self(original))
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &self.0) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The reconciler parses exactly this shape (`terminal.rs::parse_resize`);
    /// a stray space or a renamed field silently disables resizing.
    #[test]
    fn resize_matches_the_protocol_the_reconciler_parses() {
        assert_eq!(resize(120, 40), r#"{"resize":{"cols":120,"rows":40}}"#);
        let parsed: serde_json::Value = serde_json::from_str(&resize(80, 24)).unwrap();
        assert_eq!(parsed["resize"]["cols"], 80);
        assert_eq!(parsed["resize"]["rows"], 24);
    }
}
