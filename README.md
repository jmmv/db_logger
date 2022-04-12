# A database-backed logger for use with the log crate

[![crates.io](https://img.shields.io/crates/v/db_logger.svg)](https://crates.io/crates/db_logger/)
[![docs.rs](https://docs.rs/db_logger/badge.svg)](https://docs.rs/db_logger/)

db\_logger is a Rust crate providing an implementation of the
[log crate](https://crates.io/crates/log)'s logging facade to write structured
log entries to a database.  Just add a few lines of code to the beginning of
your program and all logging will be saved for later analysis, which is
especially suited to (distributed) services.

db\_logger currently supports PostgreSQL and SQLite and is backed by the
[sqlx crate](https://crates.io/crates/sqlx).

# Usage

To use db\_logger, you need to add a dependency to your project with the right
set of features, create a database with the expected schema, and then initialize
the logger during program initialization.

As a logging facade implementation, db\_logger should only be depended upon from
binary crates (never from libraries).

## Usage with PostgreSQL

1.  Add the following to your list of dependencies in `Cargo.toml`:

    ```toml
    [dependencies.db_logger]
    version = "0.1"
    default-features = false
    features = ["postgres"]
    ```

1.  Create a PostgreSQL database and initialize it with the
    [`schemas/postgres.sql`](schemas/postgres.sql) schema.

1.  Initialize the logger in your code with one of:

    *   Explicit configuration:

        ```rust
        use db_logger::postgres;
        let conn = postgres::connect_lazy(postgres::ConnectionOptions {
            host: "some host".to_owned(),
            port: 5432,
            database: "some database".to_owned(),
            username: "some username".to_owned(),
            password: "some password".to_owned(),
            ..Default::default()
        });
        let _handle = db_logger::init(conn).await;
        ```

    *   Environment-based configuration:

        ```rust
        use db_logger::postgres;
        let conn = postgres::connect_lazy(
            postgres::ConnectionOptions::from_env("LOGGER").unwrap());
        let _handle = db_logger::init(conn).await;
        ```

        This will cause your program to recognize variables of the form
        `LOGGER_HOST`, `LOGGER_PORT`, `LOGGER_DATABASE`, `LOGGER_USERNAME` and
        `LOGGER_PASSWORD` to configure the PostgreSQL connection.

1.  Make sure to keep `_handle` alive for the duration of the program in an
    async context, because the handle keeps the background logging task alive.

## Usage with SQLite

1.  Add the following to your list of dependencies in `Cargo.toml`:

    ```toml
    [dependencies.db_logger]
    version = "0.1"
    default-features = false
    features = ["sqlite"]
    ```

1.  Create an SQLite database and initialize it with the
    [`schemas/sqlite.sql`](schemas/sqlite.sql) schema.

1.  Initialize with:

    ```rust
    use db_logger::sqlite;
    let conn = sqlite::connect(sqlite::ConnectionOptions {
        uri: "file:/path/to/database?mode=rw",
        ..Default::default()
    }).await.unwrap();
    let _handle = db_logger::init(conn).await;
    ```

1.  Make sure to keep `_handle` alive for the duration of the program in an
    async context, because the handle keeps the background logging task alive.

## Environment configuration

db\_logger recognizes the `RUST_LOG` environment variable to configure the
maximum level of the log messages to record, the same way as the
[env\_logger crate](https://crates.io/crates/env_logger) does.

## Schema initialization

As indicated above, you should create the database and its schema by hand
before establishing a connection using the reference files provided in the
[`schemas`](schemas) directory.  This is a one-time operation.

You also have the option of invoking the `Connection::create_schema()` method
to initialize the database schema.  You probably don't want to do this in
production but this is useful if you are using ephemeral SQLite databases.

# Limitations

The code in this crate was extracted from the
[EndBASIC cloud service](https://www.endbasic.dev/service.html) and then cleaned
for separate publication.  This code was written as a fun experiment as part of
that service and it has not received a lot of real-world stress testing.  As a
result, expect this to have a bunch of limitations, including:

*   Because db\_logger runs in-process, the logger must filter out any log
    messages that it can generate while writing to the database (or else it
    would enter an infinite logging loop).  This includes log messages for its
    own dependencies, such as those emitted by sqlx or tokio, which means that
    if you are using this crate, you won't get their logs saved into the
    database.

*   Performance is not great.  While the logger tries to avoid blocking caller
    code by buffering log messages, if you log a lot, your application will
    slow down.  I'm positive some profiling could make this much better without
    a ton of effort, but I just haven't spent the time doing it.

*   No tracing support.  This crate may be a dead end.  Saving raw logs to a
    database is interesting but what would be more interesting is saving the
    traces generated when integrating with the [tracing
    crate](https://crates.io/crates/tracing) (so as to trace individual
    requests as they flow through an async server, which was the original
    desire).

*   stderr pollution.  Right now, any errors encountered while persisting logs
    to the database and any log messages that are filtered out are dumped to
    stderr in an ad-hoc format.  This behavior should be configurable but isn't.
