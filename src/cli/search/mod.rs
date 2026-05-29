pub mod notes;
pub mod web;

use clap::Subcommand;

#[derive(Subcommand)]
pub enum Search {
    /// Fetch a website and convert its content to markdown
    Web {
        /// The URL of the website to fetch
        url: String,
    },
    /// Search notes by term
    Notes {
        #[arg(long)]
        term: String,
        #[arg(long, default_value = "false")]
        vector: bool,
    },
}
