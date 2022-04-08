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

//! Database abstraction in terms of the operations needed by the server.

use crate::logger::LogEntry;

pub mod pgsql;
#[cfg(test)]
pub(crate) mod sqlite;
#[cfg(test)]
mod testutils;

/// Database errors.  Any unexpected errors that come from the database are classified as
/// `BackendError`, but errors we know about have more specific types.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum DbError {
    #[error("Already exists")]
    AlreadyExists,

    #[error("Database error: {0}")]
    BackendError(String),

    #[error("Data integrity error: {0}")]
    DataIntegrityError(String),

    #[error("Invalid request: {0}")]
    InvalidRequest(String),

    #[error("Entity not found")]
    NotFound,

    #[error("Service unavailable; back off and retry")]
    Unavailable,
}

/// Result type for this module.
pub(crate) type DbResult<T> = Result<T, DbError>;

/// Converts an `u32` from the in-memory model to an `i16` suitable for storage.
fn u32_to_i16(field: &'static str, unsigned: u32) -> DbResult<i16> {
    if unsigned > i16::MAX as u32 {
        return Err(DbError::BackendError(format!(
            "{} ({}) is too large for i16",
            field, unsigned
        )));
    }
    Ok(unsigned as i16)
}

/// Converts an `u64` from the in-memory model to an `i64` suitable for storage.
#[cfg(test)]
fn u64_to_i64(field: &'static str, unsigned: u64) -> DbResult<i64> {
    if unsigned > i64::MAX as u64 {
        return Err(DbError::BackendError(format!(
            "{} ({}) is too large for i64",
            field, unsigned
        )));
    }
    Ok(unsigned as i64)
}

/// Fits the string in `input` within the specified `max_len`.
fn truncate_option_str(input: Option<&str>, max_len: usize) -> Option<String> {
    match input {
        Some(s) => {
            let mut s = s.to_owned();
            s.truncate(max_len);
            Some(s)
        }
        None => None,
    }
}

/// Abstraction over the database connection.
#[async_trait::async_trait]
pub(crate) trait Db {
    /// Begins a transaction.
    async fn begin<'a>(&'a self) -> DbResult<Box<dyn Tx<'a> + 'a + Send>>;
}

/// A transaction with high-level operations that deal with our types.
#[async_trait::async_trait]
pub(crate) trait Tx<'a> {
    /// Commits the transaction.  The transaction is rolled back on drop unless this is called.
    async fn commit(self: Box<Self>) -> DbResult<()>;

    /// Returns the sorted list of all log entries in the database.
    ///
    /// Given that this is exposed for testing purposes only, this just returns a flat textual
    /// representation of the log entry and does not try to deserialize it as a `LogEntry`.  This
    /// is for simplicity given that a `LogEntry` keeps references to static strings and we cannot
    /// obtain those from the database.
    async fn get_log_entries(&mut self) -> DbResult<Vec<String>>;

    /// Appends a series of `entries` to the log.
    ///
    /// All entries are inserted at once into the database to avoid unnecessary round trips for each
    /// log entry, which happen to be very expensive for something as frequent as log messages.
    ///
    /// This takes a `Vec` instead of a slice for efficiency, as the writes may have to truncate the
    /// entries.
    async fn put_log_entries(&mut self, entries: Vec<LogEntry<'_, '_>>) -> DbResult<()>;
}
