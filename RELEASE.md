# Instructions to prepare a new release

1.  Create a local branch called `release`.

    *   **Make sure the branch is synced to `origin/main`!**
    *   `git fetch`
    *   `git checkout -b release origin/main`.

1.  Create a PR to update:

    *   `NEWS.md`: Set next version number, add date, and clean up notes.
    *   `README.md`: Update version numbers and date.
    *   `Cargo.toml`: Update version number.

1.  Once all tests pass, merge the PR.

1.  Tag the resulting merged commit as `db_logger-X.Y.Z` and push the tag.

1.  Go to GitHub and create a new release by copying the release notes into it.

1.  Push the new crate out. This is the last step because it's not reversible:

    *   `cargo publish`
