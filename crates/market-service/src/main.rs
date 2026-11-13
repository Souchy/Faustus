use std::{net::SocketAddr, path::PathBuf};

use axum::http::HeaderValue;
use clap::{Parser, Subcommand};
use market_service::{ServiceState, cors_layer, economy, router, run_periodic_gem_collector};
use market_store::SqliteStore;

#[derive(Debug, Parser)]
#[command(name = "market-service")]
struct Args {
    #[arg(
        long,
        env = "FAUST_DATABASE",
        default_value = "faust.sqlite3",
        global = true
    )]
    database: PathBuf,
    #[arg(long, global = true)]
    economy_database: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    BackfillConfidence {
        #[arg(long)]
        name: String,
        #[arg(long)]
        file: PathBuf,
    },
    Serve {
        #[arg(long, default_value = "127.0.0.1:8787")]
        listen: SocketAddr,
        #[arg(long, default_value = "Faust/0.1.0")]
        user_agent: String,
        #[arg(long, default_values = ["tauri://localhost", "http://tauri.localhost", "https://tauri.localhost"])]
        allow_origin: Vec<HeaderValue>,
    },
    IngestOnce {
        #[arg(long)]
        league: String,
        #[arg(long, default_value = "Faust/0.1.0")]
        user_agent: String,
    },
    Import {
        #[arg(long)]
        name: String,
        #[arg(long)]
        file: Option<PathBuf>,
        #[arg(long)]
        replace: bool,
        #[arg(long, default_value = "Faust/0.1.0")]
        user_agent: String,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let store = SqliteStore::connect(&args.database).await?;
    let economy_store = if let Some(path) = &args.economy_database {
        SqliteStore::connect_economy(path).await?
    } else {
        store.clone()
    };

    match args.command {
        Command::BackfillConfidence { name, file } => {
            let count = economy::imports::backfill_confidence(&economy_store, &name, file)
                .await
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            println!(
                "{name}: restored {count} price-guarded confidence records; prices were not modified."
            );
        }
        Command::Serve {
            listen,
            user_agent,
            allow_origin,
        } => {
            let listener = tokio::net::TcpListener::bind(listen).await?;
            println!("Market service listening on http://{listen}");
            let (selected_league, selected_league_rx) = tokio::sync::watch::channel(None);
            let state = ServiceState::new(store.clone(), user_agent.clone(), selected_league)
                .with_economy_store(economy_store.clone());
            economy::recover_imports(&state).await?;
            let gem_collector = tokio::spawn(run_periodic_gem_collector(
                state.clone(),
                selected_league_rx.clone(),
            ));
            let collector = tokio::spawn(economy::collect(state.clone(), selected_league_rx));
            let result = axum::serve(listener, router(state).layer(cors_layer(allow_origin)))
                .with_graceful_shutdown(shutdown_signal())
                .await;
            collector.abort();
            gem_collector.abort();
            result?;
        }
        Command::IngestOnce { league, user_agent } => {
            let (selected, _) = tokio::sync::watch::channel(None);
            let state = ServiceState::new(store, user_agent, selected)
                .with_economy_store(economy_store.clone());
            economy::ingest_economy_once(state, &league)
                .await
                .map_err(std::io::Error::other)?;
            println!("Recorded current poe.ninja economy snapshots for {league}.");
        }
        Command::Import {
            name,
            file,
            replace,
            user_agent,
        } => {
            let (selected, _) = tokio::sync::watch::channel(None);
            let state = ServiceState::new(store.clone(), user_agent, selected)
                .with_economy_store(economy_store.clone());
            let entry = if file.is_some() {
                if name.is_empty()
                    || name.len() > 100
                    || name
                        .chars()
                        .any(|c| c.is_control() || matches!(c, '/' | '\\'))
                {
                    return Err("Invalid local archive name.".into());
                }
                market_core::economy::Archive {
                    name: name.clone(),
                    status: "available".into(),
                    rows: 0,
                    skipped: 0,
                    downloaded_bytes: 0,
                    download_total_bytes: None,
                    total_rows: None,
                    error: None,
                    start: None,
                    end: None,
                    league: None,
                    progress: String::new(),
                    license: "poe.ninja CC BY-SA".into(),
                    url: economy::imports::download_url(&name),
                }
            } else {
                economy::archive_catalog(&state, false)
                    .await
                    .map_err(|e| std::io::Error::other(e.1))?
                    .into_iter()
                    .find(|entry| entry.name == name)
                    .ok_or("Archive is not in the published catalog.")?
            };
            let result = economy::imports::import_archive(&state, entry, file, replace)
                .await
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            println!(
                "{}: {} observations, {} excluded, {} through {}; {}",
                result.name,
                result.rows,
                result.skipped,
                result.start.as_deref().unwrap_or("-"),
                result.end.as_deref().unwrap_or("-"),
                result.progress
            );
        }
    }

    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn database_accepts_one_shared_path_and_declares_its_environment_default() {
        use clap::CommandFactory;
        let command = Args::command();
        let database = command
            .get_arguments()
            .find(|argument| argument.get_id() == "database")
            .unwrap();
        assert_eq!(
            database.get_env(),
            Some(std::ffi::OsStr::new("FAUST_DATABASE"))
        );
        let args = Args::try_parse_from([
            "market-service",
            "--database",
            r"D:\Faust\faust.sqlite3",
            "serve",
        ])
        .unwrap();
        assert_eq!(args.database, PathBuf::from(r"D:\Faust\faust.sqlite3"));
        assert!(args.economy_database.is_none());
    }

    #[test]
    fn serve_defaults_allow_only_desktop_origins() {
        let args = Args::try_parse_from(["market-service", "serve"]).unwrap();
        let Command::Serve { allow_origin, .. } = args.command else {
            panic!("expected serve command");
        };
        assert_eq!(
            allow_origin,
            [
                HeaderValue::from_static("tauri://localhost"),
                HeaderValue::from_static("http://tauri.localhost"),
                HeaderValue::from_static("https://tauri.localhost"),
            ]
        );
    }

    #[test]
    fn explicit_origins_replace_defaults_and_can_be_repeated() {
        let args = Args::try_parse_from([
            "market-service",
            "serve",
            "--allow-origin",
            "https://market.example",
            "--allow-origin",
            "http://tauri.localhost",
        ])
        .unwrap();
        let Command::Serve { allow_origin, .. } = args.command else {
            panic!("expected serve command");
        };
        assert_eq!(
            allow_origin,
            [
                HeaderValue::from_static("https://market.example"),
                HeaderValue::from_static("http://tauri.localhost"),
            ]
        );
    }
}
