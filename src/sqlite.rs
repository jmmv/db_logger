// db_logger
// Copyright 2022 Julio Merino
//
// Licensed under the Apache License, Version 2.0 (the "License"); you may not
// use this file except in compliance with the License.  You may obtain a copy
// of the License at:
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS, WITHOUT
// WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.  See the
// License for the specific language governing permissions and limitations
// under the License.

//! Implementation of the database abstraction using SQLite.

use crate::logger::{
    LogEntry, LOG_ENTRY_MAX_FILENAME_LENGTH, LOG_ENTRY_MAX_HOSTNAME_LENGTH,
    LOG_ENTRY_MAX_MESSAGE_LENGTH, LOG_ENTRY_MAX_MODULE_LENGTH,
};
use crate::{truncate_option_str, Connection, Db, Result};
use futures::TryStreamExt;
use sqlx::sqlite::SqlitePool;
use sqlx::Row;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use time::OffsetDateTime;
use tokio::sync::Semaphore;

/// Schema to use to initialize the test database.
const SCHEMA: &str = include_str!("../schemas/sqlite.sql");

/// Options to establish a connection to an SQLite database.
#[derive(Default)]
pub struct ConnectionOptions {
    /// URI of the database to connect to.
    pub uri: String,
}

/// Factory to connect to a SQLite database.
pub async fn connect(opts: ConnectionOptions) -> Result<Connection> {
    SqliteDb::connect(opts).await.map(|db| Connection(Arc::from(db)))
}

/// Converts an `u64` from the in-memory model to an `i64` suitable for storage.
fn u64_to_i64(field: &'static str, unsigned: u64) -> Result<i64> {
    if unsigned > i64::MAX as u64 {
        return Err(format!("{} ({}) is too large for i64", field, unsigned));
    }
    Ok(unsigned as i64)
}

/// Converts a timestamp into the seconds and nanoseconds pair needed by the database.
///
/// Nanoseconds are rounded to the next microsecond to emulate the behavior of the `pgsql`
/// implementation.
fn unpack_timestamp(ts: OffsetDateTime) -> (i64, i64) {
    let nanos = ts.unix_timestamp_nanos();

    let nanos_only = nanos % 1000;
    let mut nanos = nanos / 1000 * 1000;
    if nanos_only > 0 {
        nanos += 1000;
    }

    let sec = (nanos / 1_000_000_000) as i64;
    let nsec = (nanos % 1_000_000_000) as i64;
    (sec, nsec)
}

/// A database instance backed by an SQLite database.
#[derive(Clone)]
struct SqliteDb {
    pool: SqlitePool,
    sem: Arc<Semaphore>,
    log_sequence: Arc<AtomicU64>,
}

impl SqliteDb {
    /// Creates a new connection based on environment variables and initializes its schema.
    async fn connect(opts: ConnectionOptions) -> Result<Self> {
        let pool = SqlitePool::connect(&opts.uri).await.map_err(|e| e.to_string())?;

        // Serialize all transactions onto the SQLite database to avoid busy errors that we cannot
        // easily deal with during tests.
        let sem = Arc::from(Semaphore::new(1));

        let log_sequence = Arc::from(AtomicU64::new(0));

        Ok(Self { pool, sem, log_sequence })
    }
}

#[async_trait::async_trait]
impl Db for SqliteDb {
    async fn create_schema(&self) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(|e| e.to_string())?;
        {
            let mut results = sqlx::query(SCHEMA).execute_many(&mut tx).await;
            while results.try_next().await.map_err(|e| e.to_string())?.is_some() {
                // Nothing to do.
            }
        }
        tx.commit().await.map_err(|e| e.to_string())
    }

    async fn get_log_entries(&self) -> Result<Vec<String>> {
        let _permit = self.sem.clone().acquire_owned().await.expect("Semaphore prematurely closed");

        let query_str = "SELECT * FROM logs ORDER BY timestamp_secs, timestamp_nsecs, sequence";
        let mut rows = sqlx::query(query_str).fetch(&self.pool);
        let mut entries = vec![];
        while let Some(row) = rows.try_next().await.map_err(|e| e.to_string())? {
            let timestamp_secs: i64 = row.try_get("timestamp_secs").map_err(|e| e.to_string())?;
            let timestamp_nsecs: i64 = row.try_get("timestamp_nsecs").map_err(|e| e.to_string())?;
            let hostname: String = row.try_get("hostname").map_err(|e| e.to_string())?;
            let level: i8 = row.try_get("level").map_err(|e| e.to_string())?;
            let module: Option<String> = row.try_get("module").map_err(|e| e.to_string())?;
            let filename: Option<String> = row.try_get("filename").map_err(|e| e.to_string())?;
            let line: Option<i16> = row.try_get("line").map_err(|e| e.to_string())?;
            let message: String = row.try_get("message").map_err(|e| e.to_string())?;

            entries.push(format!(
                "{}.{} {} {} {} {}:{} {}",
                timestamp_secs,
                timestamp_nsecs,
                hostname,
                level,
                module.as_deref().unwrap_or("NO-MODULE"),
                filename.as_deref().unwrap_or("NO-FILENAME"),
                line.unwrap_or(-1),
                message
            ))
        }
        Ok(entries)
    }

    async fn put_log_entries(&self, entries: Vec<LogEntry<'_, '_>>) -> Result<()> {
        let _permit = self.sem.clone().acquire_owned().await.expect("Semaphore prematurely closed");

        let mut sequence = self.log_sequence.fetch_add(entries.len() as u64, Ordering::SeqCst);

        let query_str = "
            INSERT INTO logs
                (timestamp_secs, timestamp_nsecs, sequence, hostname,
                    level, module, filename, line, message)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)";

        // Using a transaction is critical for good SQLite write performance.
        let mut tx = self.pool.begin().await.map_err(|e| e.to_string())?;

        // This implementation is not very efficient but it is straightforward, which is
        // intentional given that this is only used for testing.
        for mut entry in entries.into_iter() {
            // This is not necessary but truncate the contents to match the Azure SQL
            // implementation.
            let module = truncate_option_str(entry.module, LOG_ENTRY_MAX_MODULE_LENGTH);
            let filename = truncate_option_str(entry.filename, LOG_ENTRY_MAX_FILENAME_LENGTH);
            entry.hostname.truncate(LOG_ENTRY_MAX_HOSTNAME_LENGTH);
            entry.message.truncate(LOG_ENTRY_MAX_MESSAGE_LENGTH);

            let (timestamp_secs, timestamp_nsecs) = unpack_timestamp(entry.timestamp);

            let done = sqlx::query(query_str)
                .bind(timestamp_secs)
                .bind(timestamp_nsecs)
                .bind(u64_to_i64("sequence", sequence)?)
                .bind(entry.hostname)
                .bind(entry.level as u8)
                .bind(module)
                .bind(filename)
                .bind(entry.line)
                .bind(entry.message)
                .execute(&mut tx)
                .await
                .map_err(|e| e.to_string())?;
            if done.rows_affected() != 1 {
                return Err("Insertion didn't create one row".to_owned());
            }

            sequence += 1;
        }

        tx.commit().await.map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutils;

    /// Test context to allow automatic cleanup of the test database.
    struct SqliteTestContext {
        db: SqliteDb,
    }

    #[async_trait::async_trait]
    impl testutils::TestContext for SqliteTestContext {
        fn db(&self) -> &(dyn Db + Send + Sync) {
            &self.db
        }
    }

    /// Initializes the test database.
    fn setup() -> Box<dyn testutils::TestContext> {
        let _can_fail = env_logger::builder().is_test(true).try_init();

        #[tokio::main]
        async fn prepare() -> SqliteDb {
            let db =
                SqliteDb::connect(ConnectionOptions { uri: ":memory:".to_owned() }).await.unwrap();
            db.create_schema().await.unwrap();
            db
        }
        Box::from(SqliteTestContext { db: prepare() })
    }

    #[test]
    fn test_sqlitedb_log_entries_none() {
        testutils::test_log_entries_none(setup());
    }

    #[test]
    fn test_sqlitedb_log_entries_individual() {
        testutils::test_log_entries_individual(setup());
    }

    #[test]
    fn test_sqlitedb_log_entries_combined() {
        testutils::test_log_entries_combined(setup());
    }

    #[test]
    fn test_sqlitedb_log_entries_long_strings() {
        testutils::test_log_entries_long_strings(setup());
    }
}
