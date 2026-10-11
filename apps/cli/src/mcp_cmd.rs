//! `open365 mcp`: serves the document tools over stdio (see `docs/agents/mcp.md`).

use std::path::PathBuf;

use open365_mcp::{Config, Server};

use crate::CliError;

pub struct Options {
    pub roots: Vec<PathBuf>,
    pub audit_log: PathBuf,
    pub allow_edit: bool,
    pub allow_destructive: bool,
    pub include_hidden: bool,
    pub deadline: std::time::Duration,
}

pub fn run(opts: Options) -> Result<(), CliError> {
    let audit = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&opts.audit_log)
        .map_err(|e| CliError::Io(opts.audit_log.clone(), e))?;
    let config = Config {
        roots: opts.roots,
        allow_edit: opts.allow_edit,
        allow_destructive: opts.allow_destructive,
        include_hidden: opts.include_hidden,
        deadline: opts.deadline,
    };
    let mut server = Server::new(config, Box::new(audit)).map_err(CliError::Other)?;
    let stdin = std::io::stdin().lock();
    let stdout = std::io::stdout().lock();
    server
        .serve(stdin, stdout)
        .map_err(|e| CliError::Other(format!("stdio: {e}")))
}
