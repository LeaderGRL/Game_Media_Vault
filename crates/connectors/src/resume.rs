//! Response bodies that resume where a dropped connection cut them short.

use std::{
    io::{self, Read},
    thread,
    time::{Duration, SystemTime},
};

use reqwest::{
    StatusCode, Url,
    blocking::{Client, Response},
    header::{ACCEPT_RANGES, CONTENT_LENGTH, CONTENT_RANGE, ETAG, IF_RANGE, RANGE, RETRY_AFTER},
};

use crate::{RetryPolicy, retry::parse_retry_after};

/// A response body that, when the connection drops partway, asks the Source for the rest.
///
/// It resumes only from the resource the first response actually came from (after redirects),
/// only while that resource serves byte ranges and a strong ETag proves the media unchanged,
/// and only with a range that carries the whole rest of the media. It never splices bytes of
/// two different versions, and every request it sends counts against the attempts the retry
/// policy allows in all, including those spent before the first response.
pub(crate) struct ResumingBody {
    client: Client,
    retry: RetryPolicy,
    /// The resource the first response came from, after any redirect.
    url: Url,
    response: Response,
    received: u64,
    /// The length of the whole media, when the first response told it.
    total: Option<u64>,
    /// The strong ETag of the media being read; resuming needs one.
    validator: Option<String>,
    serves_ranges: bool,
    /// Requests sent so far, the first ones included.
    attempts: u32,
}

enum Resume {
    Resumed(Response),
    /// The Source cannot continue this body, such as when its media changed meanwhile.
    Refused,
    /// Asking again may succeed, after the wait the Source asked for, if any.
    Failed(Option<Duration>),
}

impl ResumingBody {
    pub(crate) fn new(
        client: Client,
        retry: RetryPolicy,
        attempts: u32,
        response: Response,
    ) -> Self {
        let header = |name| {
            response
                .headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
        };
        // A weak ETag, or a date that may not tell two versions apart, cannot validate a range.
        let validator = header(ETAG).filter(|tag| !tag.trim_start().starts_with("W/"));
        let serves_ranges = header(ACCEPT_RANGES).is_some_and(|ranges| {
            ranges
                .split(',')
                .any(|unit| unit.trim().eq_ignore_ascii_case("bytes"))
        });
        let total = header(CONTENT_LENGTH).and_then(|length| length.trim().parse().ok());
        Self {
            client,
            retry,
            url: response.url().clone(),
            response,
            received: 0,
            total,
            validator,
            serves_ranges,
            attempts,
        }
    }

    fn can_resume(&self) -> bool {
        self.serves_ranges && self.validator.is_some() && self.attempts < self.retry.max_attempts
    }

    fn resume(&self) -> Resume {
        let Some(validator) = self.validator.as_deref() else {
            return Resume::Refused;
        };
        let Ok(response) = self
            .client
            .get(self.url.clone())
            .header(RANGE, format!("bytes={}-", self.received))
            .header(IF_RANGE, validator)
            .send()
        else {
            return Resume::Failed(None);
        };
        let status = response.status();
        if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
            let retry_after = response
                .headers()
                .get(RETRY_AFTER)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| parse_retry_after(value, SystemTime::now()));
            return Resume::Failed(retry_after);
        }
        // A redirect to another resource would answer for media this validator does not name.
        if status != StatusCode::PARTIAL_CONTENT || response.url() != &self.url {
            return Resume::Refused;
        }
        let rest = response
            .headers()
            .get(CONTENT_RANGE)
            .and_then(|value| value.to_str().ok())
            .and_then(content_range);
        match rest {
            Some((start, end, total))
                if start == self.received
                    && end.checked_add(1) == Some(total)
                    && self.total.is_none_or(|known| known == total) =>
            {
                Resume::Resumed(response)
            }
            _ => Resume::Refused,
        }
    }

    /// Replaces the broken response with the rest of the body, or fails with `error`. The broken
    /// response is never read again: after its error it may report a clean end of the body.
    fn resume_after(&mut self, error: io::Error) -> io::Result<()> {
        let mut retry_after = None;
        loop {
            if !self.can_resume() {
                return Err(error);
            }
            thread::sleep(self.retry.delay_before_retry(self.attempts, retry_after));
            self.attempts += 1;
            match self.resume() {
                Resume::Resumed(response) => {
                    self.response = response;
                    return Ok(());
                }
                Resume::Refused => return Err(error),
                Resume::Failed(requested) => retry_after = requested,
            }
        }
    }
}

/// The first byte, last byte and complete length a `Content-Range` such as `bytes 4-9/10`
/// states, its unit compared regardless of case.
fn content_range(value: &str) -> Option<(u64, u64, u64)> {
    let (unit, range) = value.trim().split_once(' ')?;
    if !unit.eq_ignore_ascii_case("bytes") {
        return None;
    }
    let (span, total) = range.trim().split_once('/')?;
    let (start, end) = span.split_once('-')?;
    Some((start.parse().ok()?, end.parse().ok()?, total.parse().ok()?))
}

impl Read for ResumingBody {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        loop {
            match self.response.read(buffer) {
                Ok(read) => {
                    self.received += read as u64;
                    return Ok(read);
                }
                Err(error) => self.resume_after(error)?,
            }
        }
    }
}
