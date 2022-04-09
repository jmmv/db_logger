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

//! Common tests for any database implementation.

use crate::logger::{
    LogEntry, LOG_ENTRY_MAX_FILENAME_LENGTH, LOG_ENTRY_MAX_HOSTNAME_LENGTH,
    LOG_ENTRY_MAX_MESSAGE_LENGTH, LOG_ENTRY_MAX_MODULE_LENGTH,
};
use crate::{Db, Tx};
use time::OffsetDateTime;

/// Context to parameterize the tests depending on the backing database.
///
/// Implementations of this trait can also implement `Drop` to perform cleanup operations at the
/// end of each test.
#[async_trait::async_trait]
pub(crate) trait TestContext {
    fn db(&self) -> Box<dyn Db + Send + Sync>;
    async fn tx<'a>(&'a mut self) -> Box<dyn Tx<'a> + 'a + Send>;
}

pub(crate) fn test_log_entries_none(mut context: Box<dyn TestContext>) {
    #[tokio::main]
    async fn run(context: &mut dyn TestContext) {
        let mut tx = context.tx().await;
        tx.put_log_entries(vec![]).await.unwrap();
        assert!(tx.get_log_entries().await.unwrap().is_empty());
    }
    run(context.as_mut());
}

pub(crate) fn test_log_entries_individual(mut context: Box<dyn TestContext>) {
    #[tokio::main]
    async fn run(context: &mut dyn TestContext) {
        let mut tx = context.tx().await;

        let entry1 = LogEntry {
            timestamp: OffsetDateTime::from_unix_timestamp_nanos(1_000_001_001),
            hostname: "fake-host1".to_owned(),
            level: log::Level::Error,
            module: None,
            filename: None,
            line: None,
            message: "Entry without optional fields".to_owned(),
        };
        tx.put_log_entries(vec![entry1]).await.unwrap();

        let entry2 = LogEntry {
            timestamp: OffsetDateTime::from_unix_timestamp_nanos(12_345_000_006_000),
            hostname: "fake-host2".to_owned(),
            level: log::Level::Info,
            module: Some("the-module"),
            filename: Some("the-file"),
            line: Some(42),
            message: "Entry with optional fields".to_owned(),
        };
        tx.put_log_entries(vec![entry2]).await.unwrap();

        let exp_entries = vec![
            "1.2000 fake-host1 1 NO-MODULE NO-FILENAME:-1 Entry without optional fields".to_owned(),
            "12345.6000 fake-host2 3 the-module the-file:42 Entry with optional fields".to_owned(),
        ];
        assert_eq!(exp_entries, tx.get_log_entries().await.unwrap());
    }
    run(context.as_mut());
}

pub(crate) fn test_log_entries_combined(mut context: Box<dyn TestContext>) {
    #[tokio::main]
    async fn run(context: &mut dyn TestContext) {
        let mut tx = context.tx().await;

        let entry1 = LogEntry {
            timestamp: OffsetDateTime::from_unix_timestamp_nanos(1_000_001_500),
            hostname: "fake-host1".to_owned(),
            level: log::Level::Error,
            module: None,
            filename: None,
            line: None,
            message: "Entry without optional fields".to_owned(),
        };

        let entry2 = LogEntry {
            timestamp: OffsetDateTime::from_unix_timestamp_nanos(12_345_000_006_999),
            hostname: "fake-host2".to_owned(),
            level: log::Level::Info,
            module: Some("the-module"),
            filename: Some("the-file"),
            line: Some(42),
            message: "Entry with optional fields".to_owned(),
        };

        tx.put_log_entries(vec![entry1, entry2]).await.unwrap();

        let exp_entries = vec![
            "1.2000 fake-host1 1 NO-MODULE NO-FILENAME:-1 Entry without optional fields".to_owned(),
            "12345.7000 fake-host2 3 the-module the-file:42 Entry with optional fields".to_owned(),
        ];
        assert_eq!(exp_entries, tx.get_log_entries().await.unwrap());
    }
    run(context.as_mut());
}
pub(crate) fn test_log_entries_long_strings(mut context: Box<dyn TestContext>) {
    #[tokio::main]
    async fn run(context: &mut dyn TestContext) {
        let mut tx = context.tx().await;

        let mut long_string = String::with_capacity(5000);
        for i in 0..long_string.capacity() {
            long_string.push((b'0' + ((i % 10) as u8)) as char);
        }

        let entry = LogEntry {
            timestamp: OffsetDateTime::from_unix_timestamp(0),
            hostname: long_string.to_owned(),
            level: log::Level::Trace,
            module: Some(&long_string),
            filename: Some(&long_string),
            line: None,
            message: long_string.to_owned(),
        };
        tx.put_log_entries(vec![entry]).await.unwrap();

        let truncated_hostname = &long_string[0..LOG_ENTRY_MAX_HOSTNAME_LENGTH];
        let truncated_module = &long_string[0..LOG_ENTRY_MAX_MODULE_LENGTH];
        let truncated_filename = &long_string[0..LOG_ENTRY_MAX_FILENAME_LENGTH];
        let truncated_message = &long_string[0..LOG_ENTRY_MAX_MESSAGE_LENGTH];

        let exp_entries = vec![format!(
            "0.0 {} 5 {} {}:-1 {}",
            truncated_hostname, truncated_module, truncated_filename, truncated_message
        )];
        assert_eq!(exp_entries, tx.get_log_entries().await.unwrap());
    }
    run(context.as_mut());
}
