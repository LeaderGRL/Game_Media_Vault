//! Response bodies that resume where a dropped connection cut them short.

use std::{
    io::{self, Read},
    thread,
};

use reqwest::{
    StatusCode,
    blocking::{Client, Response},
    header::{ACCEPT_RANGES, CONTENT_RANGE, ETAG, IF_RANGE, LAST_MODIFIED, RANGE},
};

use crate::RetryPolicy;

/// A response body that, when the connection drops partway, asks the Source for the rest. It
/// resumes only while the Source serves byte ranges and a validator proves the media unchanged,
/// so it never splices bytes of two different versions, and within the retry policy's attempts.
pub(crate) struct ResumingBody {
    client: Client,
    retry: RetryPolicy,
    url: String,
    response: Response,
    received: u64,
    /// The strong ETag, or else the Last-Modified date, of the media being read.
    validator: Option<String>,
    serves_ranges: bool,
    resumes: u32,
}

enum Resume {
    Resumed(Response),
    /// The Source cannot continue this body, such as when its media changed meanwhile.
    Refused,
    /// Asking again may succeed.
    Failed,
}

impl ResumingBody {
    pub(crate) fn new(client: Client, retry: RetryPolicy, url: &str, response: Response) -> Self {
        let header = |name| {
            response
                .headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
        };
        // A weak ETag cannot validate a range, so the date stands in for it.
        let validator = header(ETAG)
            .filter(|tag| !tag.starts_with("W/"))
            .or_else(|| header(LAST_MODIFIED));
        let serves_ranges = header(ACCEPT_RANGES).is_some_and(|ranges| ranges.contains("bytes"));
        Self {
            client,
            retry,
            url: url.to_owned(),
            response,
            received: 0,
            validator,
            serves_ranges,
            resumes: 0,
        }
    }

    fn can_resume(&self) -> bool {
        self.serves_ranges && self.validator.is_some() && self.resumes + 1 < self.retry.max_attempts
    }

    fn resume(&self) -> Resume {
        let Some(validator) = self.validator.as_deref() else {
            return Resume::Refused;
        };
        let Ok(response) = self
            .client
            .get(&self.url)
            .header(RANGE, format!("bytes={}-", self.received))
            .header(IF_RANGE, validator)
            .send()
        else {
            return Resume::Failed;
        };
        let status = response.status();
        let continues_here = response
            .headers()
            .get(CONTENT_RANGE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|range| range.starts_with(&format!("bytes {}-", self.received)));
        if status == StatusCode::PARTIAL_CONTENT && continues_here {
            Resume::Resumed(response)
        } else if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
            Resume::Failed
        } else {
            Resume::Refused
        }
    }
}

impl ResumingBody {
    /// Replaces the broken response with the rest of the body, or fails with `error`. The broken
    /// response is never read again: after its error it may report a clean end of the body.
    fn resume_after(&mut self, error: io::Error) -> io::Result<()> {
        loop {
            if !self.can_resume() {
                return Err(error);
            }
            self.resumes += 1;
            thread::sleep(self.retry.delay_before_retry(self.resumes, None));
            match self.resume() {
                Resume::Resumed(response) => {
                    self.response = response;
                    return Ok(());
                }
                Resume::Refused => return Err(error),
                Resume::Failed => {}
            }
        }
    }
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
