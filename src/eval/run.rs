use std::path::Path;
use tokio_rusqlite::Connection;
use uuid::Uuid;
use crate::eval::{EvalCase, EvalExpected, EvalRun};
use crate::eval::db;
use crate::openai::{Message, Role, completion};

pub async fn load_cases_from_jsonl(path: &Path) -> anyhow::Result<Vec<EvalCase>> {
    let content = tokio::fs::read_to_string(path).await?;
    let mut cases = Vec::new();

    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let case: EvalCase = serde_json::from_str(line)?;
        cases.push(case);
    }

    Ok(cases)
}

fn check_expected(output: &str, expected: &EvalExpected) -> bool {
    let output_lower = output.to_lowercase();
    for term in &expected.contains {
        if !output_lower.contains(&term.to_lowercase()) {
            return false;
        }
    }
    true
}

pub async fn run_eval(
    conn: &Connection,
    file_path: &str,
    model: &str,
    api_key: &str,
    api_hostname: &str,
) -> anyhow::Result<EvalRun> {
    let path = Path::new(file_path);
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();

    let run_id = Uuid::new_v4().to_string();
    db::insert_run(conn, &run_id, &name, model).await?;

    let cases = load_cases_from_jsonl(path).await?;
    db::update_run_status(conn, &run_id, "running").await?;

    for case in cases {
        let result_id = Uuid::new_v4().to_string();
        let case_id = case.id.clone();

        let messages = vec![Message::new(Role::User, &case.prompt)];

        match completion(&messages, &None, api_hostname, api_key, model).await {
            Ok(response) => {
                let output = response["choices"]
                    .as_array()
                    .and_then(|arr| arr.first())
                    .and_then(|c| c["message"]["content"].as_str())
                    .unwrap_or("")
                    .to_string();

                let passed = check_expected(&output, &case.expected);
                db::insert_result(conn, &result_id, &run_id, &case_id, &case.prompt, Some(&output), passed, None).await?;
            }
            Err(e) => {
                db::insert_result(conn, &result_id, &run_id, &case_id, &case.prompt, None, false, Some(&e.to_string())).await?;
            }
        }
    }

    db::update_run_status(conn, &run_id, "completed").await?;

    Ok(db::get_run(conn, &run_id).await?.expect("eval run must exist after insertion"))
}

pub async fn print_results(conn: &Connection, run_id: &str) -> anyhow::Result<()> {
    let run = db::get_run(conn, run_id).await?;
    if let Some(r) = run {
        println!("\n=== Eval Run Summary ===");
        println!("Name: {}", r.name);
        println!("Model: {}", r.model);
        println!("Status: {:?}", r.status);

        if let (Some(started), Some(completed)) = (&r.started_at, &r.completed_at) {
            println!("Started: {}", started);
            println!("Completed: {}", completed);
        }
    }

    let results = db::get_run_results(conn, run_id).await?;
    let total = results.len();
    let passed_count = results.iter().filter(|r| r.passed).count();

    println!("\n=== Results ===");
    for result in &results {
        let status = if result.passed { "PASS" } else { "FAIL" };
        println!("[{}] {}", status, result.case_id);
        if let Some(error) = &result.error {
            println!("  Error: {}", error);
        }
    }

    println!("\nTotal: {} | Passed: {} | Failed: {}", total, passed_count, total - passed_count);

    Ok(())
}