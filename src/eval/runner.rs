use std::path::Path;
use tokio_rusqlite::Connection;
use uuid::Uuid;
use anyhow::{anyhow, Result};

use crate::ai::chat::{ChatBuilder, InvisibleCharFilter};
use crate::eval::models::{EvalCase, EvalExpected, EvalRun};
use crate::eval::db::{get_run, get_run_results, insert_result, insert_run, update_run_status};
use crate::openai::{Message, Role};

pub async fn load_cases_from_jsonl(path: &Path) -> Result<Vec<EvalCase>> {
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

async fn run_case(
    api_hostname: &str,
    api_key: &str,
    model: &str,
    case: &EvalCase,
) -> Result<String> {
    let msg = Message::new(Role::User, &case.prompt);
    let mut chat = ChatBuilder::new(api_hostname, api_key, model)
        .transcript(vec![msg.clone()])
        .middleware(vec![Box::new(InvisibleCharFilter)])
        .build();

    let messages = chat.next_msg(msg).await?;

    messages
        .iter()
        .find(|m| m.role() == &Role::Assistant)
        .and_then(|m| m.content.clone())
        .ok_or_else(|| anyhow!("No assistant response for case {}", case.id))
}

pub async fn run_eval(
    db: &Connection,
    api_hostname: &str,
    api_key: &str,
    model: &str,
    file_path: &str,
) -> Result<EvalRun> {
    let path = Path::new(file_path);
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();

    let run_id = Uuid::new_v4().to_string();
    insert_run(db, &run_id, &name, model).await?;

    let cases = load_cases_from_jsonl(path).await?;
    update_run_status(db, &run_id, "running").await?;

    for case in cases {
        let result_id = Uuid::new_v4().to_string();
        let case_id = case.id.clone();

        tracing::info!("Running eval case: {}", case_id);

        match run_case(api_hostname, api_key, model, &case).await {
            Ok(output) => {
                let passed = check_expected(&output, &case.expected);
                if !passed {
                    tracing::warn!(
                        "Eval case FAILED: {} — expected to contain {:?}, got: {}",
                        case_id,
                        case.expected.contains,
                        output
                    );
                }
                insert_result(db, &result_id, &run_id, &case_id, &case.prompt, Some(&output), passed, None).await?;
            }
            Err(e) => {
                tracing::error!("Eval case ERROR: {} — {}", case_id, e);
                insert_result(db, &result_id, &run_id, &case_id, &case.prompt, None, false, Some(&e.to_string())).await?;
            }
        }
    }

    update_run_status(db, &run_id, "completed").await?;

    Ok(get_run(db, &run_id).await?.expect("eval run must exist after insertion"))
}

pub async fn run_eval_dry(
    api_hostname: &str,
    api_key: &str,
    model: &str,
    file_path: &str,
) -> Result<()> {
    let path = Path::new(file_path);
    let cases = load_cases_from_jsonl(path).await?;
    let mut total = 0;
    let mut passed_count = 0;

    println!("\n=== Eval Run Summary (dry run) ===");
    println!("Model: {}", model);
    println!("\n=== Results ===");

    for case in cases {
        let case_id = case.id.clone();
        tracing::info!("Running eval case: {}", case_id);

        match run_case(api_hostname, api_key, model, &case).await {
            Ok(output) => {
                let passed = check_expected(&output, &case.expected);
                total += 1;
                if passed {
                    passed_count += 1;
                } else {
                    tracing::warn!(
                        "Eval case FAILED: {} — expected to contain {:?}, got: {}",
                        case_id,
                        case.expected.contains,
                        output
                    );
                }
                let status = if passed { "PASS" } else { "FAIL" };
                println!("[{}] {}", status, case_id);
            }
            Err(e) => {
                total += 1;
                tracing::error!("Eval case ERROR: {} — {}", case_id, e);
                println!("[FAIL] {}", case_id);
                println!("  Error: {}", e);
            }
        }
    }

    println!("\nTotal: {} | Passed: {} | Failed: {}", total, passed_count, total - passed_count);

    Ok(())
}

pub async fn print_results(db: &Connection, run_id: &str) -> Result<()> {
    let run = get_run(db, run_id).await?;
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

    let results = get_run_results(db, run_id).await?;
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
