//! Repro for the race between concurrent `hq tasks ...` calls made from the
//! bash tool inside the agent loop.
//!
//! The agent loop runs tool calls concurrently (`try_join_all` in
//! `Chat::handle_tool_calls`), and the bash tool's `hq` builtin calls
//! `cli::run_with_args` in-process. So several `hq tasks create` invocations
//! share one process: process-global file locks, Tantivy's writer lock, and
//! the process-wide stdout/stderr redirection in `capture_async` all see
//! concurrent use.

use hq::ai::tools::run_in_sandbox;
use std::path::Path;
use std::time::Duration;
use tempfile::TempDir;

async fn init_storage(storage: &Path) {
    let notes = storage.join("notes");
    let projects = notes.join("projects");
    let index = storage.join("index");
    let db = storage.join("db");
    for dir in [&notes, &projects, &index, &db] {
        tokio::fs::create_dir_all(dir).await.unwrap();
    }

    let conn = hq::core::db::async_db(db.to_str().unwrap()).await.unwrap();
    conn.call(|conn| {
        hq::core::db::initialize_db(conn).unwrap();
        Ok(())
    })
    .await
    .unwrap();
}

/// Isolate the file/DB/index layer: invoke the CLI in-process exactly as the
/// `hq` builtin does, but without the process-wide stdout capture.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[serial_test::serial]
async fn concurrent_hq_tasks_create_via_cli() {
    let dir = TempDir::new().unwrap();
    let storage = dir.path().to_path_buf();
    init_storage(&storage).await;

    unsafe {
        std::env::set_var("HQ_STORAGE_PATH", &storage);
    }

    let count = 8;
    let futures = (0..count).map(|i| async move {
        let args = vec![
            "hq".to_string(),
            "tasks".to_string(),
            "create".to_string(),
            format!("Task {i}"),
        ];
        tokio::time::timeout(Duration::from_secs(20), hq::cli::run_with_args(args)).await
    });

    let results: Vec<_> = futures_util::future::join_all(futures).await;

    unsafe {
        std::env::remove_var("HQ_STORAGE_PATH");
    }

    let mut errors = Vec::new();
    for (i, result) in results.into_iter().enumerate() {
        match result {
            Ok(Ok(())) => {}
            Ok(Err(e)) => errors.push(format!("task {i}: {e}")),
            Err(_) => errors.push(format!("task {i}: timed out")),
        }
    }
    assert!(
        errors.is_empty(),
        "concurrent creates failed:\n{}",
        errors.join("\n")
    );

    let capture_path = storage.join("notes").join("projects").join("capture.org");
    let capture = tokio::fs::read_to_string(&capture_path).await.unwrap();
    for i in 0..count {
        let needle = format!("Task {i}\n");
        assert_eq!(
            capture.matches(&needle).count(),
            1,
            "Task {i} should appear exactly once in capture.org, got:\n{capture}"
        );
    }
}

/// Concurrent creates into a *brand-new* project. `run_create` looks up the
/// project and creates the file outside the file lock, unlike `run_refile`.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[serial_test::serial]
async fn concurrent_hq_tasks_create_new_project_via_cli() {
    let dir = TempDir::new().unwrap();
    let storage = dir.path().to_path_buf();
    init_storage(&storage).await;

    unsafe {
        std::env::set_var("HQ_STORAGE_PATH", &storage);
    }

    let count = 8;
    let futures = (0..count).map(|i| async move {
        let args = vec![
            "hq".to_string(),
            "tasks".to_string(),
            "create".to_string(),
            format!("Task {i}"),
            "--project".to_string(),
            "newproj".to_string(),
        ];
        tokio::time::timeout(Duration::from_secs(20), hq::cli::run_with_args(args)).await
    });

    let results: Vec<_> = futures_util::future::join_all(futures).await;

    unsafe {
        std::env::remove_var("HQ_STORAGE_PATH");
    }

    let mut errors = Vec::new();
    for (i, result) in results.into_iter().enumerate() {
        match result {
            Ok(Ok(())) => {}
            Ok(Err(e)) => errors.push(format!("task {i}: {e}")),
            Err(_) => errors.push(format!("task {i}: timed out")),
        }
    }

    let projects_dir = storage.join("notes").join("projects");
    let mut entries = tokio::fs::read_dir(&projects_dir).await.unwrap();
    let mut project_path = None;
    while let Some(entry) = entries.next_entry().await.unwrap() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.contains("--project-newproj") {
            project_path = Some(entry.path());
        }
    }
    let project_path = project_path.expect("project file should exist");
    let content = tokio::fs::read_to_string(&project_path).await.unwrap();
    let mut lost = Vec::new();
    for i in 0..count {
        let needle = format!("Task {i}\n");
        if content.matches(&needle).count() != 1 {
            lost.push(i);
        }
    }
    assert!(
        errors.is_empty() && lost.is_empty(),
        "concurrent creates failed:\n{}\nlost tasks: {lost:?}\nfile:\n{content}",
        errors.join("\n")
    );
}

/// Full path through the bash tool, including `capture_async`'s process-wide
/// fd redirection.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[serial_test::serial]
async fn concurrent_hq_tasks_create_via_bash() {
    let dir = TempDir::new().unwrap();
    let storage = dir.path().to_path_buf();
    init_storage(&storage).await;

    unsafe {
        std::env::set_var("HQ_STORAGE_PATH", &storage);
    }

    let workspace = storage.join("workspace").join("session");
    tokio::fs::create_dir_all(&workspace).await.unwrap();

    // capture_async redirects the process's fd 1/2, so the test harness's own
    // panic output can be swallowed if this regresses. Write diagnostics to a
    // file instead.
    let panic_path =
        std::env::temp_dir().join(format!("hq_bash_race_panic_{}.txt", std::process::id()));
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = std::fs::write(&panic_path, format!("{info}"));
        prev_hook(info);
    }));

    let count = 8;
    let futures = (0..count).map(|i| {
        let workspace = workspace.clone();
        async move {
            let command = format!("hq tasks create 'Task {i}'");
            tokio::time::timeout(
                Duration::from_secs(20),
                run_in_sandbox(&command, &workspace),
            )
            .await
        }
    });

    let results: Vec<_> = futures_util::future::join_all(futures).await;

    unsafe {
        std::env::remove_var("HQ_STORAGE_PATH");
    }

    let mut errors = Vec::new();
    let mut report = String::new();
    for (i, result) in results.into_iter().enumerate() {
        let line = match result {
            Ok(Ok(output)) => {
                if output.exit_code != 0 {
                    errors.push(format!("task {i}: exit {}", output.exit_code));
                }
                format!(
                    "task {i}: exit {} stdout={:?} stderr={:?}",
                    output.exit_code, output.stdout, output.stderr
                )
            }
            Ok(Err(e)) => {
                errors.push(format!("task {i}: error {e}"));
                format!("task {i}: error {e}")
            }
            Err(_) => {
                errors.push(format!("task {i}: timed out"));
                format!("task {i}: timed out")
            }
        };
        report.push_str(&line);
        report.push('\n');
    }
    let report_path =
        std::env::temp_dir().join(format!("hq_bash_race_report_{}.txt", std::process::id()));
    let _ = std::fs::write(report_path, &report);
    assert!(
        errors.is_empty(),
        "concurrent creates failed:\n{}",
        errors.join("\n")
    );

    let capture_path = storage.join("notes").join("projects").join("capture.org");
    let capture = tokio::fs::read_to_string(&capture_path).await.unwrap();
    for i in 0..count {
        let needle = format!("Task {i}\n");
        assert_eq!(
            capture.matches(&needle).count(),
            1,
            "Task {i} should appear exactly once in capture.org, got:\n{capture}"
        );
    }
}
