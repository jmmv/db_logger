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

//! A database-backed logger for use with the [`log` crate][log-crate-url].

// Keep these in sync with other top-level files.
#![allow(clippy::await_holding_refcell_ref)]
#![warn(anonymous_parameters, bad_style, missing_docs)]
#![warn(unused, unused_extern_crates, unused_import_braces, unused_qualifications)]
#![warn(unsafe_code)]

use std::sync::Arc;

mod clocks;
pub(crate) mod logger;
use crate::logger::LogEntry;
pub mod pgsql;
pub use logger::{DbLogger, Handle};
#[cfg(test)]
pub(crate) mod sqlite;
#[cfg(test)]
mod testutils;

/// Opaque type representing a connection to the logging database.
#[derive(Clone)]
pub struct Connection(Arc<dyn Db + Send + Sync + 'static>);

/// Database errors.  Any unexpected errors that come from the database are classified as
/// `BackendError`, but errors we know about have more specific types.
#[allow(missing_docs)]
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
