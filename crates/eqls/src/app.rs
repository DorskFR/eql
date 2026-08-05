use crate::{
    stats::{derive_gear_stats, is_equipped_location, GearStats},
    wiki::ItemStats,
};
use axum::{
    extract::{Path, Query, Request, State},
    http::{header, StatusCode},
    middleware::{from_fn_with_state, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use eql_core::{api::InventoryUpload, inventory::InventoryEntry};
use serde::{Deserialize, Serialize};
use sqlx::{types::Json as SqlJson, PgPool, Row};
use std::{collections::HashMap, path::PathBuf, sync::Arc};
use time::OffsetDateTime;
use tower_http::services::{ServeDir, ServeFile};

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub machine_token: Arc<str>,
}

pub fn router(state: AppState, web_dist: PathBuf) -> Router {
    let ingest = Router::new()
        .route("/api/v1/inventory", post(ingest))
        .layer(from_fn_with_state(state.clone(), require_machine_token));

    let api = Router::new()
        .route("/api/v1/characters", get(list_characters))
        .route(
            "/api/v1/characters/{server}/{name}/inventory",
            get(latest_inventory),
        )
        .route(
            "/api/v1/characters/{server}/{name}/stats",
            get(character_stats),
        )
        .route("/api/v1/items", get(search_items))
        .route("/api/v1/items/{key}", get(get_item))
        .merge(ingest);

    let index = web_dist.join("index.html");
    let web = ServeDir::new(web_dist).fallback(ServeFile::new(index));

    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .merge(api)
        .fallback_service(web)
        .with_state(state)
}

async fn healthz() -> &'static str {
    "ok"
}

async fn readyz(State(state): State<AppState>) -> Response {
    match sqlx::query("select 1").fetch_one(&state.pool).await {
        Ok(_) => (StatusCode::OK, "ready").into_response(),
        Err(err) => {
            tracing::warn!(%err, "readiness probe failed");
            (StatusCode::SERVICE_UNAVAILABLE, "database unavailable").into_response()
        }
    }
}

async fn require_machine_token(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, AppError> {
    let presented = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .unwrap_or_default();
    if !constant_time_eq(presented.as_bytes(), state.machine_token.as_bytes()) {
        return Err(AppError::Unauthorized);
    }
    Ok(next.run(request).await)
}

/// Compares in time independent of *where* the first difference falls; the
/// length of the presented token is still observable.
fn constant_time_eq(presented: &[u8], expected: &[u8]) -> bool {
    if presented.len() != expected.len() {
        return false;
    }
    presented
        .iter()
        .zip(expected)
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

#[derive(Serialize)]
struct UploadAccepted {
    character_id: i64,
    snapshot_id: i64,
    entries: usize,
    #[serde(with = "time::serde::rfc3339")]
    captured_at: OffsetDateTime,
}

async fn ingest(
    State(state): State<AppState>,
    Json(upload): Json<InventoryUpload>,
) -> Result<(StatusCode, Json<UploadAccepted>), AppError> {
    if upload.character.trim().is_empty() || upload.server.trim().is_empty() {
        return Err(AppError::EmptyIdentity);
    }
    if upload.entries.is_empty() {
        return Err(AppError::EmptyEntries);
    }
    let captured_at = match upload.captured_at {
        Some(secs) => {
            OffsetDateTime::from_unix_timestamp(secs).map_err(|_| AppError::BadCapturedAt(secs))?
        }
        None => OffsetDateTime::now_utc(),
    };

    let mut tx = state.pool.begin().await?;
    let character_id: i64 = sqlx::query_scalar(
        "insert into characters (name, server) values ($1, $2) \
         on conflict (name, server) do update set name = excluded.name \
         returning id",
    )
    .bind(upload.character.trim())
    .bind(upload.server.trim())
    .fetch_one(&mut *tx)
    .await?;
    let snapshot_id: i64 = sqlx::query_scalar(
        "insert into inventory_snapshots (character_id, captured_at, entries, raw) \
         values ($1, $2, $3, $4) returning id",
    )
    .bind(character_id)
    .bind(captured_at)
    .bind(SqlJson(&upload.entries))
    .bind(upload.raw.as_deref())
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;

    tracing::info!(
        character = %upload.character,
        server = %upload.server,
        entries = upload.entries.len(),
        snapshot_id,
        "stored inventory snapshot"
    );
    Ok((
        StatusCode::CREATED,
        Json(UploadAccepted {
            character_id,
            snapshot_id,
            entries: upload.entries.len(),
            captured_at,
        }),
    ))
}

#[derive(Serialize)]
struct CharacterSummary {
    name: String,
    server: String,
    #[serde(with = "time::serde::rfc3339::option")]
    last_snapshot_at: Option<OffsetDateTime>,
    snapshot_count: i64,
}

async fn list_characters(
    State(state): State<AppState>,
) -> Result<Json<Vec<CharacterSummary>>, AppError> {
    let rows = sqlx::query(
        "select c.name, c.server, max(s.captured_at) as last_snapshot_at, \
                count(s.id) as snapshot_count \
         from characters c \
         left join inventory_snapshots s on s.character_id = c.id \
         group by c.id, c.name, c.server \
         order by c.server asc, c.name asc",
    )
    .fetch_all(&state.pool)
    .await?;

    let characters = rows
        .into_iter()
        .map(|row| {
            Ok(CharacterSummary {
                name: row.try_get("name")?,
                server: row.try_get("server")?,
                last_snapshot_at: row.try_get("last_snapshot_at")?,
                snapshot_count: row.try_get("snapshot_count")?,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()?;
    Ok(Json(characters))
}

#[derive(Serialize)]
struct InventoryView {
    character: String,
    server: String,
    #[serde(with = "time::serde::rfc3339")]
    captured_at: OffsetDateTime,
    entries: Vec<InventoryEntryView>,
}

#[derive(Serialize)]
struct InventoryEntryView {
    #[serde(flatten)]
    entry: InventoryEntry,
    #[serde(skip_serializing_if = "Option::is_none")]
    item: Option<ItemRecord>,
}

#[derive(Clone, Serialize)]
struct ItemRecord {
    id: i64,
    game_id: Option<i64>,
    name: String,
    stats: serde_json::Value,
    #[serde(with = "time::serde::rfc3339")]
    scraped_at: OffsetDateTime,
}

fn item_from_row(row: &sqlx::postgres::PgRow) -> Result<ItemRecord, sqlx::Error> {
    let stats: SqlJson<serde_json::Value> = row.try_get("stats")?;
    Ok(ItemRecord {
        id: row.try_get("id")?,
        game_id: row.try_get("game_id")?,
        name: row.try_get("name")?,
        stats: stats.0,
        scraped_at: row.try_get("scraped_at")?,
    })
}

/// Wiki item pages carry no in-game item id, so the join is by folded name.
async fn items_by_name(
    pool: &PgPool,
    names: &[String],
) -> Result<HashMap<String, ItemRecord>, sqlx::Error> {
    if names.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = sqlx::query(
        "select id, game_id, name, stats, scraped_at from items where lower(name) = any($1)",
    )
    .bind(names)
    .fetch_all(pool)
    .await?;
    rows.iter()
        .map(|row| {
            let item = item_from_row(row)?;
            Ok((item.name.to_lowercase(), item))
        })
        .collect()
}

struct Snapshot {
    character: String,
    server: String,
    captured_at: OffsetDateTime,
    entries: Vec<InventoryEntry>,
}

async fn latest_snapshot(pool: &PgPool, server: &str, name: &str) -> Result<Snapshot, AppError> {
    let row = sqlx::query(
        "select c.name, c.server, s.captured_at, s.entries \
         from characters c \
         join inventory_snapshots s on s.character_id = c.id \
         where lower(c.server) = lower($1) and lower(c.name) = lower($2) \
         order by s.captured_at desc, s.id desc \
         limit 1",
    )
    .bind(server)
    .bind(name)
    .fetch_optional(pool)
    .await?
    .ok_or(AppError::NotFound)?;

    let entries: SqlJson<Vec<InventoryEntry>> = row.try_get("entries")?;
    Ok(Snapshot {
        character: row.try_get("name")?,
        server: row.try_get("server")?,
        captured_at: row.try_get("captured_at")?,
        entries: entries.0,
    })
}

async fn join_items(
    pool: &PgPool,
    entries: Vec<InventoryEntry>,
) -> Result<Vec<InventoryEntryView>, sqlx::Error> {
    let mut names: Vec<String> = entries
        .iter()
        .filter(|entry| !entry.is_empty_slot())
        .map(|entry| entry.name.to_lowercase())
        .collect();
    names.sort_unstable();
    names.dedup();
    let items = items_by_name(pool, &names).await?;

    Ok(entries
        .into_iter()
        .map(|entry| InventoryEntryView {
            item: items.get(&entry.name.to_lowercase()).cloned(),
            entry,
        })
        .collect())
}

async fn latest_inventory(
    State(state): State<AppState>,
    Path((server, name)): Path<(String, String)>,
) -> Result<Json<InventoryView>, AppError> {
    let snapshot = latest_snapshot(&state.pool, &server, &name).await?;
    Ok(Json(InventoryView {
        character: snapshot.character,
        server: snapshot.server,
        captured_at: snapshot.captured_at,
        entries: join_items(&state.pool, snapshot.entries).await?,
    }))
}

#[derive(Serialize)]
struct StatsView {
    character: String,
    server: String,
    #[serde(with = "time::serde::rfc3339")]
    captured_at: OffsetDateTime,
    stats: GearStats,
    equipped: Vec<InventoryEntryView>,
}

async fn character_stats(
    State(state): State<AppState>,
    Path((server, name)): Path<(String, String)>,
) -> Result<Json<StatsView>, AppError> {
    let snapshot = latest_snapshot(&state.pool, &server, &name).await?;
    let views = join_items(&state.pool, snapshot.entries).await?;

    let pairs: Vec<(InventoryEntry, Option<ItemStats>)> = views
        .iter()
        .map(|view| {
            let stats = view
                .item
                .as_ref()
                .and_then(|item| serde_json::from_value(item.stats.clone()).ok());
            (view.entry.clone(), stats)
        })
        .collect();

    Ok(Json(StatsView {
        character: snapshot.character,
        server: snapshot.server,
        captured_at: snapshot.captured_at,
        stats: derive_gear_stats(&pairs),
        equipped: views
            .into_iter()
            .filter(|view| {
                !view.entry.is_empty_slot() && is_equipped_location(&view.entry.location)
            })
            .collect(),
    }))
}

#[derive(Deserialize)]
struct ItemSearch {
    #[serde(default)]
    q: String,
}

async fn search_items(
    State(state): State<AppState>,
    Query(search): Query<ItemSearch>,
) -> Result<Json<Vec<ItemRecord>>, AppError> {
    let needle = search.q.trim().to_lowercase();
    if needle.is_empty() {
        return Ok(Json(Vec::new()));
    }
    let rows = sqlx::query(
        "select id, game_id, name, stats, scraped_at from items \
         where lower(name) like '%' || $1 || '%' \
         order by position($1 in lower(name)), length(name), name limit 20",
    )
    .bind(&needle)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(
        rows.iter()
            .map(item_from_row)
            .collect::<Result<Vec<_>, sqlx::Error>>()?,
    ))
}

async fn get_item(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> Result<Json<ItemRecord>, AppError> {
    let numeric: Option<i64> = key.parse().ok();
    let row = sqlx::query(
        "select id, game_id, name, stats, scraped_at from items \
         where lower(name) = lower($1) or id = $2 or game_id = $2 limit 1",
    )
    .bind(&key)
    .bind(numeric)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;
    Ok(Json(item_from_row(&row)?))
}

#[derive(Debug, thiserror::Error)]
enum AppError {
    #[error("missing or invalid bearer token")]
    Unauthorized,
    #[error("character and server must not be empty")]
    EmptyIdentity,
    #[error("inventory upload must contain at least one entry")]
    EmptyEntries,
    #[error("captured_at {0} is not a valid unix timestamp")]
    BadCapturedAt(i64),
    #[error("no inventory snapshot for that character")]
    NotFound,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = match self {
            AppError::Unauthorized => StatusCode::UNAUTHORIZED,
            AppError::EmptyIdentity | AppError::EmptyEntries | AppError::BadCapturedAt(_) => {
                StatusCode::UNPROCESSABLE_ENTITY
            }
            AppError::NotFound => StatusCode::NOT_FOUND,
            AppError::Database(ref err) => {
                tracing::error!(%err, "database error");
                StatusCode::SERVICE_UNAVAILABLE
            }
        };
        let message = match self {
            AppError::Database(_) => "database unavailable".to_string(),
            other => other.to_string(),
        };
        (status, Json(serde_json::json!({ "error": message }))).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    fn test_app() -> Router {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .acquire_timeout(std::time::Duration::from_millis(250))
            .connect_lazy("postgres://nobody:nobody@127.0.0.1:1/nothing")
            .unwrap();
        router(
            AppState {
                pool,
                machine_token: Arc::from("s3cret"),
            },
            PathBuf::from("web/build"),
        )
    }

    async fn status_of(request: Request<Body>) -> StatusCode {
        test_app().oneshot(request).await.unwrap().status()
    }

    #[tokio::test]
    async fn healthz_needs_no_database_and_no_token() {
        let response = test_app()
            .oneshot(
                Request::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[..], b"ok");
    }

    #[tokio::test]
    async fn ingest_rejects_missing_token() {
        let request = Request::builder()
            .method("POST")
            .uri("/api/v1/inventory")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"character":"Dorsk","server":"erudin","entries":[]}"#,
            ))
            .unwrap();
        assert_eq!(status_of(request).await, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn ingest_rejects_wrong_token_before_parsing_body() {
        let request = Request::builder()
            .method("POST")
            .uri("/api/v1/inventory")
            .header("authorization", "Bearer wrong")
            .header("content-type", "application/json")
            .body(Body::from("not json at all"))
            .unwrap();
        assert_eq!(status_of(request).await, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn ingest_rejects_empty_entries() {
        let request = Request::builder()
            .method("POST")
            .uri("/api/v1/inventory")
            .header("authorization", "Bearer s3cret")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"character":"Dorsk","server":"erudin","entries":[]}"#,
            ))
            .unwrap();
        assert_eq!(status_of(request).await, StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn database_outage_is_service_unavailable() {
        let request = Request::builder()
            .uri("/api/v1/characters")
            .body(Body::empty())
            .unwrap();
        assert_eq!(status_of(request).await, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[test]
    fn constant_time_eq_matches_only_identical_tokens() {
        assert!(constant_time_eq(b"s3cret", b"s3cret"));
        assert!(!constant_time_eq(b"s3crea", b"s3cret"));
        assert!(!constant_time_eq(b"", b"s3cret"));
        assert!(!constant_time_eq(b"s3cret-longer", b"s3cret"));
    }
}
