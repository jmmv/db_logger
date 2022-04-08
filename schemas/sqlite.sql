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
    timestamp_secs INTEGER NOT NULL,
    timestamp_nsecs INTEGER NOT NULL,
    sequence INTEGER NOT NULL,
    hostname TEXT,

    level INTEGER NOT NULL,

    module TEXT,
    filename TEXT,
    line INTEGER,

    message TEXT NOT NULL,

    PRIMARY KEY (timestamp_secs, timestamp_nsecs, sequence, hostname)
);

CREATE INDEX logs_by_timestamp ON logs (timestamp_secs, timestamp_nsecs, sequence);
