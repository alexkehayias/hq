use anyhow::Result;
use std::env;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::core::AppConfig;
use crate::core::db::async_db;
use crate::jobs::{
    DailyAgenda, GenerateSessionTitles, PeriodicJob, ProcessEmail, ResearchMeetingAttendees,
};

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
pub enum JobId {
    ProcessEmail,
    ResearchMeetingAttendees,
    GenerateSessionTitles,
    DailyAgenda,
}

pub async fn run(id: JobId) -> Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| format!("{}=debug", env!("CARGO_CRATE_NAME")).into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let config = AppConfig::default();
    let db = async_db(&config.vec_db_path)
        .await
        .expect("Failed to connect to db");

    let job: Box<dyn PeriodicJob> = match id {
        JobId::ProcessEmail => Box::new(ProcessEmail),
        JobId::ResearchMeetingAttendees => Box::new(ResearchMeetingAttendees),
        JobId::GenerateSessionTitles => Box::new(GenerateSessionTitles),
        JobId::DailyAgenda => Box::new(DailyAgenda),
    };

    println!("Running job: {:?}", id);
    job.run_job(&config, &db).await;
    println!("Job completed");

    Ok(())
}
