//! hqsh — Bashkit shell with the hq builtin registered.
//!
//! Usage:
//!   hqsh                  Interactive REPL
//!   hqsh -c <script>      Run a single command
//!   hqsh <file>           Run a script file

use bashkit::Bash;
use clap::Parser;
use hq::cli::bashkit::HqBuiltin;
use std::io::{self, BufRead, Write};

#[derive(Parser)]
#[command(name = "hqsh", version, about = "Bashkit shell with hq builtin")]
struct Args {
    /// Run a single command and exit
    #[arg(short = 'c')]
    command: Option<String>,

    /// Script file to execute
    script: Option<String>,
}

fn build_shell() -> Bash {
    Bash::builder()
        .builtin("hq", Box::new(HqBuiltin))
        .build()
}

#[tokio::main]
async fn main() -> io::Result<()> {
    let args = Args::parse();

    if let Some(cmd) = &args.command {
        let mut bash = build_shell();
        match bashkit_try_exec(&mut bash, cmd).await {
            Ok(output) => {
                print!("{output}");
                let _ = io::stdout().flush();
            }
            Err(e) => {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        }
        return Ok(());
    }

    if let Some(path) = &args.script {
        let contents =
            std::fs::read_to_string(path).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        let mut bash = build_shell();
        match bashkit_try_exec(&mut bash, &contents).await {
            Ok(output) => {
                print!("{output}");
                let _ = io::stdout().flush();
            }
            Err(e) => {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        }
        return Ok(());
    }

    // REPL mode
    println!("hqsh — type 'exit' or Ctrl-D to quit");
    let mut bash = build_shell();
    let stdin = io::stdin();
    let mut reader = stdin.lock();
    let mut line = String::new();

    loop {
        print!("hq> ");
        let _ = io::stdout().flush();
        line.clear();

        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if trimmed == "exit" || trimmed == "quit" {
                    break;
                }

                match bashkit_try_exec(&mut bash, trimmed).await {
                    Ok(output) => {
                        print!("{output}");
                        let _ = io::stdout().flush();
                    }
                    Err(e) => {
                        eprintln!("error: {e}");
                    }
                }
            }
            Err(e) => {
                eprintln!("read error: {e}");
                break;
            }
        }
    }

    Ok(())
}

/// Run a script through bashkit and return the captured stdout.
async fn bashkit_try_exec(bash: &mut Bash, script: &str) -> Result<String, String> {
    let result = bash
        .exec(script)
        .await
        .map_err(|e| format!("bashkit error: {e}"))?;

    let mut output = String::new();

    if !result.stdout.is_empty() {
        output.push_str(&result.stdout);
    }
    if !result.stderr.is_empty() {
        output.push_str(&result.stderr);
    }
    if result.exit_code != 0 && output.is_empty() {
        output.push_str(&format!("exit code: {}", result.exit_code));
    }

    Ok(output)
}
