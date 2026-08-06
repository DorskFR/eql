use crate::{
    icons::{self, IconError},
    itemdump::{self, DumpIcon},
    skin::{self, SkinError},
    stats::{derive_gear_stats, is_equipped_location, GearStats},
    wiki::ItemStats,
};
use axum::{
    body::{Body, Bytes},
    extract::{DefaultBodyLimit, Path, Query, Request, State},
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
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, RwLock},
};
use time::OffsetDateTime;
use tower_http::services::{ServeDir, ServeFile};

const SHEET_LIMIT: usize = 1024 * 1024;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub machine_token: Arc<str>,
    icons: Arc<RwLock<HashMap<i32, Bytes>>>,
}

impl AppState {
    pub fn new(pool: PgPool, machine_token: Arc<str>) -> Self {
        Self {
            pool,
            machine_token,
            icons: Arc::default(),
        }
    }
}

pub fn router(state: AppState, web_dist: PathBuf) -> Router {
    let ingest = Router::new()
        .route("/api/v1/inventory", post(ingest))
        .route("/api/v1/events", post(ingest_events))
        .route("/api/v1/harvest", post(ingest_harvest))
        .route("/api/v1/fights", post(ingest_fights))
        .route(
            "/api/v1/icons/sheets/{sheet}",
            put(put_icon_sheet).layer(DefaultBodyLimit::max(SHEET_LIMIT)),
        )
        .route(
            "/api/v1/layouts/{name}",
            put(put_layout).delete(delete_layout),
        )
        .route("/api/v1/layouts/{name}/clone-default", post(clone_default))
        .layer(from_fn_with_state(state.clone(), require_machine_token));

    let api = Router::new()
        .route("/api/v1/characters", get(list_characters))
        .route("/api/v1/characters/{server}/{name}", get(get_character))
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
        .route(
            "/api/v1/characters/{server}/{name}/fights",
            get(list_fights),
        )
        .route("/api/v1/icons/{file}", get(get_icon))
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
    if let Some((at, level, classes, race)) = newest_identity(&batch.events) {
        sqlx::query(
            "update characters set level = $2, classes = $3, race = $4, identity_at = $5 \
             where id = $1 and (identity_at is null or identity_at <= $5)",
        )
        .bind(character_id)
        .bind(level)
        .bind(&classes)
        .bind(&race)
        .bind(at)
        .execute(&mut *tx)
        .await?;
    }
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

type Identity = (OffsetDateTime, i32, Vec<String>, Option<String>);

/// A batch can replay an old `/who`; the newest row in it wins, and the update
/// itself refuses to go backwards.
fn newest_identity(events: &[eql_core::api::LogEvent]) -> Option<Identity> {
    events
        .iter()
        .filter_map(|event| match &event.kind {
            LogEventKind::Who {
                level,
                classes,
                race,
            } => {
                let at = OffsetDateTime::from_unix_timestamp(event.at).ok()?;
                Some((
                    at,
                    i32::try_from(*level).ok()?,
                    classes.clone(),
                    race.clone(),
                ))
            }
            _ => None,
        })
        .max_by_key(|(at, ..)| *at)
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

fn parse_cursor(before: Option<&str>) -> Result<Option<OffsetDateTime>, AppError> {
    before
        .filter(|cursor| !cursor.is_empty())
        .map(|cursor| {
            OffsetDateTime::parse(cursor, &time::format_description::well_known::Rfc3339)
                .map_err(|_| AppError::BadCursor(cursor.to_string()))
        })
        .transpose()
}

#[derive(Deserialize)]
struct FightsUpload {
    character: String,
    server: String,
    fights: Vec<serde_json::Value>,
}

#[derive(Deserialize)]
struct FightMeta {
    start_wall: f64,
    #[serde(default)]
    zone: Option<String>,
    #[serde(default)]
    span: f64,
    #[serde(default)]
    active_secs: f64,
    #[serde(default)]
    dmg_out_you: i64,
    #[serde(default)]
    dmg_in_you: i64,
    #[serde(default)]
    heal_out: i64,
    #[serde(default)]
    kills: i32,
    #[serde(default)]
    deaths: i32,
    #[serde(default)]
    enemies: Vec<String>,
}

#[derive(Serialize)]
struct FightsAccepted {
    received: usize,
    stored: usize,
}

fn fight_meta(fight: &serde_json::Value) -> Result<(FightMeta, OffsetDateTime), AppError> {
    let meta: FightMeta =
        serde_json::from_value(fight.clone()).map_err(|err| AppError::BadFight(err.to_string()))?;
    if !meta.start_wall.is_finite() {
        return Err(AppError::BadFight("start_wall is not finite".into()));
    }
    let nanos = (meta.start_wall * 1e9).round() as i128;
    let started_at = OffsetDateTime::from_unix_timestamp_nanos(nanos).map_err(|_| {
        AppError::BadFight(format!("start_wall {} is out of range", meta.start_wall))
    })?;
    Ok((meta, started_at))
}

/// Fights are history: rows accumulate, and a replayed log re-posting a fight
/// the server already has is a no-op on `(character_id, start_wall)`.
async fn ingest_fights(
    State(state): State<AppState>,
    Json(upload): Json<FightsUpload>,
) -> Result<(StatusCode, Json<FightsAccepted>), AppError> {
    if upload.character.trim().is_empty() || upload.server.trim().is_empty() {
        return Err(AppError::EmptyIdentity);
    }
    if upload.fights.is_empty() {
        return Err(AppError::EmptyFights);
    }

    let mut parsed = Vec::with_capacity(upload.fights.len());
    for fight in &upload.fights {
        let (meta, started_at) = fight_meta(fight)?;
        parsed.push((meta, started_at, fight));
    }

    let mut tx = state.pool.begin().await?;
    let character_id = upsert_character(&mut tx, &upload.character, &upload.server).await?;
    let mut stored = 0usize;
    for (meta, started_at, fight) in &parsed {
        stored += sqlx::query(
            "insert into fights (character_id, start_wall, started_at, zone, span, active_secs, \
                 dmg_out, dmg_in, heal_out, kills, deaths, enemies, fight) \
             values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13) \
             on conflict (character_id, start_wall) do nothing",
        )
        .bind(character_id)
        .bind(meta.start_wall)
        .bind(started_at)
        .bind(meta.zone.as_deref())
        .bind(meta.span)
        .bind(meta.active_secs)
        .bind(meta.dmg_out_you)
        .bind(meta.dmg_in_you)
        .bind(meta.heal_out)
        .bind(meta.kills)
        .bind(meta.deaths)
        .bind(&meta.enemies)
        .bind(SqlJson(fight))
        .execute(&mut *tx)
        .await?
        .rows_affected() as usize;
    }
    tx.commit().await?;

    tracing::info!(
        character = %upload.character,
        server = %upload.server,
        received = parsed.len(),
        stored,
        "stored fights"
    );
    Ok((
        StatusCode::CREATED,
        Json(FightsAccepted {
            received: parsed.len(),
            stored,
        }),
    ))
}

#[derive(Serialize)]
struct FightView {
    id: i64,
    #[serde(with = "time::serde::rfc3339")]
    started_at: OffsetDateTime,
    start_wall: f64,
    fight: serde_json::Value,
}

async fn list_fights(
    State(state): State<AppState>,
    Path((server, name)): Path<(String, String)>,
    Query(page): Query<EventPage>,
) -> Result<Json<Vec<FightView>>, AppError> {
    let limit = page.limit.unwrap_or(50).clamp(1, 200);
    let before = parse_cursor(page.before.as_deref())?;

    let rows = sqlx::query(
        "select f.id, f.started_at, f.start_wall, f.fight \
         from fights f \
         join characters c on c.id = f.character_id \
         where lower(c.server) = lower($1) and lower(c.name) = lower($2) \
           and ($3::timestamptz is null or f.started_at < $3) \
         order by f.started_at desc, f.id desc \
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
                let fight: SqlJson<serde_json::Value> = row.try_get("fight")?;
                Ok(FightView {
                    id: row.try_get("id")?,
                    started_at: row.try_get("started_at")?,
                    start_wall: row.try_get("start_wall")?,
                    fight: fight.0,
                })
            })
            .collect::<Result<Vec<_>, sqlx::Error>>()?,
    ))
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
    let before = parse_cursor(page.before.as_deref())?;

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

#[derive(Serialize)]
struct CharacterView {
    name: String,
    server: String,
    level: Option<i32>,
    race: Option<String>,
    classes: Vec<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    identity_at: Option<OffsetDateTime>,
}

async fn get_character(
    State(state): State<AppState>,
    Path((server, name)): Path<(String, String)>,
) -> Result<Json<CharacterView>, AppError> {
    let row = sqlx::query(
        "select name, server, level, race, classes, identity_at from characters \
         where lower(server) = lower($1) and lower(name) = lower($2) limit 1",
    )
    .bind(&server)
    .bind(&name)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;

    Ok(Json(CharacterView {
        name: row.try_get("name")?,
        server: row.try_get("server")?,
        level: row.try_get("level")?,
        race: row.try_get("race")?,
        classes: row
            .try_get::<Option<Vec<String>>, _>("classes")?
            .unwrap_or_default(),
        identity_at: row.try_get("identity_at")?,
    }))
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
    #[serde(skip_serializing_if = "Option::is_none")]
    upgrade: Option<u32>,
}

/// The server decorates names ("Bronze Helm +5"); the wiki knows only base names.
fn upgrade_suffix(name: &str) -> Option<(&str, u32)> {
    let (base, level) = name.rsplit_once(" +")?;
    if base.is_empty() || level.is_empty() || !level.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((base, level.parse().ok()?))
}

#[derive(Clone, Serialize)]
struct ItemRecord {
    id: i64,
    game_id: Option<i64>,
    name: String,
    stats: serde_json::Value,
    #[serde(with = "time::serde::rfc3339")]
    scraped_at: OffsetDateTime,
    #[serde(skip)]
    from_dump: bool,
}

fn item_from_row(row: &sqlx::postgres::PgRow) -> Result<ItemRecord, sqlx::Error> {
    let stats: SqlJson<serde_json::Value> = row.try_get("stats")?;
    Ok(ItemRecord {
        id: row.try_get("id")?,
        game_id: row.try_get("game_id")?,
        name: row.try_get("name")?,
        stats: stats.0,
        scraped_at: row.try_get("scraped_at")?,
        from_dump: false,
    })
}

/// Stands up a record with no `items` row behind it: the dump knows a name and
/// an icon and nothing else, so `from_dump` keeps it out of the stat totals.
fn item_from_dump(found: &DumpIcon) -> ItemRecord {
    let stats = ItemStats {
        name: found.name.clone(),
        icon: Some(found.icon.into()),
        ..Default::default()
    };
    ItemRecord {
        id: 0,
        game_id: Some(found.game_id),
        name: found.name.clone(),
        stats: serde_json::to_value(stats).expect("item stats serialise to json"),
        scraped_at: OffsetDateTime::UNIX_EPOCH,
        from_dump: true,
    }
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

fn peel_decoration(name: &str) -> Option<(String, Option<u32>)> {
    if let Some((base, level)) = upgrade_suffix(name) {
        return Some((base.to_string(), Some(level)));
    }
    if let Some(base) = name.strip_suffix('*') {
        let base = base.trim_end();
        if !base.is_empty() {
            return Some((base.to_string(), None));
        }
    }
    if name.ends_with(')') {
        if let Some((base, _)) = name.rsplit_once(" (") {
            if !base.is_empty() {
                return Some((base.to_string(), None));
            }
        }
    }
    None
}

fn name_candidates(name: &str) -> Vec<(String, Option<u32>)> {
    let mut candidates = vec![(name.to_lowercase(), None)];
    while let Some((base, level)) = peel_decoration(&candidates.last().unwrap().0) {
        let level = level.or(candidates.last().unwrap().1);
        candidates.push((base, level));
    }
    candidates
}

fn resolve_item(
    items: &HashMap<String, ItemRecord>,
    name: &str,
) -> (Option<ItemRecord>, Option<u32>) {
    for (candidate, level) in name_candidates(name) {
        if let Some(item) = items.get(&candidate) {
            return (Some(item.clone()), level);
        }
    }
    (None, None)
}

async fn join_items(
    pool: &PgPool,
    entries: Vec<InventoryEntry>,
) -> Result<Vec<InventoryEntryView>, sqlx::Error> {
    let mut names: Vec<String> = entries
        .iter()
        .filter(|entry| !entry.is_empty_slot())
        .flat_map(|entry| {
            name_candidates(&entry.name)
                .into_iter()
                .map(|(candidate, _)| candidate)
        })
        .collect();
    names.sort_unstable();
    names.dedup();
    let items = items_by_name(pool, &names).await?;

    let mut views: Vec<InventoryEntryView> = entries
        .into_iter()
        .map(|entry| {
            let (item, upgrade) = resolve_item(&items, &entry.name);
            InventoryEntryView {
                item,
                upgrade,
                entry,
            }
        })
        .collect();
    fill_dump_icons(pool, &mut views).await?;
    Ok(views)
}

fn wants_dump_icon(view: &InventoryEntryView) -> bool {
    !view.entry.is_empty_slot()
        && view.item.as_ref().is_none_or(|item| {
            item.stats
                .get("icon")
                .is_none_or(serde_json::Value::is_null)
        })
}

async fn fill_dump_icons(
    pool: &PgPool,
    views: &mut [InventoryEntryView],
) -> Result<(), sqlx::Error> {
    let mut keys: Vec<String> = views
        .iter()
        .filter(|view| wants_dump_icon(view))
        .flat_map(|view| {
            name_candidates(&view.entry.name)
                .into_iter()
                .map(|(candidate, _)| itemdump::name_key(&candidate))
        })
        .collect();
    keys.sort_unstable();
    keys.dedup();

    let dump = itemdump::lookup(pool, &keys).await?;
    if dump.is_empty() {
        return Ok(());
    }
    for view in views.iter_mut().filter(|view| wants_dump_icon(view)) {
        let found = name_candidates(&view.entry.name)
            .into_iter()
            .find_map(|(candidate, level)| {
                dump.get(&itemdump::name_key(&candidate))
                    .map(|found| (found, level))
            });
        let Some((found, level)) = found else {
            continue;
        };
        match view.item.as_mut() {
            Some(item) => {
                if let Some(stats) = item.stats.as_object_mut() {
                    stats.insert("icon".into(), found.icon.into());
                }
            }
            None => {
                view.item = Some(item_from_dump(found));
                view.upgrade = level;
            }
        }
    }
    Ok(())
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
                .filter(|item| !item.from_dump)
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

#[derive(Serialize)]
struct SheetAccepted {
    sheet: u32,
    icons: usize,
}

async fn put_icon_sheet(
    State(state): State<AppState>,
    Path(sheet): Path<u32>,
    dds: Bytes,
) -> Result<Json<SheetAccepted>, AppError> {
    let cells = icons::split_sheet(sheet, &dds)?;
    let (ids, pngs): (Vec<i32>, Vec<Vec<u8>>) = cells.into_iter().unzip();
    sqlx::query(
        "insert into item_icons (icon, png) \
         select * from unnest($1::int[], $2::bytea[]) \
         on conflict (icon) do update set png = excluded.png, updated_at = now()",
    )
    .bind(&ids)
    .bind(&pngs)
    .execute(&state.pool)
    .await?;

    let mut cache = state.icons.write().expect("icon cache is not poisoned");
    for icon in &ids {
        cache.remove(icon);
    }
    drop(cache);

    tracing::info!(sheet, icons = ids.len(), "stored icon sheet");
    Ok(Json(SheetAccepted {
        sheet,
        icons: ids.len(),
    }))
}

/// A path capture cannot carry a literal suffix in axum, so `624.png` arrives
/// whole and the extension is stripped here.
async fn get_icon(
    State(state): State<AppState>,
    Path(file): Path<String>,
) -> Result<Response, AppError> {
    let icon: i32 = file
        .strip_suffix(".png")
        .and_then(|icon| icon.parse().ok())
        .ok_or(AppError::NotFound)?;
    if icons::locate(icon).is_none() {
        return Err(AppError::NotFound);
    }
    let cached = state
        .icons
        .read()
        .expect("icon cache is not poisoned")
        .get(&icon)
        .cloned();
    let png = match cached {
        Some(png) => png,
        None => {
            let png: Vec<u8> = sqlx::query_scalar("select png from item_icons where icon = $1")
                .bind(icon)
                .fetch_optional(&state.pool)
                .await?
                .ok_or(AppError::NotFound)?;
            let png = Bytes::from(png);
            state
                .icons
                .write()
                .expect("icon cache is not poisoned")
                .insert(icon, png.clone());
            png
        }
    };

    Ok((
        [
            (header::CONTENT_TYPE, "image/png".to_string()),
            (
                header::CACHE_CONTROL,
                "public, max-age=31536000, immutable".to_string(),
            ),
            (header::ETAG, format!("\"icon-{icon}\"")),
        ],
        Body::from(png),
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
    #[error("fight upload must contain at least one fight")]
    EmptyFights,
    #[error("unusable fight: {0}")]
    BadFight(String),
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
    #[error(transparent)]
    Icon(#[from] IconError),
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
            | AppError::EmptyFights
            | AppError::BadFight(_)
            | AppError::UnknownHarvestKind(_)
            | AppError::BadCapturedAt(_)
            | AppError::EmptyLayoutName
            | AppError::BadScreen(_, _)
            | AppError::Skin(_)
            | AppError::Icon(_)
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
            AppState::new(pool, Arc::from("s3cret")),
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
            AppState::new(pool.clone(), Arc::from("s3cret")),
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

    const WHO: &str = r#"{
        "character": "Morveus",
        "server": "erudin",
        "events": [
            {"at": 1785958164, "kind": "who", "level": 15, "classes": ["WAR", "DRU", "NEC"], "race": "Dark Elf"},
            {"at": 1785962763, "kind": "who", "level": 16, "classes": ["WAR", "DRU", "NEC"], "race": "Dark Elf"}
        ]
    }"#;

    #[test]
    fn the_newest_who_in_a_batch_wins() {
        let batch: LogBatch = serde_json::from_str(WHO).unwrap();
        let (at, level, classes, race) = newest_identity(&batch.events).unwrap();
        assert_eq!(at.unix_timestamp(), 1_785_962_763);
        assert_eq!(level, 16);
        assert_eq!(classes, ["WAR", "DRU", "NEC"]);
        assert_eq!(race.as_deref(), Some("Dark Elf"));
        assert_eq!(
            newest_identity(&serde_json::from_str::<LogBatch>(BATCH).unwrap().events),
            None
        );
    }

    #[tokio::test]
    async fn a_who_row_names_the_character_and_never_goes_backwards() {
        let Some((app, _pool)) = live_app().await else {
            return;
        };

        let identity = || {
            Request::builder()
                .uri("/api/v1/characters/erudin/morveus")
                .body(Body::empty())
                .unwrap()
        };
        assert_eq!(json_of(&app, identity()).await.0, StatusCode::NOT_FOUND);

        assert_eq!(
            json_of(&app, post_events("s3cret", WHO)).await.0,
            StatusCode::CREATED
        );
        let (status, view) = json_of(&app, identity()).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(view["name"], "Morveus");
        assert_eq!(view["level"], 16);
        assert_eq!(view["race"], "Dark Elf");
        assert_eq!(view["classes"], serde_json::json!(["WAR", "DRU", "NEC"]));
        assert_eq!(view["identity_at"], "2026-08-05T20:46:03Z");

        let older = r#"{"character":"Morveus","server":"erudin","events":[
            {"at": 1785958164, "kind": "who", "level": 15, "classes": ["WAR"]}
        ]}"#;
        json_of(&app, post_events("s3cret", older)).await;
        let (_, view) = json_of(&app, identity()).await;
        assert_eq!(view["level"], 16, "a replayed older row is ignored");

        let newer = r#"{"character":"Morveus","server":"erudin","events":[
            {"at": 1785999999, "kind": "who", "level": 17, "classes": ["WAR", "DRU"]}
        ]}"#;
        json_of(&app, post_events("s3cret", newer)).await;
        let (_, view) = json_of(&app, identity()).await;
        assert_eq!(view["level"], 17);
        assert_eq!(view["classes"], serde_json::json!(["WAR", "DRU"]));
        assert_eq!(view["race"], serde_json::Value::Null, "anonymity clears it");
    }

    #[tokio::test]
    async fn a_character_who_never_ran_who_reads_as_unknown() {
        let Some((app, _pool)) = live_app().await else {
            return;
        };
        json_of(&app, post_events("s3cret", BATCH)).await;
        let (status, view) = json_of(
            &app,
            Request::builder()
                .uri("/api/v1/characters/erudin/Dorsk")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(view["level"], serde_json::Value::Null);
        assert_eq!(view["race"], serde_json::Value::Null);
        assert_eq!(view["classes"], serde_json::json!([]));
        assert_eq!(view["identity_at"], serde_json::Value::Null);
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

    fn post_fights(token: Option<&str>, body: &str) -> Request<Body> {
        let mut request = Request::builder()
            .method("POST")
            .uri("/api/v1/fights")
            .header("content-type", "application/json");
        if let Some(token) = token {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        request.body(Body::from(body.to_string())).unwrap()
    }

    fn fights_body(fights: &str) -> String {
        format!(r#"{{"character":"Dorsk","server":"erudin","fights":{fights}}}"#)
    }

    fn fights_fixture() -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/fights/eql_fights_Dorsk_erudin.json");
        std::fs::read_to_string(path).expect("fixture is committed")
    }

    #[tokio::test]
    async fn fight_writes_need_the_machine_token() {
        let body = fights_body(r#"[{"start_wall":1785931338.0}]"#);
        assert_eq!(
            status_of(post_fights(None, &body)).await,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            status_of(post_fights(Some("wrong"), &body)).await,
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn fights_validate_before_touching_the_database() {
        let cases = [
            fights_body("[]"),
            fights_body(r#"[{"span":12}]"#),
            fights_body(r#"[{"start_wall":"yesterday"}]"#),
            fights_body(r#"[{"start_wall":1.0e30}]"#),
            r#"{"character":" ","server":"erudin","fights":[{"start_wall":1.0}]}"#.to_string(),
        ];
        for body in cases {
            assert_eq!(
                status_of(post_fights(Some("s3cret"), &body)).await,
                StatusCode::UNPROCESSABLE_ENTITY,
                "{body}"
            );
        }
    }

    #[tokio::test]
    async fn an_unparsable_fight_cursor_is_rejected() {
        let request = Request::builder()
            .uri("/api/v1/characters/erudin/Dorsk/fights?before=yesterday")
            .body(Body::empty())
            .unwrap();
        assert_eq!(status_of(request).await, StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn fights_accumulate_dedupe_and_page_newest_first() {
        let Some((app, pool)) = live_app().await else {
            return;
        };

        let (status, accepted) = json_of(
            &app,
            post_fights(Some("s3cret"), &fights_body(&fights_fixture())),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(accepted["received"], 13);
        assert_eq!(accepted["stored"], 13);

        let (status, again) = json_of(
            &app,
            post_fights(Some("s3cret"), &fights_body(&fights_fixture())),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(again["received"], 13);
        assert_eq!(again["stored"], 0, "re-posting the same fights is a no-op");

        let stored: i64 = sqlx::query_scalar("select count(*) from fights")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(stored, 13);

        let page = |query: &str| {
            Request::builder()
                .uri(format!("/api/v1/characters/erudin/dorsk/fights?{query}"))
                .body(Body::empty())
                .unwrap()
        };

        let (status, fights) = json_of(&app, page("limit=200")).await;
        assert_eq!(status, StatusCode::OK);
        let fights = fights.as_array().unwrap().clone();
        assert_eq!(fights.len(), 13);
        assert_eq!(fights[0]["start_wall"], 1785960884.0);
        assert_eq!(fights[0]["started_at"], "2026-08-05T20:14:44Z");
        assert_eq!(fights[0]["fight"]["zone"], "The Greater Faydark");
        assert_eq!(fights[0]["fight"]["kills"], 5);
        assert_eq!(fights[0]["fight"]["abilities_dmg"]["Icestrike"]["hits"], 11);
        assert!(
            fights
                .windows(2)
                .all(|pair| pair[0]["start_wall"].as_f64() > pair[1]["start_wall"].as_f64()),
            "newest first"
        );

        let (_, first) = json_of(&app, page("limit=2")).await;
        assert_eq!(first.as_array().unwrap().len(), 2);

        let (_, older) = json_of(&app, page("limit=200&before=2026-08-05T20:14:44Z")).await;
        let older = older.as_array().unwrap().clone();
        assert_eq!(older.len(), 12);
        assert_eq!(older[0]["start_wall"], 1785960439.0);

        let sparse = r#"[{"start_wall":1785931338.5,"span":0,"active_secs":0,"enemies":[],
                          "dmg_out_you":0,"dmg_in_you":0,"heal_out":0,"kills":0,"deaths":0}]"#;
        let (status, accepted) =
            json_of(&app, post_fights(Some("s3cret"), &fights_body(sparse))).await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(accepted["stored"], 1, "a fight with no zone still stores");

        let (_, listed) = json_of(&app, page("limit=200")).await;
        assert_eq!(listed.as_array().unwrap().len(), 14);

        let characters: i64 = sqlx::query_scalar("select count(*) from characters")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(characters, 1);

        let (status, unknown) = json_of(&app, {
            Request::builder()
                .uri("/api/v1/characters/erudin/Nobody/fights")
                .body(Body::empty())
                .unwrap()
        })
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(unknown.as_array().unwrap().len(), 0);
    }

    #[test]
    fn fight_meta_reads_the_columns_it_needs_and_tolerates_the_rest() {
        let (meta, at) = fight_meta(&serde_json::json!({
            "start_wall": 1785931338.0,
            "zone": "Najena 4 (Refined)",
            "enemies": ["a greater skeleton"],
            "dmg_out_you": 7654,
            "kills": 5,
            "unheard_of_field": {"a": 1}
        }))
        .unwrap();
        assert_eq!(meta.zone.as_deref(), Some("Najena 4 (Refined)"));
        assert_eq!(meta.enemies, ["a greater skeleton"]);
        assert_eq!(meta.dmg_out_you, 7654);
        assert_eq!(meta.kills, 5);
        assert_eq!(meta.deaths, 0);
        assert_eq!(at.unix_timestamp(), 1785931338);

        let bare = fight_meta(&serde_json::json!({ "start_wall": 1.0 })).unwrap();
        assert_eq!(bare.0.zone, None);
        assert!(bare.0.enemies.is_empty());

        assert!(fight_meta(&serde_json::json!({})).is_err());
        assert!(fight_meta(&serde_json::json!([])).is_err());
        assert!(fight_meta(&serde_json::json!({ "start_wall": 1e30 })).is_err());
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
    fn upgrade_suffix_strips_only_trailing_plus_digits() {
        assert_eq!(upgrade_suffix("Bronze Helm +5"), Some(("Bronze Helm", 5)));
        assert_eq!(
            upgrade_suffix("Drop of Crystallized Flame +12"),
            Some(("Drop of Crystallized Flame", 12))
        );
        assert_eq!(upgrade_suffix("Bronze Helm"), None);
        assert_eq!(upgrade_suffix("Bronze Helm +"), None);
        assert_eq!(upgrade_suffix("Bronze Helm +5a"), None);
        assert_eq!(upgrade_suffix("Bronze Helm+5"), None);
        assert_eq!(upgrade_suffix(" +5"), None);
    }

    #[test]
    fn name_candidates_peel_decorations_until_the_base_name() {
        let folded = |name: &str| -> Vec<(String, Option<u32>)> { name_candidates(name) };
        assert_eq!(
            folded("Backpack*"),
            vec![("backpack*".into(), None), ("backpack".into(), None)]
        );
        assert_eq!(
            folded("Savant's Cap (Exaltation)"),
            vec![
                ("savant's cap (exaltation)".into(), None),
                ("savant's cap".into(), None)
            ]
        );
        assert_eq!(
            folded("Gossamer Cap (Exaltation) +2"),
            vec![
                ("gossamer cap (exaltation) +2".into(), None),
                ("gossamer cap (exaltation)".into(), Some(2)),
                ("gossamer cap".into(), Some(2)),
            ],
            "the upgrade level survives further peeling"
        );
        assert_eq!(folded("Bone Chips"), vec![("bone chips".into(), None)]);
        assert_eq!(folded("*"), vec![("*".into(), None)]);
    }

    #[test]
    fn resolve_item_matches_starred_and_parenthesised_names() {
        let mut items = HashMap::new();
        items.insert("backpack".to_string(), item_named(1, "Backpack"));
        items.insert("savant's cap".to_string(), item_named(2, "Savant's Cap"));

        let (item, upgrade) = resolve_item(&items, "Backpack*");
        assert_eq!(item.unwrap().id, 1);
        assert_eq!(upgrade, None);

        let (item, upgrade) = resolve_item(&items, "Savant's Cap (Exaltation) +3");
        assert_eq!(item.unwrap().id, 2);
        assert_eq!(upgrade, Some(3));
    }

    fn item_named(id: i64, name: &str) -> ItemRecord {
        ItemRecord {
            id,
            game_id: None,
            name: name.to_string(),
            stats: serde_json::json!({}),
            scraped_at: OffsetDateTime::UNIX_EPOCH,
            from_dump: false,
        }
    }

    #[test]
    fn resolve_item_prefers_exact_then_falls_back_to_base_name() {
        let mut items = HashMap::new();
        items.insert("bronze helm".to_string(), item_named(1, "Bronze Helm"));
        items.insert(
            "bronze helm +5".to_string(),
            item_named(2, "Bronze Helm +5"),
        );

        let (item, upgrade) = resolve_item(&items, "Bronze Helm +5");
        assert_eq!(item.unwrap().id, 2, "exact match wins");
        assert_eq!(upgrade, None);

        let (item, upgrade) = resolve_item(&items, "Bronze Helm +3");
        assert_eq!(item.unwrap().id, 1, "falls back to the base item");
        assert_eq!(upgrade, Some(3));

        let (item, upgrade) = resolve_item(&items, "Cloth Cap +2");
        assert!(item.is_none());
        assert_eq!(upgrade, None);
    }

    #[tokio::test]
    async fn upgraded_items_resolve_to_base_stats_in_inventory_and_stats() {
        let Some((app, pool)) = live_app().await else {
            return;
        };
        let stats_json = |name: &str, ac, hp, mana| {
            serde_json::to_value(ItemStats {
                name: name.to_string(),
                ac: Some(ac),
                hp: Some(hp),
                mana: Some(mana),
                ..Default::default()
            })
            .unwrap()
        };
        for (name, ac, hp, mana) in [("Bronze Helm", 14, 20, 10), ("Mithril Earring", 2, 15, 15)] {
            sqlx::query(
                "insert into items (name, stats, wikitext) values ($1, $2, '') \
                 on conflict (name) do update set stats = excluded.stats",
            )
            .bind(name)
            .bind(SqlJson(stats_json(name, ac, hp, mana)))
            .execute(&pool)
            .await
            .unwrap();
        }

        let upload = r#"{"character":"Dorsk","server":"erudin","entries":[
            {"location":"Head","name":"Bronze Helm +5","id":4201,"count":1,"slots":10},
            {"location":"Ear","name":"Mithril Earring +2","id":10041,"count":1,"slots":10},
            {"location":"Ear","name":"Unknown Bauble +1","id":9,"count":1,"slots":10},
            {"location":"Neck","name":"Empty","id":0,"count":0,"slots":0}]}"#;
        let request = Request::builder()
            .method("POST")
            .uri("/api/v1/inventory")
            .header("authorization", "Bearer s3cret")
            .header("content-type", "application/json")
            .body(Body::from(upload))
            .unwrap();
        let (status, _) = json_of(&app, request).await;
        assert_eq!(status, StatusCode::CREATED);

        let (status, inventory) = json_of(
            &app,
            Request::builder()
                .uri("/api/v1/characters/erudin/Dorsk/inventory")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let entries = inventory["entries"].as_array().unwrap();
        assert_eq!(entries[0]["name"], "Bronze Helm +5", "display name kept");
        assert_eq!(entries[0]["item"]["name"], "Bronze Helm");
        assert_eq!(entries[0]["upgrade"], 5);
        assert_eq!(entries[1]["item"]["name"], "Mithril Earring");
        assert_eq!(entries[1]["upgrade"], 2);
        assert!(entries[2]["item"].is_null());
        assert!(entries[2]["upgrade"].is_null());

        let (status, stats) = json_of(
            &app,
            Request::builder()
                .uri("/api/v1/characters/erudin/Dorsk/stats")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(stats["stats"]["ac"], 16);
        assert_eq!(stats["stats"]["hp"], 35);
        assert_eq!(stats["stats"]["mana"], 25);
        assert_eq!(stats["stats"]["known_items"], 2);
        assert_eq!(stats["stats"]["unknown_items"], 1);
    }

    #[tokio::test]
    async fn the_item_dump_fills_icons_the_wiki_has_no_page_for() {
        let Some((app, pool)) = live_app().await else {
            return;
        };
        sqlx::query("truncate item_icon_names")
            .execute(&pool)
            .await
            .unwrap();
        let path = std::env::temp_dir().join("eqls-item-dump-test.sql");
        std::fs::write(&path, crate::itemdump::SAMPLE).unwrap();
        let load = |path: &std::path::Path| vec!["--file".to_string(), path.display().to_string()];
        let summary = crate::itemdump::run(&pool, &load(&path)).await.unwrap();
        assert_eq!(summary.rows, 8);
        assert_eq!(summary.loaded, 5);
        assert_eq!(summary.skipped, 3);
        assert_eq!(
            crate::itemdump::run(&pool, &load(&path)).await.unwrap(),
            summary,
            "re-running the loader is idempotent"
        );
        std::fs::remove_file(&path).unwrap();

        for (name, icon) in [("Spirit Reaver", Some(576)), ("Cloth Cap", None)] {
            sqlx::query(
                "insert into items (name, stats, wikitext) values ($1, $2, '') \
                 on conflict (name) do update set stats = excluded.stats",
            )
            .bind(name)
            .bind(SqlJson(
                serde_json::to_value(ItemStats {
                    name: name.to_string(),
                    icon,
                    ac: Some(4),
                    ..Default::default()
                })
                .unwrap(),
            ))
            .execute(&pool)
            .await
            .unwrap();
        }

        let upload = r#"{"character":"Dorsk","server":"erudin","entries":[
            {"location":"Waist","name":"Small Bronze Girdle +2","id":3256,"count":1,"slots":10},
            {"location":"Primary","name":"Spirit Reaver","id":2578,"count":1,"slots":10},
            {"location":"Head","name":"Cloth Cap*","id":1001,"count":1,"slots":10},
            {"location":"Feet","name":"Nothing Knows This","id":7,"count":1,"slots":10},
            {"location":"Neck","name":"Empty","id":0,"count":0,"slots":0}]}"#;
        let (status, _) = json_of(
            &app,
            Request::builder()
                .method("POST")
                .uri("/api/v1/inventory")
                .header("authorization", "Bearer s3cret")
                .header("content-type", "application/json")
                .body(Body::from(upload))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);

        let (status, inventory) = json_of(
            &app,
            Request::builder()
                .uri("/api/v1/characters/erudin/Dorsk/inventory")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let entries = inventory["entries"].as_array().unwrap();

        assert_eq!(entries[0]["item"]["stats"]["icon"], 549, "no wiki page");
        assert_eq!(entries[0]["item"]["name"], "Small Bronze Girdle");
        assert_eq!(entries[0]["item"]["game_id"], 3256);
        assert_eq!(entries[0]["upgrade"], 2, "the decoration still peels");
        assert_eq!(entries[1]["item"]["stats"]["icon"], 576, "the wiki wins");
        assert_eq!(entries[1]["item"]["stats"]["ac"], 4);
        assert_eq!(
            entries[2]["item"]["stats"]["icon"], 639,
            "a wiki page with no icon takes the dump's"
        );
        assert_eq!(entries[2]["item"]["stats"]["ac"], 4, "wiki stats survive");
        assert!(
            entries[3]["item"].is_null(),
            "unknown to both stays unknown"
        );
        assert!(entries[4]["item"].is_null());

        let (_, stats) = json_of(
            &app,
            Request::builder()
                .uri("/api/v1/characters/erudin/Dorsk/stats")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(stats["stats"]["ac"], 8);
        assert_eq!(stats["stats"]["known_items"], 2);
        assert_eq!(
            stats["stats"]["unknown_items"], 2,
            "an icon alone is not knowing the item"
        );
    }

    /// A bad route pattern only panics when the `Router` is assembled, so this
    /// keeps a route-syntax mistake from reaching `main`.
    #[tokio::test]
    async fn the_router_accepts_every_route_pattern() {
        let _ = test_app();
    }

    fn put_sheet(sheet: u32, token: Option<&str>, dds: Vec<u8>) -> Request<Body> {
        let mut request = Request::builder()
            .method("PUT")
            .uri(format!("/api/v1/icons/sheets/{sheet}"))
            .header("content-type", "application/octet-stream");
        if let Some(token) = token {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        request.body(Body::from(dds)).unwrap()
    }

    #[tokio::test]
    async fn icon_sheet_uploads_need_the_machine_token() {
        let dds = crate::icons::dxt5_sheet(&[]);
        assert_eq!(
            status_of(put_sheet(1, None, dds.clone())).await,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            status_of(put_sheet(1, Some("wrong"), dds)).await,
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn undecodable_sheets_are_rejected_before_the_database() {
        assert_eq!(
            status_of(put_sheet(1, Some("s3cret"), b"not a dds".to_vec())).await,
            StatusCode::UNPROCESSABLE_ENTITY
        );
        assert_eq!(
            status_of(put_sheet(0, Some("s3cret"), crate::icons::dxt5_sheet(&[]))).await,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }

    #[tokio::test]
    async fn icon_reads_reject_bad_names_and_ids_without_a_query() {
        let get = |path: &str| {
            Request::builder()
                .uri(format!("/api/v1/icons/{path}"))
                .body(Body::empty())
                .unwrap()
        };
        for path in ["624", "624.jpg", "notanicon.png", "499.png", "14144.png"] {
            assert_eq!(status_of(get(path)).await, StatusCode::NOT_FOUND, "{path}");
        }
        assert_eq!(
            status_of(get("624.png")).await,
            StatusCode::SERVICE_UNAVAILABLE,
            "an in-range id does reach the database"
        );
    }

    #[tokio::test]
    async fn icon_sheets_upsert_and_serve_cropped_pngs() {
        let Some((app, pool)) = live_app().await else {
            return;
        };
        sqlx::query("truncate item_icons")
            .execute(&pool)
            .await
            .unwrap();

        let (status, accepted) = json_of(
            &app,
            put_sheet(4, Some("s3cret"), crate::icons::dxt5_sheet(&[(2, 4)])),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(accepted["sheet"], 4);
        assert_eq!(accepted["icons"], 35);

        let stored: i64 = sqlx::query_scalar("select count(*) from item_icons")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(stored, 35);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/icons/608.png")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["content-type"], "image/png");
        assert_eq!(
            response.headers()["cache-control"],
            "public, max-age=31536000, immutable"
        );
        assert_eq!(response.headers()["etag"], "\"icon-608\"");
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let png = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png).unwrap();
        assert_eq!(png.width(), 40);
        assert_eq!(png.height(), 40);

        assert_eq!(
            status_of_on(&app, "/api/v1/icons/624.png").await,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            status_of_on(&app, "/api/v1/icons/500.png").await,
            StatusCode::NOT_FOUND
        );

        let (_, again) = json_of(
            &app,
            put_sheet(4, Some("s3cret"), crate::icons::bgra_sheet(&[])),
        )
        .await;
        assert_eq!(again["icons"], 36);
        assert_eq!(
            status_of_on(&app, "/api/v1/icons/624.png").await,
            StatusCode::OK
        );
    }

    async fn status_of_on(app: &Router, uri: &str) -> StatusCode {
        app.clone()
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap()
            .status()
    }

    #[tokio::test]
    async fn reparse_rewrites_stats_from_the_stored_wikitext() {
        let Some((app, pool)) = live_app().await else {
            return;
        };
        let wikitext =
            "{{Itempage|itemname = Bronze Helm|lucy_img_ID = 550|statsblock = \nAC: 14<br>\n}}";
        sqlx::query(
            "insert into items (name, stats, wikitext) values ('Reparse Probe', '{}'::jsonb, $1) \
             on conflict (name) do update set stats = '{}'::jsonb, wikitext = excluded.wikitext",
        )
        .bind(wikitext)
        .execute(&pool)
        .await
        .unwrap();

        let summary = crate::scrape::reparse(&pool).await.unwrap();
        assert!(summary.upserted >= 1);

        let (status, item) = json_of(
            &app,
            Request::builder()
                .uri("/api/v1/items/Reparse%20Probe")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(item["stats"]["icon"], 550);
        assert_eq!(item["stats"]["ac"], 14);
    }

    #[test]
    fn constant_time_eq_matches_only_identical_tokens() {
        assert!(constant_time_eq(b"s3cret", b"s3cret"));
        assert!(!constant_time_eq(b"s3crea", b"s3cret"));
        assert!(!constant_time_eq(b"", b"s3cret"));
        assert!(!constant_time_eq(b"s3cret-longer", b"s3cret"));
    }
}
