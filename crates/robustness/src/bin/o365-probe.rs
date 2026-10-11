//! Child process for the hostile-input tests: reads a file, exercises one parser, prints the
//! outcome. A panic exits with code 101 and its location on stderr; a stack overflow or abort
//! shows up as a crash exit code. The parent enforces the time and memory limits.

use std::io::Write;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let (Some(kind), Some(path)) = (args.get(1), args.get(2)) else {
        eprintln!("usage: o365-probe <opc|docx|xlsx|package|formula> <file>");
        return ExitCode::from(2);
    };
    let Ok(bytes) = std::fs::read(path) else {
        eprintln!("cannot read {path}");
        return ExitCode::from(2);
    };
    let kind = kind.clone();
    // 8 MiB: the main-thread stack of common Linux desktops.
    let worker = std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024)
        .spawn(move || open365_robustness::exercise(&kind, &bytes))
        .expect("spawn worker");
    match worker.join() {
        Ok(Ok(note)) => println!("OK {note}"),
        Ok(Err(message)) => println!("ERR {}", message.chars().take(300).collect::<String>()),
        Err(_) => {
            let _ = std::io::stdout().flush();
            return ExitCode::from(101);
        }
    }
    ExitCode::SUCCESS
}
