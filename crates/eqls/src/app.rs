use crate::{
    skin::{self, SkinError},
    stats::{derive_gear_stats, is_equipped_location, GearStats},
    wiki::ItemStats,
};
use axum::{
    body::Body,
    extract::{Path, Query, Request, State},
    http::{header, StatusCode},
    middleware::{from_fn_with_state, Next},
    response::{IntoResponse, Response},
    routing::{get, post, put},
    Json, Router,
};
use eql_core::{
    api::{HarvestDoc, InventoryUpload, LogBatch, LogEventKind, HARVEST_KINDS},
    inventory::InventoryEntry,
    layout::Layout,
};
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
        .route("/api/v1/events", post(ingest_events))
        .route("/api/v1/harvest", post(ingest_harvest))
        .route(
            "/api/v1/layouts/{name}",
            put(put_layout).delete(delete_layout),
        )
        .route("/api/v1/layouts/{name}/clone-default", post(clone_default))
        .layer(from_fn_with_state(state.clone(), require_machine_token));

    let api = Router::new()
        .route("/api/v1/characters", get(list_characters))
        .route(
            "/api/v1/characters/{server}/{name}/events",
            get(list_events),
        )
        .route(
            "/api/v1/characters/{server}/{name}/inventory",
            get(latest_inventory),
        )
        .route(
            "/api/v1/characters/{server}/{name}/stats",
            get(character_stats),
        )
        .route(
            "/api/v1/characters/{server}/{name}/harvest/{kind}",
            get(get_harvest),
        )
        .route("/api/v1/items", get(search_items))
        .route("/api/v1/items/{key}", get(get_item))
        .route("/api/v1/layouts", get(list_layouts))
        .route("/api/v1/layouts/{name}", get(get_layout))
        .route("/api/v1/layouts/{name}/bundle", get(layout_bundle))
        .route("/api/v1/layout-windows", get(layout_windows))
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
    let character_id = upsert_character(&mut tx, &upload.character, &upload.server).await?;
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

async fn upsert_character(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    name: &str,
    server: &str,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "insert into characters (name, server) values ($1, $2) \
         on conflict (name, server) do update set name = excluded.name \
         returning id",
    )
    .bind(name.trim())
    .bind(server.trim())
    .fetch_one(&mut **tx)
    .await
}

#[derive(Serialize)]
struct EventsAccepted {
    character_id: i64,
    events: usize,
}

/// The daemon replays a batch it never saw accepted, so duplicates are expected
/// and harmless: rows are append-only and the UI reads them newest-first.
async fn ingest_events(
    State(state): State<AppState>,
    Json(batch): Json<LogBatch>,
) -> Result<(StatusCode, Json<EventsAccepted>), AppError> {
    if batch.character.trim().is_empty() || batch.server.trim().is_empty() {
        return Err(AppError::EmptyIdentity);
    }
    if batch.events.is_empty() {
        return Err(AppError::EmptyEvents);
    }

    let mut ats = Vec::with_capacity(batch.events.len());
    let mut kinds = Vec::with_capacity(batch.events.len());
    let mut payloads = Vec::with_capacity(batch.events.len());
    for event in &batch.events {
        ats.push(
            OffsetDateTime::from_unix_timestamp(event.at)
                .map_err(|_| AppError::BadCapturedAt(event.at))?,
        );
        kinds.push(event.kind.tag().to_string());
        payloads.push(payload_of(&event.kind));
    }

    let mut tx = state.pool.begin().await?;
    let character_id = upsert_character(&mut tx, &batch.character, &batch.server).await?;
    sqlx::query(
        "insert into log_events (character_id, at, kind, payload) \
         select $1, * from unnest($2::timestamptz[], $3::text[], $4::jsonb[])",
    )
    .bind(character_id)
    .bind(&ats)
    .bind(&kinds)
    .bind(&payloads)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    tracing::info!(
        character = %batch.character,
        server = %batch.server,
        events = batch.events.len(),
        "stored log events"
    );
    Ok((
        StatusCode::CREATED,
        Json(EventsAccepted {
            character_id,
            events: batch.events.len(),
        }),
    ))
}

/// The kind tag rides in its own column, so it is stripped from the payload.
fn payload_of(kind: &LogEventKind) -> serde_json::Value {
    let mut value = serde_json::to_value(kind).unwrap_or_else(|_| serde_json::json!({}));
    if let Some(object) = value.as_object_mut() {
        object.remove("kind");
    }
    value
}

#[derive(Serialize)]
struct HarvestAccepted {
    character_id: i64,
    kind: String,
    #[serde(with = "time::serde::rfc3339")]
    captured_at: OffsetDateTime,
}

/// One row per (character, kind): the companion app rewrites the whole file on
/// every change, so older copies are strictly redundant.
async fn ingest_harvest(
    State(state): State<AppState>,
    Json(upload): Json<HarvestDoc>,
) -> Result<(StatusCode, Json<HarvestAccepted>), AppError> {
    if upload.character.trim().is_empty() || upload.server.trim().is_empty() {
        return Err(AppError::EmptyIdentity);
    }
    let kind = upload.kind.trim().to_lowercase();
    if !HARVEST_KINDS.contains(&kind.as_str()) {
        return Err(AppError::UnknownHarvestKind(upload.kind));
    }
    if upload.doc.is_null() {
        return Err(AppError::EmptyHarvestDoc);
    }
    let captured_at = match upload.captured_at {
        Some(secs) => {
            OffsetDateTime::from_unix_timestamp(secs).map_err(|_| AppError::BadCapturedAt(secs))?
        }
        None => OffsetDateTime::now_utc(),
    };

    let mut tx = state.pool.begin().await?;
    let character_id = upsert_character(&mut tx, &upload.character, &upload.server).await?;
    sqlx::query(
        "insert into harvest_docs (character_id, kind, captured_at, doc) values ($1, $2, $3, $4) \
         on conflict (character_id, kind) do update set captured_at = excluded.captured_at, \
             doc = excluded.doc, created_at = now()",
    )
    .bind(character_id)
    .bind(&kind)
    .bind(captured_at)
    .bind(SqlJson(&upload.doc))
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    tracing::info!(
        character = %upload.character,
        server = %upload.server,
        %kind,
        "stored harvest doc"
    );
    Ok((
        StatusCode::CREATED,
        Json(HarvestAccepted {
            character_id,
            kind,
            captured_at,
        }),
    ))
}

#[derive(Serialize)]
struct HarvestView {
    character: String,
    server: String,
    kind: String,
    #[serde(with = "time::serde::rfc3339")]
    captured_at: OffsetDateTime,
    doc: serde_json::Value,
}

async fn get_harvest(
    State(state): State<AppState>,
    Path((server, name, kind)): Path<(String, String, String)>,
) -> Result<Json<HarvestView>, AppError> {
    let kind = kind.trim().to_lowercase();
    if !HARVEST_KINDS.contains(&kind.as_str()) {
        return Err(AppError::UnknownHarvestKind(kind));
    }
    let row = sqlx::query(
        "select c.name, c.server, h.kind, h.captured_at, h.doc \
         from harvest_docs h \
         join characters c on c.id = h.character_id \
         where lower(c.server) = lower($1) and lower(c.name) = lower($2) and h.kind = $3",
    )
    .bind(&server)
    .bind(&name)
    .bind(&kind)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;

    let doc: SqlJson<serde_json::Value> = row.try_get("doc")?;
    Ok(Json(HarvestView {
        character: row.try_get("name")?,
        server: row.try_get("server")?,
        kind: row.try_get("kind")?,
        captured_at: row.try_get("captured_at")?,
        doc: doc.0,
    }))
}

#[derive(Deserialize)]
struct EventPage {
    limit: Option<i64>,
    before: Option<String>,
}

#[derive(Serialize)]
struct EventView {
    id: i64,
    #[serde(with = "time::serde::rfc3339")]
    at: OffsetDateTime,
    kind: String,
    payload: serde_json::Value,
}

async fn list_events(
    State(state): State<AppState>,
    Path((server, name)): Path<(String, String)>,
    Query(page): Query<EventPage>,
) -> Result<Json<Vec<EventView>>, AppError> {
    let limit = page.limit.unwrap_or(100).clamp(1, 500);
    let before = page
        .before
        .as_deref()
        .filter(|cursor| !cursor.is_empty())
        .map(|cursor| {
            OffsetDateTime::parse(cursor, &time::format_description::well_known::Rfc3339)
                .map_err(|_| AppError::BadCursor(cursor.to_string()))
        })
        .transpose()?;

    let rows = sqlx::query(
        "select e.id, e.at, e.kind, e.payload \
         from log_events e \
         join characters c on c.id = e.character_id \
         where lower(c.server) = lower($1) and lower(c.name) = lower($2) \
           and ($3::timestamptz is null or e.at < $3) \
         order by e.at desc, e.id desc \
         limit $4",
    )
    .bind(&server)
    .bind(&name)
    .bind(before)
    .bind(limit)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(
        rows.iter()
            .map(|row| {
                let payload: SqlJson<serde_json::Value> = row.try_get("payload")?;
                Ok(EventView {
                    id: row.try_get("id")?,
                    at: row.try_get("at")?,
                    kind: row.try_get("kind")?,
                    payload: payload.0,
                })
            })
            .collect::<Result<Vec<_>, sqlx::Error>>()?,
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

#[derive(Serialize)]
struct LayoutSummary {
    name: String,
    screen_w: i32,
    screen_h: i32,
    windows: usize,
    #[serde(with = "time::serde::rfc3339")]
    updated_at: OffsetDateTime,
}

#[derive(Serialize)]
struct LayoutView {
    name: String,
    screen_w: i32,
    screen_h: i32,
    layout: Layout,
    problems: Vec<String>,
    #[serde(with = "time::serde::rfc3339")]
    updated_at: OffsetDateTime,
}

#[derive(Deserialize)]
struct LayoutBody {
    screen_w: i32,
    screen_h: i32,
    layout: Layout,
}

fn layout_from_row(row: &sqlx::postgres::PgRow) -> Result<LayoutView, sqlx::Error> {
    let layout: SqlJson<Layout> = row.try_get("layout")?;
    let screen_w: i32 = row.try_get("screen_w")?;
    let screen_h: i32 = row.try_get("screen_h")?;
    Ok(LayoutView {
        name: row.try_get("name")?,
        screen_w,
        screen_h,
        problems: layout.0.validate(screen_w, screen_h),
        layout: layout.0,
        updated_at: row.try_get("updated_at")?,
    })
}

async fn list_layouts(State(state): State<AppState>) -> Result<Json<Vec<LayoutSummary>>, AppError> {
    let rows = sqlx::query(
        "select name, screen_w, screen_h, layout, updated_at from layouts order by name asc",
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(
        rows.iter()
            .map(|row| {
                let layout: SqlJson<Layout> = row.try_get("layout")?;
                Ok(LayoutSummary {
                    name: row.try_get("name")?,
                    screen_w: row.try_get("screen_w")?,
                    screen_h: row.try_get("screen_h")?,
                    windows: layout.0 .0.len(),
                    updated_at: row.try_get("updated_at")?,
                })
            })
            .collect::<Result<Vec<_>, sqlx::Error>>()?,
    ))
}

async fn layout_windows() -> Json<Vec<&'static str>> {
    Json(skin::template_windows().collect())
}

async fn fetch_layout(pool: &PgPool, name: &str) -> Result<LayoutView, AppError> {
    let row = sqlx::query(
        "select name, screen_w, screen_h, layout, updated_at from layouts where name = $1",
    )
    .bind(name)
    .fetch_optional(pool)
    .await?
    .ok_or(AppError::NotFound)?;
    Ok(layout_from_row(&row)?)
}

async fn get_layout(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<LayoutView>, AppError> {
    Ok(Json(fetch_layout(&state.pool, &name).await?))
}

/// A layout with overlaps is still storable; the problems ride back so the
/// editor can flag them.
async fn store_layout(
    pool: &PgPool,
    name: &str,
    body: LayoutBody,
) -> Result<(StatusCode, Json<LayoutView>), AppError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(AppError::EmptyLayoutName);
    }
    if body.screen_w <= 0 || body.screen_h <= 0 {
        return Err(AppError::BadScreen(body.screen_w, body.screen_h));
    }
    let row = sqlx::query(
        "insert into layouts (name, screen_w, screen_h, layout) values ($1, $2, $3, $4) \
         on conflict (name) do update set screen_w = excluded.screen_w, \
             screen_h = excluded.screen_h, layout = excluded.layout, updated_at = now() \
         returning name, screen_w, screen_h, layout, updated_at",
    )
    .bind(name)
    .bind(body.screen_w)
    .bind(body.screen_h)
    .bind(SqlJson(&body.layout))
    .fetch_one(pool)
    .await?;
    Ok((StatusCode::OK, Json(layout_from_row(&row)?)))
}

async fn put_layout(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(body): Json<LayoutBody>,
) -> Result<(StatusCode, Json<LayoutView>), AppError> {
    store_layout(&state.pool, &name, body).await
}

async fn clone_default(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<(StatusCode, Json<LayoutView>), AppError> {
    store_layout(
        &state.pool,
        &name,
        LayoutBody {
            screen_w: skin::TEMPLATE_WIDTH,
            screen_h: skin::TEMPLATE_HEIGHT,
            layout: skin::default_layout(),
        },
    )
    .await
}

async fn delete_layout(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<StatusCode, AppError> {
    let deleted = sqlx::query("delete from layouts where name = $1")
        .bind(&name)
        .execute(&state.pool)
        .await?
        .rows_affected();
    if deleted == 0 {
        return Err(AppError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct BundleQuery {
    skin: Option<String>,
}

async fn layout_bundle(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(query): Query<BundleQuery>,
) -> Result<Response, AppError> {
    let view = fetch_layout(&state.pool, &name).await?;
    let requested = query.skin.filter(|s| !s.is_empty()).unwrap_or(view.name);
    let skin_name = skin::sanitize_skin_name(&requested);
    let files = skin::generate_bundle(&view.layout, &requested, view.screen_w, view.screen_h)?;
    let zipped = skin::zip_bundle(&files)?;

    Ok((
        [
            (header::CONTENT_TYPE, "application/zip".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{skin_name}.zip\""),
            ),
        ],
        Body::from(zipped),
    )
        .into_response())
}

#[derive(Debug, thiserror::Error)]
enum AppError {
    #[error("missing or invalid bearer token")]
    Unauthorized,
    #[error("character and server must not be empty")]
    EmptyIdentity,
    #[error("inventory upload must contain at least one entry")]
    EmptyEntries,
    #[error("log batch must contain at least one event")]
    EmptyEvents,
    #[error("harvest doc must not be null")]
    EmptyHarvestDoc,
    #[error("unknown harvest kind {0}")]
    UnknownHarvestKind(String),
    #[error("before={0} is not an rfc3339 timestamp")]
    BadCursor(String),
    #[error("captured_at {0} is not a valid unix timestamp")]
    BadCapturedAt(i64),
    #[error("layout name must not be empty")]
    EmptyLayoutName,
    #[error("screen size must be positive, got {0}x{1}")]
    BadScreen(i32, i32),
    #[error(transparent)]
    Skin(#[from] SkinError),
    #[error("no inventory snapshot for that character")]
    NotFound,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = match self {
            AppError::Unauthorized => StatusCode::UNAUTHORIZED,
            AppError::EmptyIdentity
            | AppError::EmptyEntries
            | AppError::EmptyEvents
            | AppError::EmptyHarvestDoc
            | AppError::UnknownHarvestKind(_)
            | AppError::BadCapturedAt(_)
            | AppError::EmptyLayoutName
            | AppError::BadScreen(_, _)
            | AppError::Skin(_)
            | AppError::BadCursor(_) => StatusCode::UNPROCESSABLE_ENTITY,
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

    const BATCH: &str = r#"{
        "character": "Dorsk",
        "server": "erudin",
        "events": [
            {"at": 1784668523, "kind": "zone", "zone": "East Commonlands"},
            {"at": 1784668524, "kind": "loot", "item": "Rusty Dagger"},
            {"at": 1784668525, "kind": "death"}
        ]
    }"#;

    fn post_events(token: &str, body: &str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/api/v1/events")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    #[tokio::test]
    async fn events_reject_a_missing_or_wrong_token() {
        let anonymous = Request::builder()
            .method("POST")
            .uri("/api/v1/events")
            .header("content-type", "application/json")
            .body(Body::from(BATCH))
            .unwrap();
        assert_eq!(status_of(anonymous).await, StatusCode::UNAUTHORIZED);
        assert_eq!(
            status_of(post_events("wrong", "not json at all")).await,
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn events_reject_empty_batches_before_touching_the_database() {
        let empty = r#"{"character":"Dorsk","server":"erudin","events":[]}"#;
        assert_eq!(
            status_of(post_events("s3cret", empty)).await,
            StatusCode::UNPROCESSABLE_ENTITY
        );
        let nameless = r#"{"character":" ","server":"erudin","events":[{"at":1,"kind":"death"}]}"#;
        assert_eq!(
            status_of(post_events("s3cret", nameless)).await,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }

    #[tokio::test]
    async fn an_unparsable_cursor_is_rejected() {
        let request = Request::builder()
            .uri("/api/v1/characters/erudin/Dorsk/events?before=yesterday")
            .body(Body::empty())
            .unwrap();
        assert_eq!(status_of(request).await, StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[test]
    fn payloads_drop_the_kind_tag_they_are_stored_beside() {
        let payload = payload_of(&LogEventKind::Loot {
            item: "Rusty Dagger".into(),
        });
        assert_eq!(payload, serde_json::json!({ "item": "Rusty Dagger" }));
        assert_eq!(
            payload_of(&LogEventKind::Death { killer: None }),
            serde_json::json!({})
        );
    }

    /// Runs against a throwaway database when `EQLS_TEST_DATABASE_URL` is set;
    /// the suite stays green on machines without one.
    async fn live_app() -> Option<(Router, PgPool)> {
        let url = std::env::var("EQLS_TEST_DATABASE_URL").ok()?;
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect(&url)
            .await
            .expect("EQLS_TEST_DATABASE_URL is set but unreachable");
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        sqlx::query("truncate characters restart identity cascade")
            .execute(&pool)
            .await
            .unwrap();
        let app = router(
            AppState {
                pool: pool.clone(),
                machine_token: Arc::from("s3cret"),
            },
            PathBuf::from("web/build"),
        );
        Some((app, pool))
    }

    async fn json_of(app: &Router, request: Request<Body>) -> (StatusCode, serde_json::Value) {
        let response = app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&body).unwrap_or_default())
    }

    #[tokio::test]
    async fn events_upsert_the_character_and_page_newest_first() {
        let Some((app, pool)) = live_app().await else {
            return;
        };

        let (status, accepted) = json_of(&app, post_events("s3cret", BATCH)).await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(accepted["events"], 3);
        let character_id = accepted["character_id"].as_i64().unwrap();

        let page = |query: &str| {
            Request::builder()
                .uri(format!("/api/v1/characters/erudin/dorsk/events?{query}"))
                .body(Body::empty())
                .unwrap()
        };

        let (status, events) = json_of(&app, page("limit=10")).await;
        assert_eq!(status, StatusCode::OK);
        let events = events.as_array().unwrap().clone();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0]["kind"], "death");
        assert_eq!(events[0]["at"], "2026-07-21T21:15:25Z");
        assert_eq!(events[0]["payload"], serde_json::json!({}));
        assert_eq!(events[1]["kind"], "loot");
        assert_eq!(events[1]["payload"]["item"], "Rusty Dagger");
        assert_eq!(events[2]["kind"], "zone");
        assert_eq!(events[2]["payload"]["zone"], "East Commonlands");

        let (_, first_page) = json_of(&app, page("limit=1")).await;
        assert_eq!(first_page.as_array().unwrap().len(), 1);

        let (_, older) = json_of(&app, page("limit=10&before=2026-07-21T21:15:25Z")).await;
        let older = older.as_array().unwrap().clone();
        assert_eq!(older.len(), 2);
        assert_eq!(older[0]["kind"], "loot");
        assert!(older
            .iter()
            .all(|event| event["at"].as_str().unwrap() < "2026-07-21T21:15:25Z"));

        let (status, again) = json_of(&app, post_events("s3cret", BATCH)).await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(again["character_id"].as_i64().unwrap(), character_id);
        let characters: i64 = sqlx::query_scalar("select count(*) from characters")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(characters, 1);
        let stored: i64 = sqlx::query_scalar("select count(*) from log_events")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(stored, 6);

        let (_, unknown) = json_of(&app, {
            Request::builder()
                .uri("/api/v1/characters/erudin/Nobody/events")
                .body(Body::empty())
                .unwrap()
        })
        .await;
        assert_eq!(unknown.as_array().unwrap().len(), 0);
    }

    fn post_harvest(token: Option<&str>, body: &str) -> Request<Body> {
        let mut request = Request::builder()
            .method("POST")
            .uri("/api/v1/harvest")
            .header("content-type", "application/json");
        if let Some(token) = token {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        request.body(Body::from(body.to_string())).unwrap()
    }

    fn harvest_body(kind: &str, doc: &str) -> String {
        format!(
            r#"{{"character":"Dorsk","server":"erudin","kind":"{kind}",
                 "captured_at":1754390000,"doc":{doc}}}"#
        )
    }

    fn fixture(name: &str) -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/harvest")
            .join(name);
        std::fs::read_to_string(path).expect("fixture is committed")
    }

    #[tokio::test]
    async fn harvest_writes_need_the_machine_token() {
        let body = harvest_body("atlas", "{}");
        assert_eq!(
            status_of(post_harvest(None, &body)).await,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            status_of(post_harvest(Some("wrong"), &body)).await,
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn harvest_validates_before_touching_the_database() {
        let cases = [
            harvest_body("spellbook", "{}"),
            harvest_body("atlas", "null"),
            r#"{"character":" ","server":"erudin","kind":"atlas","doc":{}}"#.to_string(),
        ];
        for body in cases {
            assert_eq!(
                status_of(post_harvest(Some("s3cret"), &body)).await,
                StatusCode::UNPROCESSABLE_ENTITY,
                "{body}"
            );
        }
    }

    #[tokio::test]
    async fn an_unknown_harvest_kind_is_rejected_on_read() {
        let request = Request::builder()
            .uri("/api/v1/characters/erudin/Dorsk/harvest/spellbook")
            .body(Body::empty())
            .unwrap();
        assert_eq!(status_of(request).await, StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn harvest_docs_upsert_latest_wins_and_read_back_per_kind() {
        let Some((app, pool)) = live_app().await else {
            return;
        };

        let read = |kind: &str| {
            Request::builder()
                .uri(format!("/api/v1/characters/erudin/dorsk/harvest/{kind}"))
                .body(Body::empty())
                .unwrap()
        };

        let (status, _) = json_of(&app, read("atlas")).await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        for (kind, file) in [
            ("atlas", "eql_atlas_Dorsk_erudin.json"),
            ("quest", "eql_quest_Dorsk_erudin.json"),
            ("alltime", "eql_alltime_Dorsk_erudin__WAR-CLR.json"),
        ] {
            let (status, accepted) = json_of(
                &app,
                post_harvest(Some("s3cret"), &harvest_body(kind, &fixture(file))),
            )
            .await;
            assert_eq!(status, StatusCode::CREATED, "{kind}");
            assert_eq!(accepted["kind"], kind);
        }

        let (status, atlas) = json_of(&app, read("atlas")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(atlas["character"], "Dorsk");
        assert_eq!(atlas["server"], "erudin");
        assert_eq!(atlas["captured_at"], "2025-08-05T10:33:20Z");
        assert_eq!(atlas["doc"]["totals"]["kills"], 137);
        assert_eq!(
            atlas["doc"]["zones"]["befallen"]["mobs"]["a skeleton"]["kills"],
            84
        );

        let (_, quest) = json_of(&app, read("quest")).await;
        assert_eq!(quest["doc"]["current"], "1042");
        let (_, alltime) = json_of(&app, read("alltime")).await;
        assert_eq!(alltime["doc"]["source_dmg"]["melee"], 4_120_334);

        let newer = r#"{"character":"Dorsk","server":"erudin","kind":"atlas",
                        "captured_at":1754400000,"doc":{"format":1,"totals":{"kills":999}}}"#;
        let (status, _) = json_of(&app, post_harvest(Some("s3cret"), newer)).await;
        assert_eq!(status, StatusCode::CREATED);

        let (_, replaced) = json_of(&app, read("atlas")).await;
        assert_eq!(replaced["doc"]["totals"]["kills"], 999);
        assert!(replaced["doc"]["zones"].is_null());
        assert_eq!(replaced["captured_at"], "2025-08-05T13:20:00Z");

        let rows: i64 = sqlx::query_scalar("select count(*) from harvest_docs")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(rows, 3);
        let characters: i64 = sqlx::query_scalar("select count(*) from characters")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(characters, 1);

        let (status, _) = json_of(&app, {
            Request::builder()
                .uri("/api/v1/characters/erudin/Nobody/harvest/atlas")
                .body(Body::empty())
                .unwrap()
        })
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    fn layout_write(method: &str, uri: &str, token: Option<&str>, body: &str) -> Request<Body> {
        let mut request = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json");
        if let Some(token) = token {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        request.body(Body::from(body.to_string())).unwrap()
    }

    const SIMPLE_LAYOUT: &str = r#"{"screen_w":3840,"screen_h":2160,
        "layout":{"PlayerWindow":[420,1290,660,320],"MainChat":[420,1830,1480,310]}}"#;

    #[tokio::test]
    async fn layout_writes_need_the_machine_token() {
        for request in [
            layout_write("PUT", "/api/v1/layouts/mine", None, SIMPLE_LAYOUT),
            layout_write("PUT", "/api/v1/layouts/mine", Some("wrong"), SIMPLE_LAYOUT),
            layout_write("DELETE", "/api/v1/layouts/mine", None, ""),
            layout_write("POST", "/api/v1/layouts/mine/clone-default", None, ""),
        ] {
            assert_eq!(status_of(request).await, StatusCode::UNAUTHORIZED);
        }
    }

    #[tokio::test]
    async fn layout_reads_are_public_and_the_window_list_needs_no_database() {
        let request = Request::builder()
            .uri("/api/v1/layouts")
            .body(Body::empty())
            .unwrap();
        assert_eq!(status_of(request).await, StatusCode::SERVICE_UNAVAILABLE);

        let app = test_app();
        let (status, windows) = json_of(
            &app,
            Request::builder()
                .uri("/api/v1/layout-windows")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let windows = windows.as_array().unwrap().clone();
        assert_eq!(windows.len(), 13);
        assert!(windows.contains(&serde_json::json!("MainChat")));
    }

    #[tokio::test]
    async fn a_bad_screen_size_is_rejected_before_the_database() {
        let body = r#"{"screen_w":0,"screen_h":2160,"layout":{}}"#;
        let request = layout_write("PUT", "/api/v1/layouts/mine", Some("s3cret"), body);
        assert_eq!(status_of(request).await, StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn layouts_round_trip_and_produce_a_downloadable_bundle() {
        let Some((app, _pool)) = live_app().await else {
            return;
        };
        sqlx::query("truncate layouts restart identity")
            .execute(&_pool)
            .await
            .unwrap();

        let (status, cloned) = json_of(
            &app,
            layout_write(
                "POST",
                "/api/v1/layouts/dorskui/clone-default",
                Some("s3cret"),
                "",
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(cloned["layout"].as_object().unwrap().len(), 13);
        assert_eq!(cloned["problems"].as_array().unwrap().len(), 0);
        assert_eq!(cloned["screen_w"], 3840);

        let overlapping = r#"{"screen_w":3840,"screen_h":2160,
            "layout":{"PlayerWindow":[0,0,900,400],"GroupWindow":[100,100,900,400],
                      "MainChat":[3000,2000,2000,500]}}"#;
        let (status, stored) = json_of(
            &app,
            layout_write(
                "PUT",
                "/api/v1/layouts/dorskui",
                Some("s3cret"),
                overlapping,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let problems = stored["problems"].as_array().unwrap().clone();
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(problems
            .iter()
            .any(|p| p.as_str().unwrap().contains("offscreen")));
        assert!(problems
            .iter()
            .any(|p| p.as_str().unwrap().contains("overlaps")));

        let (status, fetched) = json_of(
            &app,
            Request::builder()
                .uri("/api/v1/layouts/dorskui")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            fetched["layout"]["PlayerWindow"],
            serde_json::json!([0, 0, 900, 400])
        );
        assert_eq!(fetched["problems"].as_array().unwrap().len(), 2);

        let (status, listed) = json_of(
            &app,
            Request::builder()
                .uri("/api/v1/layouts")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(listed.as_array().unwrap().len(), 1);
        assert_eq!(listed[0]["windows"], 3);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/layouts/dorskui/bundle?skin=My%20Skin")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["content-type"], "application/zip");
        assert_eq!(
            response.headers()["content-disposition"],
            "attachment; filename=\"my_skin.zip\""
        );
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let archive = zip::ZipArchive::new(std::io::Cursor::new(bytes.to_vec())).unwrap();
        let names: Vec<&str> = archive.file_names().collect();
        assert!(names.contains(&"uifiles/my_skin/EQUI_PlayerWindow.xml"));
        assert!(names.contains(&skin::INI_NAME));

        let unknown = r#"{"screen_w":3840,"screen_h":2160,"layout":{"BankWindow":[0,0,10,10]}}"#;
        let (status, error) = json_of(
            &app,
            layout_write("PUT", "/api/v1/layouts/bad", Some("s3cret"), unknown),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "storing is permissive");
        assert_eq!(error["problems"].as_array().unwrap().len(), 0);
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/layouts/bad/bundle")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);

        let deleted = app
            .clone()
            .oneshot(layout_write(
                "DELETE",
                "/api/v1/layouts/bad",
                Some("s3cret"),
                "",
            ))
            .await
            .unwrap();
        assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
        let missing = app
            .clone()
            .oneshot(layout_write(
                "DELETE",
                "/api/v1/layouts/bad",
                Some("s3cret"),
                "",
            ))
            .await
            .unwrap();
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn constant_time_eq_matches_only_identical_tokens() {
        assert!(constant_time_eq(b"s3cret", b"s3cret"));
        assert!(!constant_time_eq(b"s3crea", b"s3cret"));
        assert!(!constant_time_eq(b"", b"s3cret"));
        assert!(!constant_time_eq(b"s3cret-longer", b"s3cret"));
    }
}
