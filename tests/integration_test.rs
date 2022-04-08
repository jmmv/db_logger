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

//! Integration tests for the custom logger.
//!
//! These tests intend to exercise the `DbLogger` against the real `PostgresDb`.  This makes testing
//! difficult, but we need this level of integration testing to ensure that the logger works
//! properly and doesn't, for example, enter infinite loops due to recursion.
//!
//! Note also that the logger is, essentially, a global resource.  This means that we can only
//! initialize it once here, which in turn limits the granularity of our tests.  We can only have
//! a single `#[test]` to wrap the whole code in this file

use db_logger::{DbLogger, Handle, PostgresDb, SystemClock};
use gethostname::gethostname;
use log::*;
use std::env;
use std::sync::Arc;
use std::time::Duration;

/// Wrapper to prepare and clean up the test database.
struct TestContext {
    db: Arc<PostgresDb>,
}

impl TestContext {
    /// Initializes the test database.
    fn setup() -> TestContext {
        #[tokio::main]
        async fn prepare() -> TestContext {
            let db = Arc::from(PostgresDb::setup_test().await);
            TestContext { db }
        }
        prepare()
    }
}

impl Drop for TestContext {
    fn drop(&mut self) {
        #[tokio::main]
        async fn cleanup(context: &mut TestContext) {
            context.db.teardown_test().await;
        }
        cleanup(self)
    }
}

/// Generates a log line to match the test format returned by `Db::get_log_entries` for a log
/// entry emitted from this module and processed by `make_deterministic`.
///
/// `level` is the numerical level of the entry; `line` is the line of code where the message was
/// raised, and `message` is the free-form text of the entry.
fn make_log_line(level: u8, line: u32, message: &str) -> String {
    let hostname =
        gethostname().into_string().unwrap_or_else(|_e| String::from("invalid-hostname"));

    format!(
        "SSSS.uuuu {} {} integration_test tests/integration_test.rs:{} {}",
        hostname, level, line, message
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

async fn test_all_levels(handle: &Handle, exp_logs: &mut Vec<String>) {
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

    exp_logs.push(make_log_line(1, base_line + 1, "An error message"));
    exp_logs.push(make_log_line(2, base_line + 2, "A warning message"));
    exp_logs.push(make_log_line(3, base_line + 3, "An info message"));
    exp_logs.push(make_log_line(4, base_line + 4, "A debug message"));
    exp_logs.push(make_log_line(5, base_line + 5, "A trace message"));

    let entries = handle.get_log_entries().await.unwrap();
    assert_eq!(exp_logs, &make_deterministic(entries));
}

async fn test_level_filtering(handle: &Handle, exp_logs: &mut Vec<String>) {
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

    exp_logs.push(make_log_line(1, base_line + 1, "An error message"));
    exp_logs.push(make_log_line(2, base_line + 2, "A warning message"));

    let entries = handle.get_log_entries().await.unwrap();
    assert_eq!(exp_logs, &make_deterministic(entries));
}

async fn test_auto_flush(handle: &Handle, exp_logs: &mut Vec<String>) {
    log::set_max_level(Level::Info.to_level_filter());

    let base_line = line!();
    info!("Another message");
    // Do not call flush here.  We should see the message show up eventually.

    exp_logs.push(make_log_line(3, base_line + 1, "Another message"));

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

async fn test_flood(handle: &Handle, exp_logs: &mut Vec<String>) {
    log::set_max_level(Level::Info.to_level_filter());

    for i in 0..10240 {
        let base_line = line!();
        info!("Message with index {}", i);
        exp_logs.push(make_log_line(3, base_line + 1, &format!("Message with index {}", i)));
    }
    log::logger().flush();

    let entries = handle.get_log_entries().await.unwrap();
    assert_eq!(exp_logs, &make_deterministic(entries));
}

#[test]
#[ignore = "Requires environment configuration and is expensive"]
fn test_everything() {
    // Initialize the test database.  We must do this outside of the async block because we rely on
    // `drop` to clean up the resources, and because `Drop` is not async. Our implementation starts
    // a tokio runtime, so we must ensure there is no runtime running when we call it below.
    let context = TestContext::setup();

    // Launch the tests.
    #[tokio::main(flavor = "multi_thread", worker_threads = 2)]
    async fn run_tests(db: Arc<PostgresDb>) {
        env::set_var("RUST_LOG", "trace");
        let handle = DbLogger::init(db.clone(), Arc::from(SystemClock::default()));

        let mut logs_accumulator = vec![];

        test_all_levels(&handle, &mut logs_accumulator).await;
        test_level_filtering(&handle, &mut logs_accumulator).await;
        test_auto_flush(&handle, &mut logs_accumulator).await;
        test_flood(&handle, &mut logs_accumulator).await;
    }
    run_tests(context.db.clone());

    // We don't have to explicitly drop `context` here, but this is to clarify that this is where
    // cleaning up the test database happens.
    //
    // Note that the global logger still holds a reference to the database... so if we happen to
    // log anything after this, we enter undefined behavior.
    drop(context);
}
