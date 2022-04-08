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

//! Integration tests for the database logger using the PostgreSQL backend.

use db_logger::{pgsql, Connection};

mod common;

#[test]
#[ignore = "Requires environment configuration and is expensive"]
fn test_everything() {
    // Initialize the test database.  We must do this outside of the async block created to run
    // the tests because we rely on `drop` to clean up the resources, and because `Drop` is not
    // async.  Our implementation starts a tokio runtime, so we must ensure there is no runtime
    // running when we call it later.
    #[tokio::main]
    async fn prepare() -> Connection {
        pgsql::setup_test().await
    }
    let db = prepare();

    common::do_test_everything("pgsql_test", db.clone());

    // We don't have to explicitly drop `db` here, but this is to clarify that this is where
    // cleaning up the test database happens.
    //
    // Note that the global logger still holds a reference to the database... so if we happen to
    // log anything after this, we enter undefined behavior.
    drop(db);
}
