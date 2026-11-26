use anyhow::{Context, Result, bail};
use std::io::Write;

use hq::ai::tools::python::sandbox::shared_sandbox;

/// Run Python code in the embedded pybox WASM sandbox, mirroring the `python`
/// command line: either an inline program (`-c`) or a script file, each
/// optionally followed by arguments exposed to the program as `sys.argv`. The
/// current working directory is mounted as the sandbox root (`/`), so scripts
/// can read and write files there and nowhere else.
pub async fn run(command: Option<String>, script: Vec<String>) -> Result<()> {
    let (code, argv) = resolve_program(command, script)?;
    let workspace = std::env::current_dir().context("failed to read current directory")?;

    // The sandbox is CPU-bound and can run up to the timeout, so keep it off
    // the async runtime's worker threads.
    let runtime = tokio::runtime::Handle::current();
    let result = tokio::task::spawn_blocking(move || {
        let sandbox = shared_sandbox()?;
        runtime.block_on(sandbox.exec(&code, &workspace, b"", &argv))
    })
    .await?;

    match result {
        Ok(exec) => {
            let mut stdout = std::io::stdout();
            stdout.write_all(exec.stdout.as_bytes())?;
            stdout.flush()?;

            let mut stderr = std::io::stderr();
            stderr.write_all(exec.stderr.as_bytes())?;
            stderr.flush()?;

            Ok(())
        }
        Err(e) => {
            // A Python traceback from the guest. Print it raw and exit non-zero,
            // like the `python` command, rather than wrapping it as a CLI error.
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}

/// Resolve the code to run and its `sys.argv` from the `-c`/script inputs.
///
/// With `-c`, every positional is a program argument. Otherwise the first
/// positional names the script and the rest are its arguments, matching
/// CPython's `sys.argv[0]` conventions.
fn resolve_program(command: Option<String>, script: Vec<String>) -> Result<(String, Vec<String>)> {
    match command {
        Some(code) => Ok((code, argv_with("-c", script))),
        None => {
            let Some((file, args)) = script.split_first() else {
                bail!("no code provided: pass -c <code> or a script file");
            };
            let code =
                std::fs::read_to_string(file).with_context(|| format!("failed to read {file}"))?;
            Ok((code, argv_with(file, args.to_vec())))
        }
    }
}

/// Build a CPython-shaped `sys.argv`: the program name followed by its args.
fn argv_with(program: &str, args: Vec<String>) -> Vec<String> {
    std::iter::once(program.to_string()).chain(args).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn inline_command_is_used_verbatim() {
        let (code, argv) = resolve_program(Some("1 + 1".to_string()), vec![]).unwrap();
        assert_eq!(code, "1 + 1");
        assert_eq!(argv, vec!["-c"]);
    }

    #[test]
    fn inline_command_takes_arguments() {
        let (_, argv) = resolve_program(
            Some("pass".to_string()),
            vec!["first".to_string(), "second".to_string()],
        )
        .unwrap();
        assert_eq!(argv, vec!["-c", "first", "second"]);
    }

    #[test]
    fn file_contents_are_read() {
        let path = std::env::temp_dir().join(format!("hq-python-{}.py", Uuid::new_v4()));
        std::fs::write(&path, "print('hi')").unwrap();

        let (code, argv) = resolve_program(None, vec![path.display().to_string()]).unwrap();
        std::fs::remove_file(&path).ok();

        assert_eq!(code, "print('hi')");
        assert_eq!(argv, vec![path.display().to_string()]);
    }

    #[test]
    fn file_takes_arguments() {
        let path = std::env::temp_dir().join(format!("hq-python-{}.py", Uuid::new_v4()));
        std::fs::write(&path, "print('hi')").unwrap();

        let (_, argv) =
            resolve_program(None, vec![path.display().to_string(), "first".to_string()]).unwrap();
        std::fs::remove_file(&path).ok();

        assert_eq!(argv, vec![path.display().to_string(), "first".to_string()]);
    }

    #[test]
    fn rejects_no_input() {
        assert!(resolve_program(None, vec![]).is_err());
    }
}
