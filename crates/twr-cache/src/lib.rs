//! SQLite entity cache + watchlist (bead twitter_cli-5o3.7.1).
//!
//! discord-db pattern port: WAL mode, `foreign_keys=ON`, FTS5 full-text
//! search over tweet text. Critical gotcha preserved: tweet upserts use
//! `ON CONFLICT DO UPDATE`, never `INSERT OR REPLACE` (REPLACE =
//! delete+insert, violating foreign keys with attached media on re-sync).
//! Cursor-based `sync_state` per feed-key (channel/user/search-query).

use rusqlite::{params, Connection};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CacheError {
    #[error("cache db: {0}")]
    Db(String),
}

impl From<rusqlite::Error> for CacheError {
    fn from(e: rusqlite::Error) -> Self {
        CacheError::Db(e.to_string())
    }
}

/// Default DB path: `~/.twr/cache.db`.
pub fn default_db_path() -> Option<std::path::PathBuf> {
    home_dir().map(|h| h.join(".twr").join("cache.db"))
}

fn home_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(std::path::PathBuf::from)
}

const SCHEMA: &str = "
PRAGMA journal_mode=WAL;
PRAGMA foreign_keys=ON;
CREATE TABLE IF NOT EXISTS tweets (
    id TEXT PRIMARY KEY,
    author_screen_name TEXT NOT NULL DEFAULT '',
    text TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT '',
    metrics_json TEXT NOT NULL DEFAULT '{}',
    synced_at_secs INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS media (
    tweet_id TEXT NOT NULL REFERENCES tweets(id) ON DELETE CASCADE,
    url TEXT NOT NULL,
    media_type TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (tweet_id, url)
);
CREATE TABLE IF NOT EXISTS sync_state (
    feed_key TEXT PRIMARY KEY,
    cursor TEXT,
    updated_at_secs INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS watchlist (
    screen_name TEXT PRIMARY KEY,
    added_at_secs INTEGER NOT NULL DEFAULT 0
);
CREATE VIRTUAL TABLE IF NOT EXISTS tweets_fts USING fts5(id, text, content='tweets', content_rowid='rowid');
CREATE TRIGGER IF NOT EXISTS tweets_ai AFTER INSERT ON tweets BEGIN
    INSERT INTO tweets_fts(rowid, id, text) VALUES (new.rowid, new.id, new.text);
END;
CREATE TRIGGER IF NOT EXISTS tweets_ad AFTER DELETE ON tweets BEGIN
    INSERT INTO tweets_fts(tweets_fts, rowid, id, text) VALUES ('delete', old.rowid, old.id, old.text);
END;
CREATE TRIGGER IF NOT EXISTS tweets_au AFTER UPDATE ON tweets BEGIN
    INSERT INTO tweets_fts(tweets_fts, rowid, id, text) VALUES ('delete', old.rowid, old.id, old.text);
    INSERT INTO tweets_fts(rowid, id, text) VALUES (new.rowid, new.id, new.text);
END;
";

/// Open (creating parents) + migrate + set pragmas.
pub fn open(path: &std::path::Path) -> Result<Connection, CacheError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| CacheError::Db(e.to_string()))?;
    }
    let conn = Connection::open(path)?;
    conn.execute_batch(SCHEMA)?;
    Ok(conn)
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Upsert one tweet + its media (DO UPDATE, never REPLACE).
pub fn upsert_tweet(conn: &Connection, tweet: &twr_model::Tweet) -> Result<(), CacheError> {
    let metrics = serde_json::to_string(&tweet.metrics).unwrap_or_else(|_| "{}".into());
    conn.execute(
        "INSERT INTO tweets (id, author_screen_name, text, created_at, metrics_json, synced_at_secs)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(id) DO UPDATE SET
           author_screen_name=excluded.author_screen_name,
           text=excluded.text,
           created_at=excluded.created_at,
           metrics_json=excluded.metrics_json,
           synced_at_secs=excluded.synced_at_secs",
        params![
            tweet.id,
            tweet.author.screen_name,
            tweet.text,
            tweet.created_at,
            metrics,
            now_secs() as i64,
        ],
    )?;
    for m in &tweet.media {
        conn.execute(
            "INSERT INTO media (tweet_id, url, media_type) VALUES (?1, ?2, ?3)
             ON CONFLICT(tweet_id, url) DO UPDATE SET media_type=excluded.media_type",
            params![tweet.id, m.url, m.media_type],
        )?;
    }
    Ok(())
}

/// Upsert a batch (single transaction).
pub fn upsert_tweets(
    conn: &mut Connection,
    tweets: &[twr_model::Tweet],
) -> Result<usize, CacheError> {
    let tx = conn.transaction()?;
    let mut n = 0;
    for t in tweets {
        let metrics = serde_json::to_string(&t.metrics).unwrap_or_else(|_| "{}".into());
        tx.execute(
            "INSERT INTO tweets (id, author_screen_name, text, created_at, metrics_json, synced_at_secs)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET
               author_screen_name=excluded.author_screen_name,
               text=excluded.text,
               created_at=excluded.created_at,
               metrics_json=excluded.metrics_json,
               synced_at_secs=excluded.synced_at_secs",
            params![t.id, t.author.screen_name, t.text, t.created_at, metrics, now_secs() as i64],
        )?;
        for m in &t.media {
            tx.execute(
                "INSERT INTO media (tweet_id, url, media_type) VALUES (?1, ?2, ?3)
                 ON CONFLICT(tweet_id, url) DO UPDATE SET media_type=excluded.media_type",
                params![t.id, m.url, m.media_type],
            )?;
        }
        n += 1;
    }
    tx.commit()?;
    Ok(n)
}

/// FTS5 search over cached tweet text. Returns tweet IDs (most recent first).
pub fn search(conn: &Connection, query: &str, limit: usize) -> Result<Vec<String>, CacheError> {
    let mut stmt = conn.prepare("SELECT id FROM tweets_fts WHERE tweets_fts MATCH ?1 LIMIT ?2")?;
    let ids = stmt
        .query_map(params![query, limit as i64], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ids)
}

/// Get one cached tweet's raw row (id, author, text, created_at).
pub fn get(
    conn: &Connection,
    id: &str,
) -> Result<Option<(String, String, String, String)>, CacheError> {
    let mut stmt =
        conn.prepare("SELECT id, author_screen_name, text, created_at FROM tweets WHERE id = ?1")?;
    let mut rows = stmt.query_map(params![id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
        ))
    })?;
    match rows.next() {
        None => Ok(None),
        Some(r) => Ok(Some(r?)),
    }
}

/// Cursor state per feed key.
pub fn get_cursor(conn: &Connection, feed_key: &str) -> Result<Option<String>, CacheError> {
    let mut stmt = conn.prepare("SELECT cursor FROM sync_state WHERE feed_key = ?1")?;
    let mut rows = stmt.query_map(params![feed_key], |row| row.get::<_, Option<String>>(0))?;
    match rows.next() {
        None => Ok(None),
        Some(r) => Ok(r?),
    }
}

pub fn set_cursor(
    conn: &Connection,
    feed_key: &str,
    cursor: Option<&str>,
) -> Result<(), CacheError> {
    conn.execute(
        "INSERT INTO sync_state (feed_key, cursor, updated_at_secs) VALUES (?1, ?2, ?3)
         ON CONFLICT(feed_key) DO UPDATE SET cursor=excluded.cursor, updated_at_secs=excluded.updated_at_secs",
        params![feed_key, cursor, now_secs() as i64],
    )?;
    Ok(())
}

/// Watchlist: handles to keep an eye on.
pub fn watch_add(conn: &Connection, screen_name: &str) -> Result<(), CacheError> {
    conn.execute(
        "INSERT INTO watchlist (screen_name, added_at_secs) VALUES (?1, ?2)
         ON CONFLICT(screen_name) DO NOTHING",
        params![screen_name.to_lowercase(), now_secs() as i64],
    )?;
    Ok(())
}

pub fn watch_remove(conn: &Connection, screen_name: &str) -> Result<bool, CacheError> {
    let n = conn.execute(
        "DELETE FROM watchlist WHERE screen_name = ?1",
        params![screen_name.to_lowercase()],
    )?;
    Ok(n > 0)
}

pub fn watch_list(conn: &Connection) -> Result<Vec<String>, CacheError> {
    let mut stmt = conn.prepare("SELECT screen_name FROM watchlist ORDER BY screen_name")?;
    let out = stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(out)
}

/// Health summary for `doctor --cache` (bead 3.7.2 consumer).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct CacheHealth {
    pub reachable: bool,
    pub tweet_count: u64,
    pub media_count: u64,
    pub watch_count: u64,
    pub wal_mode: bool,
}

pub fn health(conn: &Connection) -> Result<CacheHealth, CacheError> {
    let tweet_count: u64 = conn.query_row("SELECT COUNT(*) FROM tweets", [], |r| r.get(0))?;
    let media_count: u64 = conn.query_row("SELECT COUNT(*) FROM media", [], |r| r.get(0))?;
    let watch_count: u64 = conn.query_row("SELECT COUNT(*) FROM watchlist", [], |r| r.get(0))?;
    let mode: String = conn.query_row("PRAGMA journal_mode", [], |r| r.get(0))?;
    Ok(CacheHealth {
        reachable: true,
        tweet_count,
        media_count,
        watch_count,
        wal_mode: mode.eq_ignore_ascii_case("wal"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        conn
    }

    fn tweet(id: &str, text: &str) -> twr_model::Tweet {
        twr_model::Tweet {
            id: id.into(),
            text: text.into(),
            author: twr_model::Author {
                id: "u".into(),
                name: "A".into(),
                screen_name: "a".into(),
                profile_image_url: String::new(),
                verified: false,
            },
            metrics: Default::default(),
            created_at: "t".into(),
            media: vec![twr_model::TweetMedia {
                media_type: "photo".into(),
                url: format!("https://img/{id}.jpg"),
                width: None,
                height: None,
            }],
            urls: vec![],
            is_retweet: false,
            lang: "en".into(),
            retweeted_by: None,
            quoted_tweet: None,
            score: None,
            article_title: None,
            article_text: None,
            is_subscriber_only: false,
            is_promoted: false,
        }
    }

    #[test]
    fn upsert_never_destroys_media_fk() {
        // The REPLACE gotcha: re-syncing a tweet must keep its media rows.
        let conn = mem();
        upsert_tweet(&conn, &tweet("1", "hello rust world")).unwrap();
        upsert_tweet(&conn, &tweet("1", "hello rust world edited")).unwrap();
        let media: u64 = conn
            .query_row("SELECT COUNT(*) FROM media WHERE tweet_id='1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(media, 1);
        let (.., text, _): (String, String, String, String) = get(&conn, "1").unwrap().unwrap();
        assert!(text.contains("edited"));
    }

    #[test]
    fn fts_search_finds_cached_text() {
        let conn = mem();
        upsert_tweet(&conn, &tweet("1", "hello rust world")).unwrap();
        upsert_tweet(&conn, &tweet("2", "unrelated content here")).unwrap();
        let hits = search(&conn, "rust", 10).unwrap();
        assert_eq!(hits, vec!["1".to_string()]);
    }

    #[test]
    fn cursors_and_watchlist() {
        let conn = mem();
        assert!(get_cursor(&conn, "feed:home").unwrap().is_none());
        set_cursor(&conn, "feed:home", Some("c1")).unwrap();
        assert_eq!(get_cursor(&conn, "feed:home").unwrap(), Some("c1".into()));
        watch_add(&conn, "Ada").unwrap();
        watch_add(&conn, "ada").unwrap();
        assert_eq!(watch_list(&conn).unwrap(), vec!["ada".to_string()]);
        assert!(watch_remove(&conn, "ADA").unwrap());
        assert!(watch_list(&conn).unwrap().is_empty());
    }

    #[test]
    fn health_reports_counts() {
        let conn = mem();
        upsert_tweet(&conn, &tweet("1", "hi")).unwrap();
        let h = health(&conn).unwrap();
        assert!(h.reachable && h.tweet_count == 1 && h.media_count == 1);
    }
}
