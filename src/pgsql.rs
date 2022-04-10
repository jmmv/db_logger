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

use crate::logger::{
    LogEntry, LOG_ENTRY_MAX_FILENAME_LENGTH, LOG_ENTRY_MAX_HOSTNAME_LENGTH,
    LOG_ENTRY_MAX_MESSAGE_LENGTH, LOG_ENTRY_MAX_MODULE_LENGTH,
};
use crate::{truncate_option_str, Connection, Db, Result};
use futures::TryStreamExt;
use sqlx::postgres::{PgConnectOptions, PgPool};
use sqlx::Row;
use std::env;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use time::OffsetDateTime;

/// Schema to use to initialize the test database.
const SCHEMA: &str = include_str!("../schemas/pgsql.sql");

/// Options to establish a connection to a PostgreSQL database.
#[derive(Default)]
#[cfg_attr(test, derive(PartialEq))]
pub struct ConnectionOptions {
    /// Host to connect to.
    pub host: String,

    /// Port to connect to (typically 5432).
    pub port: u16,

    /// Database name to connect to.
    pub database: String,

    /// Username to establish the connection with.
    pub username: String,

    /// Password to establish the connection with.
    pub password: String,
}

#[cfg(test)]
impl std::fmt::Debug for ConnectionOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectionOptions")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("database", &self.database)
            .field("username", &self.username)
            .field("password", &"scrubbed".to_owned())
            .finish()
    }
}

impl ConnectionOptions {
    /// Initializes a set of options from environment variables whose name is prefixed with the
    /// given `prefix`.
    ///
    /// This will use variables such as `<prefix>_HOST`, `<prefix>_PORT`, `<prefix>_DATABASE`,
    /// `<prefix>_USERNAME` and `<prefix>_PASSWORD`.
    pub fn from_env(prefix: &str) -> Result<ConnectionOptions> {
        fn get_required_var(prefix: &str, suffix: &str) -> Result<String> {
            let name = format!("{}_{}", prefix, suffix);
            match env::var(&name) {
                Ok(value) => Ok(value),
                Err(env::VarError::NotPresent) => {
                    Err(format!("Required environment variable {} not present", name))
                }
                Err(env::VarError::NotUnicode(_)) => {
                    Err(format!("Invalid value in environment variable {}", name))
                }
            }
        }
        Ok(ConnectionOptions {
            host: get_required_var(prefix, "HOST")?,
            port: get_required_var(prefix, "PORT")?
                .parse::<u16>()
                .map_err(|e| format!("Invalid port number: {}", e))?,
            database: get_required_var(prefix, "DATABASE")?,
            username: get_required_var(prefix, "USERNAME")?,
            password: get_required_var(prefix, "PASSWORD")?,
        })
    }
}

/// Factory to connect to a PostgreSQL database.
pub fn connect_lazy(opts: ConnectionOptions) -> Connection {
    Connection(Arc::from(PostgresDb::connect_lazy(opts)))
}

/// Factory to connect to and initialize a PostgreSQL test database.
pub async fn setup_test(opts: ConnectionOptions) -> Connection {
    let db = PostgresTestDb::setup_test(opts).await;
    Connection(Arc::from(db))
}

/// Converts an `u32` from the in-memory model to an `i16` suitable for storage.
fn u32_to_i16(field: &'static str, unsigned: u32) -> Result<i16> {
    if unsigned > i16::MAX as u32 {
        return Err(format!("{} ({}) is too large for i16", field, unsigned));
    }
    Ok(unsigned as i16)
}

/// A database instance backed by a PostgreSQL database.
#[derive(Clone)]
struct PostgresDb {
    pool: PgPool,
    suffix: Option<u32>,
    log_sequence: Arc<AtomicU32>,
}

impl PostgresDb {
    /// Creates a new connection based on the given options.
    fn connect_lazy(opts: ConnectionOptions) -> Self {
        let options = PgConnectOptions::new()
            .host(&opts.host)
            .port(opts.port)
            .database(&opts.database)
            .username(&opts.username)
            .password(&opts.password);

        Self {
            pool: PgPool::connect_lazy_with(options),
            suffix: None,
            log_sequence: Arc::from(AtomicU32::new(0)),
        }
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
    async fn get_log_entries(&self) -> Result<Vec<String>> {
        let query_str = self.patch_query("SELECT * FROM logs ORDER BY timestamp, sequence");
        let mut rows = sqlx::query(&query_str).fetch(&self.pool);
        let mut entries = vec![];
        while let Some(row) = rows.try_next().await.map_err(|e| e.to_string())? {
            let timestamp: OffsetDateTime = row.try_get("timestamp").map_err(|e| e.to_string())?;
            let hostname: String = row.try_get("hostname").map_err(|e| e.to_string())?;
            let level: i16 = row.try_get("level").map_err(|e| e.to_string())?;
            let module: Option<String> = row.try_get("module").map_err(|e| e.to_string())?;
            let filename: Option<String> = row.try_get("filename").map_err(|e| e.to_string())?;
            let line: Option<i16> = row.try_get("line").map_err(|e| e.to_string())?;
            let message: String = row.try_get("message").map_err(|e| e.to_string())?;

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

    async fn put_log_entries(&self, entries: Vec<LogEntry<'_, '_>>) -> Result<()> {
        let nentries = entries.len();
        if nentries == 0 {
            return Ok(());
        }

        if nentries > std::u32::MAX as usize {
            return Err(format!(
                "Cannot insert {} log entries at once; max is {}",
                nentries,
                std::u32::MAX
            ));
        }
        let mut sequence = self.log_sequence.fetch_add(nentries as u32, Ordering::SeqCst);

        let mut query_str = self.patch_query(
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

        let done = query.execute(&self.pool).await.map_err(|e| e.to_string())?;
        if done.rows_affected() as usize != nentries {
            return Err(format!(
                "Log entries insertion created {} rows but expected {}",
                done.rows_affected(),
                nentries
            ));
        }
        Ok(())
    }
}

/// A wrapper over `PostgresDb` to initialize and clean up a test database instance.
///
/// Instances of this object *must* be held on a non-async context without any async runtime
/// because `drop` needs to enter a new runtime to clean up the database.
#[derive(Clone)]
struct PostgresTestDb(PostgresDb);

impl PostgresTestDb {
    /// Creates a new connection to the test database and initializes it.
    ///
    /// The caller must arrange to call `teardown_test` on its own as it is appropriate.  We do not
    /// do this on `drop` due to the difficulties in handling this properly, because our code must
    /// be async but the `Drop` trait is not.
    ///
    /// As this is only for testing, any errors result in a panic.
    async fn setup_test(opts: ConnectionOptions) -> Self {
        let mut db = PostgresDb::connect_lazy(opts);

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
            sqlx::query(query_str).execute(&mut tx).await.unwrap();
        }
        tx.commit().await.unwrap();

        PostgresTestDb(db)
    }

    /// Deletes the state created by `setup_test` and shuts the pool down.
    ///
    /// As this is only for testing, any errors result in a panic.  Attempting to use the database
    /// after this has been called has undefined behavior.
    async fn teardown_test(&self) {
        let suffix = self.0.suffix.expect("This should only be called from tests");

        // Do not use patch_query here: we must make sure the fake names cannot possibly match the
        // values in production, and the extra `_` characters before the `{}` placeholders ensure
        // that this is true.
        let mut tx = self.0.pool.begin().await.unwrap();
        for query_str in &[
            format!("DROP INDEX logs_{}_by_timestamp", suffix),
            format!("DROP TABLE logs_{}", suffix),
        ] {
            sqlx::query(query_str).execute(&mut tx).await.unwrap();
        }
        tx.commit().await.unwrap();

        self.0.pool.close().await;
    }
}

impl Drop for PostgresTestDb {
    fn drop(&mut self) {
        #[tokio::main]
        async fn cleanup(context: &mut PostgresTestDb) {
            context.teardown_test().await;
        }
        cleanup(self)
    }
}

#[async_trait::async_trait]
impl Db for PostgresTestDb {
    async fn get_log_entries(&self) -> Result<Vec<String>> {
        self.0.get_log_entries().await
    }

    async fn put_log_entries(&self, entries: Vec<LogEntry<'_, '_>>) -> Result<()> {
        self.0.put_log_entries(entries).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutils;

    #[test]
    fn test_connectionoptions_from_env_ok() {
        let prefix = format!("TEST_{}", rand::random::<u32>());
        env::set_var(format!("{}_HOST", prefix), "the-host");
        env::set_var(format!("{}_PORT", prefix), "1234");
        env::set_var(format!("{}_DATABASE", prefix), "the-database");
        env::set_var(format!("{}_USERNAME", prefix), "the-username");
        env::set_var(format!("{}_PASSWORD", prefix), "the-password");
        let opts = ConnectionOptions::from_env(&prefix).unwrap();
        assert_eq!(
            ConnectionOptions {
                host: "the-host".to_owned(),
                port: 1234,
                database: "the-database".to_owned(),
                username: "the-username".to_owned(),
                password: "the-password".to_owned(),
            },
            opts
        );
    }

    /// Runs a test to validate that `ConnectionOptions::from_env` fails when the `missing`
    /// environment variable is not set.
    fn do_connectionoptions_from_env_missing_test(missing: &str) {
        let prefix = format!("TEST_{}", rand::random::<u32>());
        if missing != "HOST" {
            env::set_var(format!("{}_HOST", prefix), "host");
        }
        if missing != "PORT" {
            env::set_var(format!("{}_PORT", prefix), "5432");
        }
        if missing != "DATABASE" {
            env::set_var(format!("{}_DATABASE", prefix), "database");
        }
        if missing != "USERNAME" {
            env::set_var(format!("{}_USERNAME", prefix), "username");
        }
        if missing != "PASSWORD" {
            env::set_var(format!("{}_PASSWORD", prefix), "password");
        }
        match ConnectionOptions::from_env(&prefix) {
            Ok(_) => panic!("Should have failed"),
            Err(e) => assert!(e.contains(&format!("{}_{} not present", prefix, missing))),
        }
    }

    #[test]
    fn test_connectionoptions_from_env_missing_host() {
        do_connectionoptions_from_env_missing_test("HOST");
    }

    #[test]
    fn test_connectionoptions_from_env_missing_port() {
        do_connectionoptions_from_env_missing_test("PORT");
    }

    #[test]
    fn test_connectionoptions_from_env_missing_database() {
        do_connectionoptions_from_env_missing_test("DATABASE");
    }

    #[test]
    fn test_connectionoptions_from_env_missing_username() {
        do_connectionoptions_from_env_missing_test("USERNAME");
    }

    #[test]
    fn test_connectionoptions_from_env_missing_password() {
        do_connectionoptions_from_env_missing_test("PASSWORD");
    }

    #[test]
    fn test_connectionoptions_from_env_invalid_port() {
        let prefix = format!("TEST_{}", rand::random::<u32>());
        env::set_var(format!("{}_HOST", prefix), "host");
        env::set_var(format!("{}_PORT", prefix), "abc");
        env::set_var(format!("{}_DATABASE", prefix), "database");
        env::set_var(format!("{}_USERNAME", prefix), "username");
        env::set_var(format!("{}_PASSWORD", prefix), "password");
        match ConnectionOptions::from_env(&prefix) {
            Ok(_) => panic!("Should have failed"),
            Err(e) => assert!(e.contains("Invalid port number")),
        }
    }

    /// Test context to allow automatic cleanup of the test database.
    struct PostgresTestContext {
        db: PostgresTestDb,
    }

    #[async_trait::async_trait]
    impl testutils::TestContext for PostgresTestContext {
        fn db(&self) -> &(dyn Db + Send + Sync) {
            &self.db
        }
    }

    /// Initializes the test environment by creating unique tables in the test database.
    fn setup() -> Box<dyn testutils::TestContext> {
        let _can_fail = env_logger::builder().is_test(true).try_init();

        #[tokio::main]
        async fn prepare() -> PostgresTestDb {
            PostgresTestDb::setup_test(ConnectionOptions::from_env("PGSQL_TEST").unwrap()).await
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
