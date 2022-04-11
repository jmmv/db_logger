-- db_logger
-- Copyright 2022 Julio Merino
--
-- Licensed under the Apache License, Version 2.0 (the "License"); you may not
-- use this file except in compliance with the License.  You may obtain a copy
-- of the License at:
--
--     http://www.apache.org/licenses/LICENSE-2.0
--
-- Unless required by applicable law or agreed to in writing, software
-- distributed under the License is distributed on an "AS IS" BASIS, WITHOUT
-- WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.  See the
-- License for the specific language governing permissions and limitations
-- under the License.

CREATE TABLE logs (
    timestamp TIMESTAMPTZ NOT NULL,

    -- The sequence number is a monotonically increasing number for each instance of the server that
    -- wraps around.  Needed to disambiguate log messages when the timestamps do not have sufficient
    -- granularity.
    -- TODO(jmmv): A SMALLSERIAL would be sufficient, but sqlx 0.5 doesn't support them.
    sequence SERIAL NOT NULL,

    hostname VARCHAR(64) NOT NULL,

    level SMALLINT NOT NULL,

    module VARCHAR(64),
    filename VARCHAR(256),
    line SMALLINT,

    message VARCHAR(1024) NOT NULL,

    PRIMARY KEY (timestamp, sequence, hostname)
);

CREATE INDEX logs_by_timestamp ON logs (timestamp, sequence);
