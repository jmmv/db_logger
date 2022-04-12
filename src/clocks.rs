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

//! Collection of clock implementations.

#[cfg(test)]
use std::convert::TryFrom;
#[cfg(test)]
use std::sync::atomic::{AtomicU64, Ordering};
use time::OffsetDateTime;

/// Generic definition of a clock.
pub(crate) trait Clock {
    /// Returns the current UTC time.
    fn now_utc(&self) -> OffsetDateTime;
}

/// Clock implementation that uses the system clock.
#[derive(Default)]
pub(crate) struct SystemClock {}

impl Clock for SystemClock {
    fn now_utc(&self) -> OffsetDateTime {
        let nanos = OffsetDateTime::now_utc().unix_timestamp_nanos();

        // Truncate the timestamp to microsecond resolution as this is the resolution supported by
        // timestamps in the PostgreSQL database.  We could do this in the database instead, but
        // then we would get some strange behavior throughout the program.  Better be consistent.
        let nanos = nanos / 1000 * 1000;

        OffsetDateTime::from_unix_timestamp_nanos(nanos)
    }
}

/// A clock that returns a monotonically increasing instant every time it is queried.
#[cfg(test)]
pub(crate) struct MonotonicClock {
    now: AtomicU64,
}

#[cfg(test)]
impl MonotonicClock {
    /// Creates a new clock whose "now" start time is `now`.
    pub(crate) fn new(now: u64) -> Self {
        Self { now: AtomicU64::new(now) }
    }
}

#[cfg(test)]
impl Clock for MonotonicClock {
    fn now_utc(&self) -> OffsetDateTime {
        let now = self.now.fetch_add(1, Ordering::SeqCst);
        OffsetDateTime::from_unix_timestamp(i64::try_from(now).expect("Mock timestamp too long"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_systemclock_trivial() {
        let clock = SystemClock::default();
        let now1 = clock.now_utc();
        assert!(now1.unix_timestamp_nanos() > 0);
        let now2 = clock.now_utc();
        assert!(now2 >= now1);
    }

    #[test]
    fn test_systemclock_microsecond_resolution() {
        let clock = SystemClock::default();
        let now = clock.now_utc();
        assert!(now.unix_timestamp_nanos() > 0);
        assert_eq!(0, now.nanosecond() % 1000);
    }

    #[test]
    fn test_monotonicclock() {
        let clock = MonotonicClock::new(123);
        assert_eq!(OffsetDateTime::from_unix_timestamp(123), clock.now_utc());
        assert_eq!(OffsetDateTime::from_unix_timestamp(124), clock.now_utc());
        assert_eq!(OffsetDateTime::from_unix_timestamp(125), clock.now_utc());
    }
}
