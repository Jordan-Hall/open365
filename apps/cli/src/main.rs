//! `open365` CLI: `inspect` and `roundtrip --check`.

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use open365_opc::OpcError;
use open365_testkit::{inspect, roundtrip};

#[derive(Debug, Parser)]
#[command(name = "open365", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Print the parts, content types and integrity issues of a package.
    Inspect { file: PathBuf },
    /// Verify that files survive read -> write -> read unchanged.
    Roundtrip {
        /// Required: run the check without writing anything.
        #[arg(long, required = true)]
        check: bool,
        #[arg(required = true)]
        files: Vec<PathBuf>,
    },
}

#[derive(Debug)]
enum CliError {
    Io(PathBuf, std::io::Error),
    Opc(PathBuf, OpcError),
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(p, e) => write!(f, "{}: {e}", p.display()),
            Self::Opc(p, e) => write!(f, "{}: {e}", p.display()),
        }
    }
}

impl std::error::Error for CliError {}

fn read(path: &Path) -> Result<Vec<u8>, CliError> {
    std::fs::read(path).map_err(|e| CliError::Io(path.to_owned(), e))
}

fn run_inspect(file: &Path) -> Result<(), CliError> {
    let info = inspect(&read(file)?).map_err(|e| CliError::Opc(file.to_owned(), e))?;
    println!(
        "{:>10}  {:<8}  {:<60}  PART",
        "SIZE", "METHOD", "CONTENT-TYPE"
    );
    for p in &info.parts {
        println!(
            "{:>10}  {:<8}  {:<60}  {}",
            p.size,
            format!("{:?}", p.compression),
            p.content_type.as_deref().unwrap_or("-"),
            p.name
        );
    }
    println!(
        "{} parts, {} relationships, {} integrity issues",
        info.parts.len(),
        info.relationship_count,
        info.integrity_issues.len()
    );
    for issue in &info.integrity_issues {
        println!("  issue: {issue:?}");
    }
    Ok(())
}

/// Prints one line for `file`; returns whether it passed.
fn check_file(file: &Path) -> bool {
    let name = file.file_name().map_or_else(
        || file.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    let result =
        read(file).and_then(|b| roundtrip(&b).map_err(|e| CliError::Opc(file.to_owned(), e)));
    match result {
        Ok(r) if r.is_lossless() => {
            println!("PASS {name} parts={}", r.part_count);
            true
        }
        Ok(r) => {
            println!("FAIL {name}: {}", r.describe());
            false
        }
        Err(e) => {
            println!("FAIL {name}: {e}");
            false
        }
    }
}

fn main() -> ExitCode {
    match Cli::parse().command {
        Command::Inspect { file } => match run_inspect(&file) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        },
        Command::Roundtrip { files, .. } => {
            let results: Vec<bool> = files.iter().map(|f| check_file(f)).collect();
            let all_ok = results.iter().all(|&ok| ok);
            if all_ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
    }
}
