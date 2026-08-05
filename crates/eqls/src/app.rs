use axum::{
    extract::{Path, Request, State},
    http::{header, StatusCode},
    middleware::{from_fn_with_state, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use eql_core::{api::InventoryUpload, inventory::InventoryEntry};
use serde::Serialize;
use sqlx::{types::Json as SqlJson, PgPool, Row};
use std::{path::PathBuf, sync::Arc};
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
    entries: Vec<InventoryEntry>,
}

async fn latest_inventory(
    State(state): State<AppState>,
    Path((server, name)): Path<(String, String)>,
) -> Result<Json<InventoryView>, AppError> {
    let row = sqlx::query(
        "select c.name, c.server, s.captured_at, s.entries \
         from characters c \
         join inventory_snapshots s on s.character_id = c.id \
         where lower(c.server) = lower($1) and lower(c.name) = lower($2) \
         order by s.captured_at desc, s.id desc \
         limit 1",
    )
    .bind(&server)
    .bind(&name)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;

    let entries: SqlJson<Vec<InventoryEntry>> = row.try_get("entries")?;
    Ok(Json(InventoryView {
        character: row.try_get("name")?,
        server: row.try_get("server")?,
        captured_at: row.try_get("captured_at")?,
        entries: entries.0,
    }))
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
