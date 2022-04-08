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

//! Integration tests for the database logger using the SQLite backend.

use db_logger::{sqlite, Connection};
use std::path::Path;

mod common;

#[test]
#[ignore = "Is expensive"]
fn test_everything() {
    let temp = tempfile::tempdir().unwrap();
    let test_db = temp.path().join("test.db");

    #[tokio::main]
    async fn prepare(path: &Path) -> Connection {
        sqlite::setup_test(&format!("file:{}?mode=rwc", path.display())).await
    }
    let db = prepare(&test_db);

    common::do_test_everything("sqlite_test", db);
}
