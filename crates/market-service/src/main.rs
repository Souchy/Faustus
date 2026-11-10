use std::{net::SocketAddr, path::PathBuf};

use axum::http::HeaderValue;
use clap::{Parser, Subcommand};
use market_service::{ServiceState, cors_layer, ingest_once, router, run_periodic_collector};
use market_store::SqliteStore;

#[derive(Debug, Parser)]
#[command(name = "market-service")]
struct Args {
    #[arg(long, default_value = "faustus.sqlite3", global = true)]
    database: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Serve {
        #[arg(long, default_value = "127.0.0.1:8787")]
        listen: SocketAddr,
        #[arg(long, default_value = "Faustus/0.1.0")]
        user_agent: String,
        #[arg(long, default_values = ["tauri://localhost", "http://tauri.localhost", "https://tauri.localhost"])]
        allow_origin: Vec<HeaderValue>,
    },
    IngestOnce {
        #[arg(long)]
        league: String,
        #[arg(long, default_value = "Faustus/0.1.0")]
        user_agent: String,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let store = SqliteStore::connect(&args.database).await?;

    match args.command {
        Command::Serve {
            listen,
            user_agent,
            allow_origin,
        } => {
            let listener = tokio::net::TcpListener::bind(listen).await?;
            println!("Market service listening on http://{listen}");
            let (selected_league, selected_league_rx) = tokio::sync::watch::channel(None);
            let state = ServiceState::new(store.clone(), user_agent.clone(), selected_league);
            let collector = tokio::spawn(run_periodic_collector(
                store.clone(),
                selected_league_rx,
                user_agent,
            ));
            let result = axum::serve(listener, router(state).layer(cors_layer(allow_origin)))
                .with_graceful_shutdown(shutdown_signal())
                .await;
            collector.abort();
            result?;
        }
        Command::IngestOnce { league, user_agent } => {
            let summary = ingest_once(&store, &league, &user_agent).await?;
            println!("Recorded one exchange page and market snapshots: {summary:?}");
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
