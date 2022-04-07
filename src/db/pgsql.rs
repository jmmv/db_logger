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

//! Implementation of the database abstraction using PostgreSQL.

use crate::db::*;
use futures::TryStreamExt;
use sqlx::postgres::{PgConnectOptions, PgDatabaseError, PgPool, Postgres};
use sqlx::{Row, Transaction};
use std::env;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use time::OffsetDateTime;

/// Schema to use to initialize the test database.
const SCHEMA: &str = include_str!("pgsql_schema.sql");

// Maximum sizes of the corresponding fields in the schema.
pub(crate) const LOG_ENTRY_MAX_HOSTNAME_LENGTH: usize = 64;
pub(crate) const LOG_ENTRY_MAX_MODULE_LENGTH: usize = 64;
pub(crate) const LOG_ENTRY_MAX_FILENAME_LENGTH: usize = 256;
pub(crate) const LOG_ENTRY_MAX_MESSAGE_LENGTH: usize = 1024;

/// Takes a raw SQLx error `e` and converts it to our generic error type.
fn map_sqlx_error(e: sqlx::Error) -> DbError {
    match e {
        sqlx::Error::RowNotFound => DbError::NotFound,
        sqlx::Error::Database(e) => match e.downcast_ref::<PgDatabaseError>().code() {
            "23503" /* foreign_key_violation */ => DbError::NotFound,
            "23505" /* unique_violation */ => DbError::AlreadyExists,
            number => DbError::BackendError(format!("pgsql error {}: {}", number, e)),
        },
        e => DbError::BackendError(e.to_string()),
    }
}

/// A database instance backed by a PostgreSQL database.
#[derive(Clone)]
pub struct PostgresDb {
    pool: PgPool,
    suffix: Option<u32>,
    log_sequence: Arc<AtomicU32>,
}

impl PostgresDb {
    /// Creates a new connection based on environment variables.
    pub fn connect_lazy(
        host: &str,
        port: u16,
        database: &str,
        username: &str,
        password: &str,
    ) -> Self {
        let options = PgConnectOptions::new()
            .host(host)
            .port(port)
            .database(database)
            .username(username)
            .password(password);

        Self {
            pool: PgPool::connect_lazy_with(options),
            suffix: None,
            log_sequence: Arc::from(AtomicU32::new(0)),
        }
    }

    /// Creates a new connection to the test database and initializes it.
    ///
    /// The caller must arrange to call `teardown_test` on its own as it is appropriate.  We do not
    /// do this on `drop` due to the difficulties in handling this properly, because our code must
    /// be async but the `Drop` trait is not.
    ///
    /// As this is only for testing, any errors result in a panic.
    pub async fn setup_test() -> Self {
        let mut db = PostgresDb::connect_lazy(
            &env::var("PGSQL_TEST_HOST").unwrap(),
            env::var("PGSQL_TEST_PORT").unwrap().parse::<u16>().unwrap(),
            &env::var("PGSQL_TEST_DATABASE").unwrap(),
            &env::var("PGSQL_TEST_USERNAME").unwrap(),
            &env::var("PGSQL_TEST_PASSWORD").unwrap(),
        );

        // Strip out comments from the schema so that we can safely separate the statements by
        // looking for semicolons.
        let schema = regex::RegexBuilder::new("--.*$")
            .multi_line(true)
            .build()
            .unwrap()
            .replace_all(SCHEMA, "");

        db.suffix = Some(rand::random());
        let schema = db.patch_query(&schema);

        let mut tx = db.pool.begin().await.unwrap();
        for query_str in schema.split(';') {
            sqlx::query(query_str).execute(&mut tx).await.map_err(map_sqlx_error).unwrap();
        }
        tx.commit().await.unwrap();

        db
    }

    /// Deletes the state created by `setup_test` and shuts the pool down.
    ///
    /// As this is only for testing, any errors result in a panic.  Attempting to use the database
    /// after this has been called has undefined behavior.
    pub async fn teardown_test(&self) {
        let suffix = self.suffix.expect("This should only be called from tests");

        // Do not use patch_query here: we must make sure the fake names cannot possibly match the
        // values in production, and the extra `_` characters before the `{}` placeholders ensure
        // that this is true.
        let mut tx = self.pool.begin().await.unwrap();
        for query_str in &[
            format!("DROP INDEX logs_{}_by_timestamp", suffix),
            format!("DROP TABLE logs_{}", suffix),
        ] {
            sqlx::query(query_str).execute(&mut tx).await.map_err(map_sqlx_error).unwrap();
        }
        tx.commit().await.unwrap();

        self.pool.close().await;
    }

    /// Given a `query`, replaces table and index identifiers to account for the `suffix` rename
    /// used during tests.
    fn patch_query(&self, query: &str) -> String {
        match self.suffix {
            None => query.to_owned(),
            Some(suffix) => query.replace(" logs", &format!(" logs_{}", suffix)),
        }
    }
}

#[async_trait::async_trait]
impl Db for PostgresDb {
    async fn begin<'a>(&'a self) -> DbResult<Box<dyn Tx<'a> + 'a + Send>> {
        loop {
            match self.pool.begin().await.map_err(map_sqlx_error) {
                Ok(tx) => {
                    return Ok(Box::from(PostgresTx {
                        db: self,
                        tx,
                        log_sequence: self.log_sequence.clone(),
                    }))
                }
                Err(DbError::Unavailable) => {
                    log::warn!("Database is not available; retrying");
                    // TODO(jmmv): Implement retries and backoff.  Or maybe delegate to the client
                    // altogether, as the client should do this too.
                }
                Err(e) => return Err(e),
            }
        }
    }
}

/// A transaction backed by a PostgreSQL database.
pub(crate) struct PostgresTx<'a> {
    db: &'a PostgresDb,
    tx: Transaction<'a, Postgres>,
    log_sequence: Arc<AtomicU32>,
}

#[async_trait::async_trait]
impl<'a> Tx<'a> for PostgresTx<'a> {
    async fn commit(self: Box<Self>) -> DbResult<()> {
        self.tx.commit().await.map_err(map_sqlx_error)?;
        Ok(())
    }

    async fn get_log_entries(&mut self) -> DbResult<Vec<String>> {
        let query_str = self.db.patch_query("SELECT * FROM logs ORDER BY timestamp, sequence");
        let mut rows = sqlx::query(&query_str).fetch(&mut self.tx);
        let mut entries = vec![];
        while let Some(row) = rows.try_next().await.map_err(map_sqlx_error)? {
            let timestamp: OffsetDateTime = row.try_get("timestamp").map_err(map_sqlx_error)?;
            let hostname: String = row.try_get("hostname").map_err(map_sqlx_error)?;
            let level: i16 = row.try_get("level").map_err(map_sqlx_error)?;
            let module: Option<String> = row.try_get("module").map_err(map_sqlx_error)?;
            let filename: Option<String> = row.try_get("filename").map_err(map_sqlx_error)?;
            let line: Option<i16> = row.try_get("line").map_err(map_sqlx_error)?;
            let message: String = row.try_get("message").map_err(map_sqlx_error)?;

            entries.push(format!(
                "{}.{} {} {} {} {}:{} {}",
                timestamp.unix_timestamp(),
                timestamp.unix_timestamp_nanos() % 1000000000,
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

    async fn put_log_entries(&mut self, entries: Vec<LogEntry<'_, '_>>) -> DbResult<()> {
        let nentries = entries.len();
        if nentries == 0 {
            return Ok(());
        }

        if nentries > std::u32::MAX as usize {
            return Err(DbError::BackendError(format!(
                "Cannot insert {} log entries at once; max is {}",
                nentries,
                std::u32::MAX
            )));
        }
        let mut sequence = self.log_sequence.fetch_add(nentries as u32, Ordering::SeqCst);

        let mut query_str = self.db.patch_query(
            "INSERT INTO logs
                (timestamp, sequence, hostname, level, module, filename, line, message)
            VALUES ",
        );
        const NPARAMS: usize = 8;

        let mut param: usize = 1;
        for _ in 0..nentries {
            if param > 1 {
                query_str.push(',');
            }
            query_str.push('(');
            for i in 1..NPARAMS + 1 {
                if i == 1 {
                    query_str += &format!("${}", param);
                } else {
                    query_str += &format!(", ${}", param);
                }
                param += 1;
            }
            query_str.push(')');
        }

        let mut query = sqlx::query(&query_str);
        for mut entry in entries.into_iter() {
            let module = truncate_option_str(entry.module, LOG_ENTRY_MAX_MODULE_LENGTH);
            let filename = truncate_option_str(entry.filename, LOG_ENTRY_MAX_FILENAME_LENGTH);
            entry.hostname.truncate(LOG_ENTRY_MAX_HOSTNAME_LENGTH);
            entry.message.truncate(LOG_ENTRY_MAX_MESSAGE_LENGTH);

            let line = match entry.line {
                Some(n) => Some(u32_to_i16("line", n)?),
                None => None,
            };

            query = query
                .bind(entry.timestamp)
                .bind(sequence)
                .bind(entry.hostname)
                .bind(entry.level as i16)
                .bind(module)
                .bind(filename)
                .bind(line)
                .bind(entry.message);

            sequence += 1;
        }

        let done = query.execute(&mut self.tx).await.map_err(map_sqlx_error)?;
        if done.rows_affected() as usize != nentries {
            return Err(DbError::BackendError(format!(
                "Log entries insertion created {} rows but expected {}",
                done.rows_affected(),
                nentries
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::testutils;

    /// Test context to allow automatic cleanup of the test database.
    struct PostgresTestContext {
        db: PostgresDb,
    }

    #[async_trait::async_trait]
    impl testutils::TestContext for PostgresTestContext {
        fn db(&self) -> Box<dyn Db + Send + Sync> {
            Box::from(self.db.clone())
        }

        async fn tx<'a>(&'a mut self) -> Box<dyn Tx<'a> + 'a + Send> {
            self.db.begin().await.unwrap()
        }
    }

    impl Drop for PostgresTestContext {
        fn drop(&mut self) {
            #[tokio::main]
            async fn cleanup(db: &mut PostgresDb) {
                db.teardown_test().await;
            }
            cleanup(&mut self.db);
        }
    }

    /// Initializes the test environment by creating unique tables in the test database.
    fn setup() -> Box<dyn testutils::TestContext> {
        let _can_fail = env_logger::builder().is_test(true).try_init();

        #[tokio::main]
        async fn prepare() -> PostgresDb {
            PostgresDb::setup_test().await
        }
        Box::from(PostgresTestContext { db: prepare() })
    }

    #[test]
    #[ignore = "Requires environment configuration and is expensive"]
    fn test_postgresdb_log_entries_none() {
        testutils::test_log_entries_none(setup());
    }

    #[test]
    #[ignore = "Requires environment configuration and is expensive"]
    fn test_postgresdb_log_entries_individual() {
        testutils::test_log_entries_individual(setup());
    }

    #[test]
    #[ignore = "Requires environment configuration and is expensive"]
    fn test_postgresdb_log_entries_combined() {
        testutils::test_log_entries_combined(setup());
    }

    #[test]
    #[ignore = "Requires environment configuration and is expensive"]
    fn test_postgresdb_log_entries_long_strings() {
        testutils::test_log_entries_long_strings(setup());
    }
}
