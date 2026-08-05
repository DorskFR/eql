use crate::icons;
use sqlx::{PgPool, Row};
use std::{collections::HashMap, path::PathBuf, time::Duration};

const DUMP_URL: &str = "https://raw.githubusercontent.com/quarmdb/quarmdb/master/db/combined.sql";
const USER_AGENT: &str = concat!(
    "eql-scraper/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/DorskFR/eql)"
);
const TABLE: &str = "CREATE TABLE `items` (";
const INSERT: &str = "INSERT INTO `items` VALUES";
const CHUNK: usize = 2000;

#[derive(Debug, Default, PartialEq)]
pub struct Summary {
    pub rows: usize,
    pub loaded: usize,
    pub skipped: usize,
}

#[derive(Debug, PartialEq)]
pub enum Source {
    Url(String),
    File(PathBuf),
}

#[derive(Debug, Clone, PartialEq)]
pub struct DumpIcon {
    pub key: String,
    pub name: String,
    pub game_id: i64,
    pub icon: i32,
}

#[derive(Debug, thiserror::Error)]
pub enum DumpError {
    #[error("usage: eqls item-dump [--url <url>] [--file <path>]")]
    Usage,
    #[error(transparent)]
    Http(#[from] reqwest::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error("the dump has no `items` table definition")]
    NoTable,
    #[error("the `items` table has no {0:?} column")]
    NoColumn(&'static str),
    #[error("the dump has no usable `items` rows")]
    NoRows,
}

pub fn parse_args(args: &[String]) -> Result<Source, DumpError> {
    let mut source = Source::Url(DUMP_URL.to_string());
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        let value = iter.next().ok_or(DumpError::Usage)?;
        match arg.as_str() {
            "--url" => source = Source::Url(value.clone()),
            "--file" => source = Source::File(PathBuf::from(value)),
            _ => return Err(DumpError::Usage),
        }
    }
    Ok(source)
}

/// Quarm drops the apostrophe from some item names and keeps it on others, so
/// both sides of the join are folded the same way.
pub fn name_key(name: &str) -> String {
    name.trim()
        .to_lowercase()
        .chars()
        .filter(|c| *c != '\'' && *c != '\u{2019}')
        .collect()
}

pub async fn run(pool: &PgPool, args: &[String]) -> Result<Summary, DumpError> {
    let source = parse_args(args)?;
    let sql = match &source {
        Source::File(path) => std::fs::read_to_string(path)?,
        Source::Url(url) => {
            tracing::info!(%url, "fetching community item dump");
            reqwest::Client::builder()
                .user_agent(USER_AGENT)
                .timeout(Duration::from_secs(600))
                .build()?
                .get(url)
                .send()
                .await?
                .error_for_status()?
                .text()
                .await?
        }
    };

    let (items, rows) = parse(&sql)?;
    let loaded = store(pool, &items).await?;
    let summary = Summary {
        rows,
        loaded,
        skipped: rows - items.len(),
    };
    tracing::info!(
        rows = summary.rows,
        loaded = summary.loaded,
        skipped = summary.skipped,
        "item dump loaded"
    );
    Ok(summary)
}

pub fn parse(sql: &str) -> Result<(Vec<DumpIcon>, usize), DumpError> {
    let columns = columns(sql).ok_or(DumpError::NoTable)?;
    let column = |wanted: &'static str| {
        columns
            .iter()
            .position(|name| name.eq_ignore_ascii_case(wanted))
            .ok_or(DumpError::NoColumn(wanted))
    };
    let (id_at, name_at, icon_at) = (column("id")?, column("Name")?, column("icon")?);

    let mut rows = 0usize;
    let mut items: HashMap<String, DumpIcon> = HashMap::new();
    for statement in sql.lines().filter(|line| line.starts_with(INSERT)) {
        for tuple in tuples(statement) {
            rows += 1;
            let fields = fields(&tuple);
            if fields.len() != columns.len() {
                continue;
            }
            let (Ok(game_id), Ok(icon)) = (
                unquote(fields[id_at]).parse::<i64>(),
                unquote(fields[icon_at]).parse::<i32>(),
            ) else {
                continue;
            };
            let name = unquote(fields[name_at]);
            let key = name_key(&name);
            if key.is_empty() || icons::locate(icon).is_none() {
                continue;
            }
            let found = DumpIcon {
                key: key.clone(),
                name,
                game_id,
                icon,
            };
            items
                .entry(key)
                .and_modify(|kept| {
                    if found.game_id < kept.game_id {
                        *kept = found.clone();
                    }
                })
                .or_insert(found);
        }
    }

    if items.is_empty() {
        return Err(DumpError::NoRows);
    }
    let mut items: Vec<DumpIcon> = items.into_values().collect();
    items.sort_unstable_by(|a, b| a.key.cmp(&b.key));
    Ok((items, rows))
}

pub async fn lookup(
    pool: &PgPool,
    keys: &[String],
) -> Result<HashMap<String, DumpIcon>, sqlx::Error> {
    if keys.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = sqlx::query(
        "select name_key, name, game_id, icon from item_icon_names where name_key = any($1)",
    )
    .bind(keys)
    .fetch_all(pool)
    .await?;
    rows.iter()
        .map(|row| {
            let found = DumpIcon {
                key: row.try_get("name_key")?,
                name: row.try_get("name")?,
                game_id: row
                    .try_get::<Option<i64>, _>("game_id")?
                    .unwrap_or_default(),
                icon: row.try_get("icon")?,
            };
            Ok((found.key.clone(), found))
        })
        .collect()
}

async fn store(pool: &PgPool, items: &[DumpIcon]) -> Result<usize, sqlx::Error> {
    let mut loaded = 0usize;
    for chunk in items.chunks(CHUNK) {
        let keys: Vec<&str> = chunk.iter().map(|item| item.key.as_str()).collect();
        let names: Vec<&str> = chunk.iter().map(|item| item.name.as_str()).collect();
        let game_ids: Vec<i64> = chunk.iter().map(|item| item.game_id).collect();
        let icons: Vec<i32> = chunk.iter().map(|item| item.icon).collect();
        loaded += sqlx::query(
            "insert into item_icon_names (name_key, name, game_id, icon) \
             select * from unnest($1::text[], $2::text[], $3::bigint[], $4::int[]) \
             on conflict (name_key) do update \
             set name = excluded.name, game_id = excluded.game_id, \
                 icon = excluded.icon, updated_at = now()",
        )
        .bind(&keys)
        .bind(&names)
        .bind(&game_ids)
        .bind(&icons)
        .execute(pool)
        .await?
        .rows_affected() as usize;
    }
    Ok(loaded)
}

fn columns(sql: &str) -> Option<Vec<String>> {
    let body = &sql[sql.find(TABLE)?..];
    let body = &body[..body.find("\n);")?];
    Some(
        body.lines()
            .skip(1)
            .filter_map(|line| {
                let (name, _) = line.trim_start().strip_prefix('`')?.split_once('`')?;
                Some(name.to_string())
            })
            .collect(),
    )
}

/// SQL string literals may hold any of `(`, `)` and `,`, so the split has to
/// track quoting rather than lean on the delimiters.
fn tuples(statement: &str) -> Vec<String> {
    let bytes = statement.as_bytes();
    let (mut tuples, mut start, mut quoted, mut at) = (Vec::new(), None, false, 0usize);
    while at < bytes.len() {
        match bytes[at] {
            b'\'' if quoted && bytes.get(at + 1) == Some(&b'\'') => at += 1,
            b'\'' => quoted = !quoted,
            b'(' if !quoted && start.is_none() => start = Some(at + 1),
            b')' if !quoted => {
                if let Some(open) = start.take() {
                    tuples.push(statement[open..at].to_string());
                }
            }
            _ => {}
        }
        at += 1;
    }
    tuples
}

fn fields(tuple: &str) -> Vec<&str> {
    let bytes = tuple.as_bytes();
    let (mut fields, mut start, mut quoted, mut at) = (Vec::new(), 0usize, false, 0usize);
    while at < bytes.len() {
        match bytes[at] {
            b'\'' if quoted && bytes.get(at + 1) == Some(&b'\'') => at += 1,
            b'\'' => quoted = !quoted,
            b',' if !quoted => {
                fields.push(&tuple[start..at]);
                start = at + 1;
            }
            _ => {}
        }
        at += 1;
    }
    fields.push(&tuple[start..]);
    fields
}

fn unquote(field: &str) -> String {
    let field = field.trim();
    match field.strip_prefix('\'').and_then(|f| f.strip_suffix('\'')) {
        Some(inner) => inner.replace("''", "'"),
        None => field.to_string(),
    }
}

#[cfg(test)]
pub(crate) const SAMPLE: &str = concat!(
    "CREATE TABLE `items` (\n",
    "`id` INTEGER NOT NULL DEFAULT 0,\n",
    "`Name` TEXT NOT NULL DEFAULT '',\n",
    "`icon` INTEGER NOT NULL DEFAULT 0,\n",
    "`idfile` TEXT NOT NULL DEFAULT '',\n",
    "PRIMARY KEY (`id`)\n",
    ");\n",
    "INSERT INTO `items` VALUES (1001,'Cloth Cap',639,'IT63'),",
    "(3256,'Small Bronze Girdle',549,'IT63'),",
    "(2578,'Spirit Reaver',576,'IT1'),",
    "(9001,'Dark Ones Cap',700,'a (odd), name'),",
    "(9002,'Karana''s Tear',701,'IT63'),",
    "(9003,'',702,'IT63'),",
    "(9004,'Out Of Range',3,'IT63'),",
    "(9005,'Cloth Cap',888,'IT63');\n",
    "INSERT INTO `spells` VALUES (1,'Not An Item',999,'IT63');\n",
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_flags() {
        assert_eq!(
            parse_args(&[]).unwrap(),
            Source::Url(DUMP_URL.to_string()),
            "the default source is the published dump"
        );
        let args: Vec<String> = ["--file", "/tmp/combined.sql"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            parse_args(&args).unwrap(),
            Source::File(PathBuf::from("/tmp/combined.sql"))
        );
        let args: Vec<String> = ["--url", "http://host/x.sql"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            parse_args(&args).unwrap(),
            Source::Url("http://host/x.sql".into())
        );
        assert!(parse_args(&["--nope".to_string(), "x".to_string()]).is_err());
        assert!(parse_args(&["--url".to_string()]).is_err());
    }

    #[test]
    fn name_keys_fold_case_and_apostrophes() {
        assert_eq!(name_key("Karana's Tear"), "karanas tear");
        assert_eq!(name_key("Karanas Tear"), "karanas tear");
        assert_eq!(name_key("  Small Bronze Girdle "), "small bronze girdle");
        assert_eq!(name_key("Lcea\u{2019}s Jewel Box"), "lceas jewel box");
        assert_eq!(name_key(""), "");
    }

    #[test]
    fn parse_reads_id_name_and_icon_by_column_position() {
        let (items, rows) = parse(SAMPLE).unwrap();
        assert_eq!(rows, 8, "only the items statement is read");
        let by_key = |key: &str| items.iter().find(|item| item.key == key).cloned();

        let girdle = by_key("small bronze girdle").unwrap();
        assert_eq!(girdle.icon, 549);
        assert_eq!(girdle.game_id, 3256);
        assert_eq!(girdle.name, "Small Bronze Girdle");
        assert_eq!(by_key("spirit reaver").unwrap().icon, 576);

        assert_eq!(
            by_key("dark ones cap").unwrap().name,
            "Dark Ones Cap",
            "commas and parentheses inside a literal do not split the row"
        );
        assert_eq!(
            by_key("karanas tear").unwrap().name,
            "Karana's Tear",
            "doubled quotes are unescaped"
        );

        assert!(by_key("").is_none(), "nameless rows are dropped");
        assert!(
            by_key("out of range").is_none(),
            "icons outside the sheet bank are dropped"
        );
        assert!(
            !items.iter().any(|item| item.name == "Not An Item"),
            "other tables are ignored"
        );
        assert_eq!(
            by_key("cloth cap").unwrap().game_id,
            1001,
            "a repeated name keeps the lowest item id"
        );
        assert!(items.windows(2).all(|pair| pair[0].key < pair[1].key));
    }

    #[test]
    fn parse_rejects_a_dump_it_cannot_read() {
        assert!(matches!(parse("nothing here"), Err(DumpError::NoTable)));
        let no_icon = SAMPLE.replace("`icon` INTEGER NOT NULL DEFAULT 0,\n", "");
        assert!(matches!(
            parse(&no_icon),
            Err(DumpError::NoColumn("icon")) | Err(DumpError::NoRows)
        ));
        let empty = &SAMPLE[..SAMPLE.find(INSERT).unwrap()];
        assert!(matches!(parse(empty), Err(DumpError::NoRows)));
    }
}
