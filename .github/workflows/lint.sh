#! /bin/sh
# db_logger
# Copyright 2022 Julio Merino
#
# Licensed under the Apache License, Version 2.0 (the "License"); you may not
# use this file except in compliance with the License.  You may obtain a copy
# of the License at:
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS, WITHOUT
# WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.  See the
# License for the specific language governing permissions and limitations
# under the License.

set -eu

rustup component add clippy
rustup component add rustfmt

check_do_not_submit() {
    for f in .* *; do
        [ "${f}" != . ] || continue
        [ "${f}" != .. ] || continue
        [ "${f}" != .git ] || continue

        if grep -R '[D]O NOT SUBMIT' "${f}"; then
            echo "Submit blocked by" "DO" "NOT" "SUBMIT" 1>&2
            return 1
        fi
    done
}

check_rust() {
    cargo clippy --all-targets -- -D warnings
    cargo clippy --all-targets --all-features -- -D warnings
    cargo fmt -- --check
}

check_do_not_submit
# These checks must come last to avoid creating artifacts in the source
# directory.
check_rust
