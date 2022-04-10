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

//! Integration tests for the database logger, independent of the backing database.
//!
//! These tests intend to exercise the `DbLogger` against a real database.  This makes testing
//! difficult, but we need this level of integration testing to ensure that the logger works
//! properly and doesn't, for example, enter infinite loops due to recursion.
//!
//! Note also that the logger is, essentially, a global resource.  This means that we can only
//! initialize it once here, which in turn limits the granularity of our tests.  We can only have
//! a single `#[test]` to wrap the whole code in this file

use db_logger::{Connection, Handle};
use gethostname::gethostname;
use log::*;
use std::env;
use std::time::Duration;

/// Generates a log line to match the test format returned by `Db::get_log_entries` for a log
/// entry emitted from this module and processed by `make_deterministic`.
///
/// `level` is the numerical level of the entry; `line` is the line of code where the message was
/// raised, and `message` is the free-form text of the entry.
fn make_log_line(test_name: &str, level: u8, line: u32, message: &str) -> String {
    let hostname =
        gethostname().into_string().unwrap_or_else(|_e| String::from("invalid-hostname"));

    format!(
        "SSSS.uuuu {} {} {}::common tests/common/mod.rs:{} {}",
        hostname, level, test_name, line, message
    )
}

/// Takes the log lines returned by `PostgresDb::get_log_entries` and strips out non-determinism.
fn make_deterministic(lines: Vec<String>) -> Vec<String> {
    let mut new_lines = Vec::with_capacity(lines.len());
    let timestamp_re = regex::Regex::new("^[0-9]+.[0-9]+").unwrap();
    for line in lines {
        let line = timestamp_re.replace(&line, "SSSS.uuuu");
        new_lines.push(line.to_string());
    }
    new_lines
}

async fn test_all_levels(test_name: &str, handle: &Handle, exp_logs: &mut Vec<String>) {
    log::set_max_level(Level::Trace.to_level_filter());

    // Testing all levels is critical to ensure any potential log messages written by the database
    // connection itself don't cause us to loop infinitely.
    let base_line = line!();
    error!("An error message");
    warn!("A warning message");
    info!("An info message");
    debug!("A debug message");
    trace!("A trace message");
    log::logger().flush();

    exp_logs.push(make_log_line(test_name, 1, base_line + 1, "An error message"));
    exp_logs.push(make_log_line(test_name, 2, base_line + 2, "A warning message"));
    exp_logs.push(make_log_line(test_name, 3, base_line + 3, "An info message"));
    exp_logs.push(make_log_line(test_name, 4, base_line + 4, "A debug message"));
    exp_logs.push(make_log_line(test_name, 5, base_line + 5, "A trace message"));

    let entries = handle.get_log_entries().await.unwrap();
    assert_eq!(exp_logs, &make_deterministic(entries));
}

async fn test_level_filtering(test_name: &str, handle: &Handle, exp_logs: &mut Vec<String>) {
    log::set_max_level(Level::Warn.to_level_filter());

    // Testing all levels is critical to ensure any potential log messages written by the database
    // connection itself don't cause us to loop infinitely.
    let base_line = line!();
    error!("An error message");
    warn!("A warning message");
    info!("An info message");
    debug!("A debug message");
    trace!("A trace message");
    log::logger().flush();

    exp_logs.push(make_log_line(test_name, 1, base_line + 1, "An error message"));
    exp_logs.push(make_log_line(test_name, 2, base_line + 2, "A warning message"));

    let entries = handle.get_log_entries().await.unwrap();
    assert_eq!(exp_logs, &make_deterministic(entries));
}

async fn test_auto_flush(test_name: &str, handle: &Handle, exp_logs: &mut Vec<String>) {
    log::set_max_level(Level::Info.to_level_filter());

    let base_line = line!();
    info!("Another message");
    // Do not call flush here.  We should see the message show up eventually.

    exp_logs.push(make_log_line(test_name, 3, base_line + 1, "Another message"));

    let mut retries = 30;
    while retries > 0 {
        let entries = make_deterministic(handle.get_log_entries().await.unwrap());
        if exp_logs == &entries {
            break;
        } else if retries == 0 {
            assert_eq!(exp_logs, &entries);
            panic!("Previous assertion should have failed");
        }
        tokio::time::sleep(Duration::new(0, 100)).await;
        retries -= 1;
    }
}

async fn test_flood(test_name: &str, handle: &Handle, exp_logs: &mut Vec<String>) {
    log::set_max_level(Level::Info.to_level_filter());

    for i in 0..10240 {
        let base_line = line!();
        info!("Message with index {}", i);
        exp_logs.push(make_log_line(
            test_name,
            3,
            base_line + 1,
            &format!("Message with index {}", i),
        ));
    }
    log::logger().flush();

    let entries = handle.get_log_entries().await.unwrap();
    assert_eq!(exp_logs, &make_deterministic(entries));
}

/// Run all tests against an established `db` connection.
pub(crate) fn do_test_everything(test_name: &str, db: Connection) {
    #[tokio::main(flavor = "multi_thread", worker_threads = 2)]
    async fn run_tests(test_name: &str, db: Connection) {
        env::set_var("RUST_LOG", "trace");
        let handle = db_logger::init(db);

        let mut logs_accumulator = vec![];

        test_all_levels(test_name, &handle, &mut logs_accumulator).await;
        test_level_filtering(test_name, &handle, &mut logs_accumulator).await;
        test_auto_flush(test_name, &handle, &mut logs_accumulator).await;
        test_flood(test_name, &handle, &mut logs_accumulator).await;
    }
    run_tests(test_name, db);
}
