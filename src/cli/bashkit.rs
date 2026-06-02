//! Bashkit builtin for hq commands.
//!
//! Provides an `hq` builtin that makes all hq CLI subcommands
//! available within a bashkit shell session.

use bashkit::{async_trait, Builtin, BuiltinContext, ExecResult};
use os_pipe::pipe;
use std::io::Read;
use std::os::unix::io::AsRawFd;

/// The `hq` bashkit builtin.
///
/// Dispatches to hq CLI handlers based on the subcommand in args[0].
pub struct HqBuiltin;

#[async_trait]
impl Builtin for HqBuiltin {
    fn llm_hint(&self) -> Option<&'static str> {
        Some(
            "hq: Personal AI assistant and productivity platform. \
             Subcommands: init, migrate, serve, index, rebuild, query, \
             chat, auth, job, eval. Use 'hq help' for details.",
        )
    }

    async fn execute(&self, ctx: BuiltinContext<'_>) -> bashkit::Result<ExecResult> {
        let args = ctx.args;
        if args.is_empty() {
            return Ok(ExecResult::err(
                "hq: missing subcommand\nTry 'hq help' for more information.\n",
                1,
            ));
        }

        let subcommand = &args[0];
        let sub_args = &args[1..];

        let storage_path = ctx
            .env
            .get("HQ_STORAGE_PATH")
            .map(|s| s.as_str())
            .unwrap_or("./");
        let index_path = format!("{storage_path}/index");
        let notes_path = format!("{storage_path}/notes");
        let vec_db_path = format!("{storage_path}/db");

        match subcommand.as_str() {
            "help" | "--help" | "-h" => help(),
            "serve" => cmd_serve(sub_args).await,
            "query" => cmd_query(sub_args, &index_path, &vec_db_path).await,
            "chat" => cmd_chat(&vec_db_path).await,
            "index" => cmd_index(sub_args, &index_path, &notes_path, &vec_db_path).await,
            "rebuild" => cmd_rebuild(&index_path, &notes_path, &vec_db_path).await,
            "init" => cmd_init(sub_args, &vec_db_path, &index_path, &notes_path).await,
            "migrate" => cmd_migrate(sub_args, &vec_db_path, &index_path).await,
            "auth" => cmd_auth(sub_args, &vec_db_path).await,
            "job" => cmd_job(sub_args).await,
            "eval" => cmd_eval(sub_args).await,
            other => Ok(ExecResult::err(
                format!("hq: unknown subcommand '{other}'\nTry 'hq help' for more information.\n"),
                1,
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// Subcommand handlers
// ---------------------------------------------------------------------------

fn help() -> bashkit::Result<ExecResult> {
    let text = "\
hq — Personal AI assistant and productivity platform

Usage: hq <subcommand> [options]

Subcommands:
  init          Initialize indices and clone notes from version control
                Flags: --db, --index, --notes
  migrate       Migrate indices and db schema
                Flags: --db, --index
  serve         Run the server
                Flags: --host (default: 127.0.0.1), --port (default: 2222)
  index         Index notes
                Flags: --all, --full-text, --vector, --chat
  rebuild       Rebuild all indices from source
  query         Query the search index
                Flags: --term <query> (required), --vector
  chat          Start a chat bot session
  auth          Perform OAuth and store credentials
                Flags: --service <gmail>
  job           Run a job
                Flags: --id <process-email|research-meeting-attendees|generate-session-titles|daily-agenda>
  eval          Run an eval
                Flags: --file <path> (required), --model, --dry-run
  help          Show this help message
";
    Ok(ExecResult::ok(text))
}

async fn cmd_serve(args: &[String]) -> bashkit::Result<ExecResult> {
    let host = parse_flag(args, "--host").unwrap_or("127.0.0.1");
    let port = parse_flag(args, "--port").unwrap_or("2222");
    // serve is long-running; let output go to terminal directly
    crate::cli::serve::run(host.to_string(), port.to_string()).await;
    Ok(ExecResult::ok(""))
}

async fn cmd_query(args: &[String], index_path: &str, vec_db_path: &str) -> bashkit::Result<ExecResult> {
    let term = parse_flag(args, "--term").ok_or_else(|| {
        bashkit::Error::parse("hq query: --term <query> is required")
    })?;
    let vector = has_flag(args, "--vector");

    let index_path = index_path.to_owned();
    let vec_db_path = vec_db_path.to_owned();
    let term = term.to_owned();

    let output = capture_stdout(move || {
        Box::pin(async move {
            let _ = crate::cli::query::run(term, vector, &index_path, &vec_db_path).await;
        })
    })
    .await
    .map_err(|e| bashkit::Error::parse(format!("hq query: {e}")))?;

    Ok(ExecResult::ok(output))
}

async fn cmd_chat(vec_db_path: &str) -> bashkit::Result<ExecResult> {
    let path = vec_db_path.to_owned();
    let _ = crate::cli::chat::run(&path).await;
    Ok(ExecResult::ok(""))
}

async fn cmd_index(
    args: &[String],
    index_path: &str,
    notes_path: &str,
    vec_db_path: &str,
) -> bashkit::Result<ExecResult> {
    let all = has_flag(args, "--all");
    let full_text = has_flag(args, "--full-text");
    let vector = has_flag(args, "--vector");
    let chat = has_flag(args, "--chat");

    if !all && !full_text && !vector && !chat {
        return Ok(ExecResult::err(
            "hq index: requires at least one of --all, --full-text, --vector, --chat\n",
            1,
        ));
    }

    let index_path = index_path.to_owned();
    let notes_path = notes_path.to_owned();
    let vec_db_path = vec_db_path.to_owned();

    capture_stdout(move || {
        Box::pin(async move {
            let _ = crate::cli::index::run(all, full_text, vector, chat, &index_path, &notes_path, &vec_db_path).await;
        })
    })
    .await
    .map_err(|e| bashkit::Error::parse(format!("hq index: {e}")))?;

    Ok(ExecResult::ok(""))
}

async fn cmd_rebuild(index_path: &str, notes_path: &str, vec_db_path: &str) -> bashkit::Result<ExecResult> {
    let index_path = index_path.to_owned();
    let notes_path = notes_path.to_owned();
    let vec_db_path = vec_db_path.to_owned();

    capture_stdout(move || {
        Box::pin(async move {
            let _ = crate::cli::rebuild::run(&index_path, &notes_path, &vec_db_path).await;
        })
    })
    .await
    .map_err(|e| bashkit::Error::parse(format!("hq rebuild: {e}")))?;

    Ok(ExecResult::ok(""))
}

async fn cmd_init(
    args: &[String],
    vec_db_path: &str,
    index_path: &str,
    notes_path: &str,
) -> bashkit::Result<ExecResult> {
    let db = has_flag(args, "--db");
    let index = has_flag(args, "--index");
    let notes = has_flag(args, "--notes");

    if !db && !index && !notes {
        return Ok(ExecResult::err(
            "hq init: requires at least one of --db, --index, --notes\n",
            1,
        ));
    }

    let vec_db_path = vec_db_path.to_owned();
    let index_path = index_path.to_owned();
    let notes_path = notes_path.to_owned();

    capture_stdout(move || {
        Box::pin(async move {
            let _ = crate::cli::init::run(db, index, notes, &vec_db_path, &index_path, &notes_path).await;
        })
    })
    .await
    .map_err(|e| bashkit::Error::parse(format!("hq init: {e}")))?;

    Ok(ExecResult::ok(""))
}

async fn cmd_migrate(
    args: &[String],
    vec_db_path: &str,
    index_path: &str,
) -> bashkit::Result<ExecResult> {
    let db = has_flag(args, "--db");
    let index = has_flag(args, "--index");

    let vec_db_path = vec_db_path.to_owned();
    let index_path = index_path.to_owned();

    capture_stdout(move || {
        Box::pin(async move {
            let _ = crate::cli::migrate::run(db, index, &vec_db_path, &index_path).await;
        })
    })
    .await
    .map_err(|e| bashkit::Error::parse(format!("hq migrate: {e}")))?;

    Ok(ExecResult::ok(""))
}

async fn cmd_auth(args: &[String], vec_db_path: &str) -> bashkit::Result<ExecResult> {
    let service_str = parse_flag(args, "--service").ok_or_else(|| {
        bashkit::Error::parse("hq auth: --service <gmail> is required")
    })?;

    let service = match service_str {
        "gmail" => crate::cli::auth::ServiceKind::Gmail,
        other => {
            return Ok(ExecResult::err(
                format!("hq auth: unknown service '{other}'\n"),
                1,
            ));
        }
    };

    let vec_db_path = vec_db_path.to_owned();
    let _ = crate::cli::auth::run(service, &vec_db_path).await;
    Ok(ExecResult::ok(""))
}

async fn cmd_job(args: &[String]) -> bashkit::Result<ExecResult> {
    let id_str = parse_flag(args, "--id").ok_or_else(|| {
        bashkit::Error::parse("hq job: --id <job-id> is required")
    })?;

    let id = match id_str {
        "process-email" => crate::cli::job::JobId::ProcessEmail,
        "research-meeting-attendees" => crate::cli::job::JobId::ResearchMeetingAttendees,
        "generate-session-titles" => crate::cli::job::JobId::GenerateSessionTitles,
        "daily-agenda" => crate::cli::job::JobId::DailyAgenda,
        other => {
            return Ok(ExecResult::err(
                format!(
                    "hq job: unknown job '{other}'. \
                     Valid: process-email, research-meeting-attendees, \
                     generate-session-titles, daily-agenda\n"
                ),
                1,
            ));
        }
    };

    let output = capture_stdout(move || {
        Box::pin(async move {
            let _ = crate::cli::job::run(id).await;
        })
    })
    .await
    .map_err(|e| bashkit::Error::parse(format!("hq job: {e}")))?;

    Ok(ExecResult::ok(output))
}

async fn cmd_eval(args: &[String]) -> bashkit::Result<ExecResult> {
    let file = parse_flag(args, "--file").ok_or_else(|| {
        bashkit::Error::parse("hq eval: --file <path> is required")
    })?;
    let model = parse_flag(args, "--model");
    let dry_run = has_flag(args, "--dry-run");

    let db_path = std::env::var("HQ_STORAGE_PATH").unwrap_or_else(|_| "./".to_string());
    let vec_db_path = format!("{db_path}/db");
    let api_key =
        std::env::var("OPENAI_API_KEY").unwrap_or_else(|_| "thiswontworkforopenai".to_string());
    let api_hostname = std::env::var("HQ_LOCAL_LLM_HOST")
        .unwrap_or_else(|_| "https://api.openai.com".to_string());
    let model = model
        .map(|s| s.to_string())
        .unwrap_or_else(|| std::env::var("HQ_LOCAL_LLM_MODEL").expect("Missing model name"));

    let file = file.to_owned();

    let output = capture_stdout(move || {
        Box::pin(async move {
            let _ =
                crate::cli::eval::run(vec_db_path, api_hostname, api_key, model, file, dry_run)
                    .await;
        })
    })
    .await
    .map_err(|e| bashkit::Error::parse(format!("hq eval: {e}")))?;

    Ok(ExecResult::ok(output))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn parse_flag<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.windows(2)
        .find(|w| w[0] == flag)
        .map(|w| w[1].as_str())
}

fn has_flag(args: &[String], flag: &str) -> bool {
    args.iter().any(|a| a == flag)
}

/// Capture stdout produced during execution of an async closure.
///
/// Creates a pipe, redirects the process's stdout fd to the write end,
/// runs the future to completion, restores the original stdout, and
/// returns everything written to the pipe.
async fn capture_stdout<F, Fut>(f: F) -> std::io::Result<String>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let (mut reader, writer) = pipe()?;
    let writer_fd = writer.as_raw_fd();

    // Save original stdout
    let saved_stdout = unsafe { libc::dup(libc::STDOUT_FILENO) };
    if saved_stdout < 0 {
        return Err(std::io::Error::last_os_error());
    }

    // Redirect stdout → writer
    unsafe {
        libc::dup2(writer_fd, libc::STDOUT_FILENO);
    }
    // writer handle is dropped here → closes write end so reader sees EOF

    drop(writer);

    // Run the future
    f().await;

    // Restore original stdout
    unsafe {
        libc::dup2(saved_stdout, libc::STDOUT_FILENO);
        libc::close(saved_stdout);
    }

    // Read captured output
    let mut buf = String::new();
    reader.read_to_string(&mut buf)?;
    Ok(buf)
}
