use crate::wiki::{parse_item, ItemStats};
use serde::Deserialize;
use sqlx::{types::Json as SqlJson, PgPool, Row};
use std::time::Duration;

const API: &str = "https://eqlwiki.com/api.php";
const USER_AGENT: &str = concat!(
    "eql-scraper/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/DorskFR/eql)"
);
const CATEGORY: &str = "Category:Items";
const BATCH: usize = 50;
const REQUEST_INTERVAL: Duration = Duration::from_secs(1);
const MAX_ATTEMPTS: u32 = 4;

#[derive(Debug, Default)]
pub struct Summary {
    pub fetched: usize,
    pub upserted: usize,
    pub skipped: usize,
    pub unparsed_fields: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum ScrapeError {
    #[error("usage: eqls scrape [--limit N] [--page <title>]")]
    Usage,
    #[error(transparent)]
    Http(#[from] reqwest::Error),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error("wiki api returned no page for {0:?}")]
    MissingPage(String),
}

#[derive(Debug, Deserialize)]
struct QueryResponse {
    #[serde(default)]
    query: Option<QueryBody>,
    #[serde(default, rename = "continue")]
    continuation: Option<Continuation>,
}

#[derive(Debug, Deserialize)]
struct QueryBody {
    #[serde(default)]
    pages: Vec<WikiPage>,
}

#[derive(Debug, Deserialize)]
struct Continuation {
    #[serde(default)]
    gcmcontinue: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WikiPage {
    title: String,
    #[serde(default)]
    missing: bool,
    #[serde(default)]
    revisions: Vec<Revision>,
}

#[derive(Debug, Deserialize)]
struct Revision {
    #[serde(default)]
    slots: Option<Slots>,
}

#[derive(Debug, Deserialize)]
struct Slots {
    main: Slot,
}

#[derive(Debug, Deserialize)]
struct Slot {
    #[serde(default, rename = "content")]
    content: String,
}

impl WikiPage {
    fn wikitext(&self) -> Option<&str> {
        self.revisions
            .first()?
            .slots
            .as_ref()
            .map(|slots| slots.main.content.as_str())
    }
}

pub fn parse_args(args: &[String]) -> Result<(Option<usize>, Option<String>), ScrapeError> {
    let mut limit = None;
    let mut page = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--limit" => {
                limit = Some(
                    iter.next()
                        .and_then(|v| v.parse().ok())
                        .ok_or(ScrapeError::Usage)?,
                )
            }
            "--page" => page = Some(iter.next().ok_or(ScrapeError::Usage)?.clone()),
            _ => return Err(ScrapeError::Usage),
        }
    }
    Ok((limit, page))
}

pub async fn run(pool: &PgPool, args: &[String]) -> Result<Summary, ScrapeError> {
    let (limit, page) = parse_args(args)?;
    let client = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(30))
        .build()?;

    let mut summary = Summary::default();
    match page {
        Some(title) => {
            let pages = fetch_titles(&client, &title).await?;
            let page = pages
                .into_iter()
                .find(|p| !p.missing)
                .ok_or_else(|| ScrapeError::MissingPage(title.clone()))?;
            store_page(pool, &page, &mut summary).await?;
        }
        None => scrape_category(&client, pool, limit, &mut summary).await?,
    }

    tracing::info!(
        fetched = summary.fetched,
        upserted = summary.upserted,
        skipped = summary.skipped,
        unparsed_fields = summary.unparsed_fields,
        "scrape complete"
    );
    Ok(summary)
}

async fn scrape_category(
    client: &reqwest::Client,
    pool: &PgPool,
    limit: Option<usize>,
    summary: &mut Summary,
) -> Result<(), ScrapeError> {
    let mut cursor: Option<String> = None;
    loop {
        let response = fetch_category_batch(client, cursor.as_deref()).await?;
        let pages = response.query.map(|q| q.pages).unwrap_or_default();
        for page in &pages {
            store_page(pool, page, summary).await?;
            if summary.fetched.is_multiple_of(BATCH) {
                tracing::info!(
                    fetched = summary.fetched,
                    upserted = summary.upserted,
                    skipped = summary.skipped,
                    "scrape progress"
                );
            }
            if limit.is_some_and(|max| summary.fetched >= max) {
                return Ok(());
            }
        }
        cursor = response.continuation.and_then(|c| c.gcmcontinue);
        if cursor.is_none() {
            return Ok(());
        }
    }
}

async fn store_page(
    pool: &PgPool,
    page: &WikiPage,
    summary: &mut Summary,
) -> Result<(), ScrapeError> {
    summary.fetched += 1;
    let Some(wikitext) = page.wikitext() else {
        summary.skipped += 1;
        return Ok(());
    };
    let Some(stats) = parse_item(&page.title, wikitext) else {
        tracing::debug!(page = %page.title, "no Itempage template");
        summary.skipped += 1;
        return Ok(());
    };
    if !stats.unparsed.is_empty() {
        summary.unparsed_fields += stats.unparsed.len();
        tracing::warn!(page = %page.title, fields = ?stats.unparsed, "unparsed statsblock fields");
    }
    upsert(pool, &stats, wikitext).await?;
    summary.upserted += 1;
    Ok(())
}

pub async fn upsert(pool: &PgPool, stats: &ItemStats, wikitext: &str) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "insert into items (name, stats, wikitext, scraped_at) \
         values ($1, $2, $3, now()) \
         on conflict (name) do update \
         set stats = excluded.stats, wikitext = excluded.wikitext, scraped_at = excluded.scraped_at \
         returning id",
    )
    .bind(&stats.name)
    .bind(SqlJson(stats))
    .bind(wikitext)
    .fetch_one(pool)
    .await
}

/// Lets a parser change land without re-fetching every wiki page.
pub async fn reparse(pool: &PgPool) -> Result<Summary, ScrapeError> {
    let rows = sqlx::query("select id, name, wikitext from items order by id")
        .fetch_all(pool)
        .await?;

    let mut summary = Summary::default();
    for row in &rows {
        summary.fetched += 1;
        let id: i64 = row.try_get("id")?;
        let name: String = row.try_get("name")?;
        let wikitext: String = row.try_get("wikitext")?;
        let Some(stats) = parse_item(&name, &wikitext) else {
            summary.skipped += 1;
            continue;
        };
        summary.unparsed_fields += stats.unparsed.len();
        sqlx::query("update items set stats = $2 where id = $1")
            .bind(id)
            .bind(SqlJson(&stats))
            .execute(pool)
            .await?;
        summary.upserted += 1;
    }

    tracing::info!(
        items = summary.fetched,
        rewritten = summary.upserted,
        skipped = summary.skipped,
        "reparse complete"
    );
    Ok(summary)
}

async fn fetch_category_batch(
    client: &reqwest::Client,
    cursor: Option<&str>,
) -> Result<QueryResponse, ScrapeError> {
    let mut params = vec![
        ("action", "query".to_string()),
        ("format", "json".to_string()),
        ("formatversion", "2".to_string()),
        ("generator", "categorymembers".to_string()),
        ("gcmtitle", CATEGORY.to_string()),
        ("gcmnamespace", "0".to_string()),
        ("gcmlimit", BATCH.to_string()),
        ("prop", "revisions".to_string()),
        ("rvprop", "content".to_string()),
        ("rvslots", "main".to_string()),
        ("maxlag", "5".to_string()),
    ];
    if let Some(cursor) = cursor {
        params.push(("gcmcontinue", cursor.to_string()));
    }
    request(client, &params).await
}

async fn fetch_titles(
    client: &reqwest::Client,
    titles: &str,
) -> Result<Vec<WikiPage>, ScrapeError> {
    let params = vec![
        ("action", "query".to_string()),
        ("format", "json".to_string()),
        ("formatversion", "2".to_string()),
        ("titles", titles.to_string()),
        ("redirects", "1".to_string()),
        ("prop", "revisions".to_string()),
        ("rvprop", "content".to_string()),
        ("rvslots", "main".to_string()),
        ("maxlag", "5".to_string()),
    ];
    let response: QueryResponse = request(client, &params).await?;
    Ok(response.query.map(|q| q.pages).unwrap_or_default())
}

async fn request(
    client: &reqwest::Client,
    params: &[(&str, String)],
) -> Result<QueryResponse, ScrapeError> {
    let mut backoff = Duration::from_secs(1);
    for attempt in 1..=MAX_ATTEMPTS {
        tokio::time::sleep(REQUEST_INTERVAL).await;
        let outcome = client
            .get(API)
            .query(params)
            .send()
            .await
            .and_then(|r| r.error_for_status());
        let error = match outcome {
            Ok(response) => match response.json::<QueryResponse>().await {
                Ok(body) => return Ok(body),
                Err(err) => err,
            },
            Err(err) => err,
        };
        if attempt == MAX_ATTEMPTS {
            return Err(error.into());
        }
        tracing::warn!(%error, attempt, backoff_ms = backoff.as_millis(), "wiki request failed");
        tokio::time::sleep(backoff).await;
        backoff *= 2;
    }
    unreachable!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_flags() {
        let args: Vec<String> = ["--limit", "15"].iter().map(|s| s.to_string()).collect();
        assert_eq!(parse_args(&args).unwrap(), (Some(15), None));

        let args: Vec<String> = ["--page", "Spirit Reaver"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            parse_args(&args).unwrap(),
            (None, Some("Spirit Reaver".into()))
        );
        assert!(parse_args(&[]).unwrap() == (None, None));
        assert!(parse_args(&["--nope".to_string()]).is_err());
        assert!(parse_args(&["--limit".to_string()]).is_err());
    }

    #[test]
    fn reads_generator_response_shape() {
        let body = r#"{
            "continue": {"gcmcontinue": "page|41|52641"},
            "query": {"pages": [
                {"pageid": 1, "title": "Bone Chips", "revisions": [
                    {"slots": {"main": {"contentmodel": "wikitext", "content": "{{Itempage|itemname = Bone Chips|statsblock = \nWT: 0.1  Size: SMALL<br>\n}}"}}}
                ]},
                {"title": "Nope", "missing": true}
            ]}
        }"#;
        let response: QueryResponse = serde_json::from_str(body).unwrap();
        let pages = response.query.unwrap().pages;
        assert_eq!(pages.len(), 2);
        assert!(pages[1].missing && pages[1].wikitext().is_none());
        let stats = parse_item(&pages[0].title, pages[0].wikitext().unwrap()).unwrap();
        assert_eq!(stats.name, "Bone Chips");
        assert_eq!(stats.weight, Some(0.1));
        assert_eq!(
            response.continuation.unwrap().gcmcontinue.unwrap(),
            "page|41|52641"
        );
    }
}
