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

sinclude config.mk

.PHONY: default
default: test

.PHONY: test
test:
	@RUST_LOG=debug \
	    PGSQL_TEST_HOST="$(PGSQL_TEST_HOST)" \
	    PGSQL_TEST_PORT="$(PGSQL_TEST_PORT)" \
	    PGSQL_TEST_DATABASE="$(PGSQL_TEST_DATABASE)" \
	    PGSQL_TEST_USERNAME="$(PGSQL_TEST_USERNAME)" \
	    PGSQL_TEST_PASSWORD="$(PGSQL_TEST_PASSWORD)" \
	    cargo test $(TEST_ARGS) -- --include-ignored

.PHONY: clean
clean:
	@true

.PHONY: distclean
distclean: clean
	rm -rf Cargo.lock target
