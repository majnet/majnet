//! One module per area of the API. Each exports plain `async fn`s taking the
//! shared [`crate::App`]; the command tree in `main.rs` is the only place that
//! knows about clap.

pub mod admin;
pub mod auth;
pub mod deploy;
pub mod guide;
pub mod read;
pub mod release;
pub mod run;
pub mod shell;
