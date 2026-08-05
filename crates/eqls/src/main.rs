mod app;
mod scrape;
mod skin;
mod stats;
mod wiki;

use app::AppState;
use sqlx::postgres::PgPoolOptions;
use std::{path::PathBuf, sync::Arc, time::Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("scrape") => scrape_command(&args[1..]).await,
        Some(other) => Err(format!("unknown subcommand {other:?}").into()),
        None => serve().await,
    }
}

async fn scrape_command(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let pool = connect()?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    scrape::run(&pool, args).await?;
    pool.close().await;
    Ok(())
}

async fn serve() -> Result<(), Box<dyn std::error::Error>> {
    let machine_token: Arc<str> = Arc::from(std::env::var("EQLS_MACHINE_TOKEN")?);
    let listen_addr = std::env::var("LISTEN_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".into());
    let web_dist = PathBuf::from(std::env::var("WEB_DIST").unwrap_or_else(|_| "web/build".into()));
    let pool = connect()?;

    tokio::spawn(migrate_forever(pool.clone()));

    let state = AppState {
        pool,
        machine_token,
    };
    let listener = tokio::net::TcpListener::bind(&listen_addr).await?;
    tracing::info!(addr = %listen_addr, web_dist = %web_dist.display(), "eqls listening");
    axum::serve(listener, app::router(state, web_dist)).await?;
    Ok(())
}

fn connect() -> Result<sqlx::PgPool, Box<dyn std::error::Error>> {
    let database_url = std::env::var("DATABASE_URL")?;
    Ok(PgPoolOptions::new()
        .max_connections(8)
        .acquire_timeout(Duration::from_secs(5))
        .connect_lazy(&database_url)?)
}

/// Retries forever so the process keeps serving /healthz while the database is
/// unreachable.
async fn migrate_forever(pool: sqlx::PgPool) {
    loop {
        match sqlx::migrate!("./migrations").run(&pool).await {
            Ok(()) => {
                tracing::info!("migrations applied");
                return;
            }
            Err(err) => {
                tracing::error!(%err, "migration failed; retrying in 5s");
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }
}
