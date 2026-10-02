use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

use game_media_vault_connectors::{
    Fetched, HttpTransport, ReqwestHttpTransport, RetryPolicy, Validators,
};

/// Retries quickly, so the tests do not wait.
const FAST_RETRIES: RetryPolicy = RetryPolicy {
    max_attempts: 3,
    base_delay: Duration::from_millis(1),
    max_delay: Duration::from_millis(4),
};

/// Answers successive requests on a local port with `responses`, each a status line and its
/// headers, and counts the requests served.
fn serve(responses: Vec<&'static str>) -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let served = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&served);
    thread::spawn(move || {
        for response in responses {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request);
            counter.fetch_add(1, Ordering::SeqCst);
            let body = if response.starts_with("HTTP/1.1 200") {
                "media"
            } else {
                ""
            };
            let reply = format!(
                "{response}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(reply.as_bytes()).unwrap();
        }
    });
    (format!("http://{address}/media.png"), served)
}

fn fetch(transport: &ReqwestHttpTransport, url: &str) -> Result<String, String> {
    let mut body = String::new();
    transport
        .get_stream(url)
        .map_err(|error| error.message().to_owned())?
        .read_to_string(&mut body)
        .unwrap();
    Ok(body)
}

#[test]
fn media_the_server_no_longer_has_is_unavailable_and_never_retried() {
    for status_line in ["HTTP/1.1 404 Not Found", "HTTP/1.1 410 Gone"] {
        let (url, served) = serve(vec![status_line, "HTTP/1.1 200 OK"]);

        let error = ReqwestHttpTransport::with_retry_policy(FAST_RETRIES)
            .get_stream(&url)
            .err()
            .unwrap();

        assert!(error.is_unavailable(), "{status_line}: {}", error.message());
        assert_eq!(served.load(Ordering::SeqCst), 1, "{status_line}");
    }
}

#[test]
fn transient_failures_are_retried_until_a_request_succeeds() {
    let (url, served) = serve(vec![
        "HTTP/1.1 503 Service Unavailable",
        "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 0",
        "HTTP/1.1 200 OK",
    ]);

    let body = fetch(&ReqwestHttpTransport::with_retry_policy(FAST_RETRIES), &url).unwrap();

    assert_eq!(body, "media");
    assert_eq!(served.load(Ordering::SeqCst), 3);
}

#[test]
fn retries_stop_after_the_attempts_the_policy_allows() {
    let (url, served) = serve(vec!["HTTP/1.1 502 Bad Gateway"; 5]);

    let error = ReqwestHttpTransport::with_retry_policy(FAST_RETRIES)
        .get_stream(&url)
        .err()
        .unwrap();

    // A Source that keeps failing stays retryable by a later execution.
    assert!(!error.is_unavailable(), "{}", error.message());
    assert!(
        error.message().contains("after 3 attempts"),
        "{}",
        error.message()
    );
    assert_eq!(served.load(Ordering::SeqCst), 3);
}

#[test]
fn requests_the_server_refuses_are_not_retried() {
    let (url, served) = serve(vec!["HTTP/1.1 403 Forbidden", "HTTP/1.1 200 OK"]);

    let error = ReqwestHttpTransport::with_retry_policy(FAST_RETRIES)
        .get_stream(&url)
        .err()
        .unwrap();

    assert!(error.message().contains("403"), "{}", error.message());
    assert_eq!(served.load(Ordering::SeqCst), 1);
}

#[test]
fn a_source_that_cannot_be_reached_is_retried_then_reported() {
    // A port nothing listens on refuses every connection; Windows takes seconds to say so.
    let policy = RetryPolicy {
        max_attempts: 2,
        ..FAST_RETRIES
    };
    let address = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();

    let error = ReqwestHttpTransport::with_retry_policy(policy)
        .get_stream(&format!("http://{address}/media.png"))
        .err()
        .unwrap();

    assert!(
        error.message().contains("after 2 attempts"),
        "{}",
        error.message()
    );
}

/// Serves `body` once with `headers`, dropping the connection after `cut_after` bytes, then
/// answers each later request with the next of `resume_replies` (given the request it received).
fn serve_cut_short(
    headers: &'static str,
    body: &'static [u8],
    cut_after: usize,
    resume_replies: Vec<fn(&str) -> Vec<u8>>,
) -> (String, Arc<std::sync::Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = Arc::clone(&requests);
    thread::spawn(move || {
        for attempt in 0..=resume_replies.len() {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut request = [0_u8; 2048];
            let read = stream.read(&mut request).unwrap_or(0);
            let request = String::from_utf8_lossy(&request[..read]).into_owned();
            seen.lock().unwrap().push(request.clone());
            if attempt == 0 {
                let head = format!(
                    "HTTP/1.1 200 OK\r\n{headers}\r\nContent-Length: {}\r\n\r\n",
                    body.len()
                );
                stream.write_all(head.as_bytes()).unwrap();
                stream.write_all(&body[..cut_after]).unwrap();
                // Dropping the stream cuts the body short.
            } else {
                stream
                    .write_all(&resume_replies[attempt - 1](&request))
                    .unwrap();
            }
        }
    });
    (format!("http://{address}/media.png"), requests)
}

fn resumed_tail(_request: &str) -> Vec<u8> {
    b"HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 4-9/10\r\nContent-Length: 6\r\n\r\n456789"
        .to_vec()
}

fn read_all(transport: &ReqwestHttpTransport, url: &str) -> std::io::Result<Vec<u8>> {
    let mut body = Vec::new();
    transport
        .get_stream(url)
        .unwrap()
        .read_to_end(&mut body)
        .map(|_| body)
}

#[test]
fn a_download_cut_short_resumes_where_it_stopped() {
    let (url, requests) = serve_cut_short(
        "Accept-Ranges: bytes\r\nETag: \"v1\"",
        b"0123456789",
        4,
        vec![resumed_tail],
    );

    let body = read_all(&ReqwestHttpTransport::with_retry_policy(FAST_RETRIES), &url).unwrap();

    assert_eq!(body, b"0123456789");
    let requests = requests.lock().unwrap();
    let resume = requests[1].to_ascii_lowercase();
    assert!(resume.contains("range: bytes=4-"), "{resume}");
    assert!(resume.contains("if-range: \"v1\""), "{resume}");
}

#[test]
fn a_download_cut_short_fails_when_the_source_cannot_resume_it() {
    // Without byte ranges, or without a validator proving the bytes are the same, the body
    // cannot be spliced.
    // A weak ETag or a date is no strong validator either.
    for headers in [
        "ETag: \"v1\"",
        "Accept-Ranges: bytes",
        "Accept-Ranges: bytes
ETag: W/\"v1\"",
        "Accept-Ranges: bytes
Last-Modified: Wed, 21 Oct 2015 07:28:00 GMT",
    ] {
        let (url, requests) = serve_cut_short(headers, b"0123456789", 4, vec![resumed_tail]);

        let result = read_all(&ReqwestHttpTransport::with_retry_policy(FAST_RETRIES), &url);

        assert!(result.is_err(), "{headers}");
        assert_eq!(requests.lock().unwrap().len(), 1, "{headers}");
    }
}

#[test]
fn a_download_whose_media_changed_meanwhile_is_not_spliced() {
    // The Source answers the resume with the whole, changed media.
    let (url, _) = serve_cut_short(
        "Accept-Ranges: bytes\r\nETag: \"v1\"",
        b"0123456789",
        4,
        vec![|_| {
            b"HTTP/1.1 200 OK\r\nETag: \"v2\"\r\nContent-Length: 10\r\n\r\nabcdefghij".to_vec()
        }],
    );

    let result = read_all(&ReqwestHttpTransport::with_retry_policy(FAST_RETRIES), &url);

    assert!(result.is_err());
}

fn unavailable_for_now(_request: &str) -> Vec<u8> {
    b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n".to_vec()
}

#[test]
fn a_resume_the_source_refuses_for_now_is_tried_again() {
    let (url, requests) = serve_cut_short(
        "Accept-Ranges: bytes\r\nETag: \"v1\"",
        b"0123456789",
        4,
        vec![unavailable_for_now, resumed_tail],
    );

    let body = read_all(&ReqwestHttpTransport::with_retry_policy(FAST_RETRIES), &url).unwrap();

    assert_eq!(body, b"0123456789");
    assert_eq!(requests.lock().unwrap().len(), 3);
}

#[test]
fn resumes_stop_after_the_attempts_the_policy_allows() {
    // Three attempts in all: the first request and two resumes.
    let (url, requests) = serve_cut_short(
        "Accept-Ranges: bytes\r\nETag: \"v1\"",
        b"0123456789",
        4,
        vec![unavailable_for_now, unavailable_for_now, resumed_tail],
    );

    let result = read_all(&ReqwestHttpTransport::with_retry_policy(FAST_RETRIES), &url);

    assert!(result.is_err());
    assert_eq!(requests.lock().unwrap().len(), 3);
}

/// Answers successive connections with the raw `responses`, each written as is and the
/// connection then closed, so a response whose body is shorter than its length is cut short.
fn serve_raw(responses: Vec<Vec<u8>>) -> (String, Arc<std::sync::Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = Arc::clone(&requests);
    thread::spawn(move || {
        for response in responses {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut request = [0_u8; 2048];
            let read = stream.read(&mut request).unwrap_or(0);
            seen.lock()
                .unwrap()
                .push(String::from_utf8_lossy(&request[..read]).into_owned());
            // Each answer closes its connection, so the client must not reuse it: announcing it
            // keeps a pooled connection from failing the next request on some platforms.
            let status_line_end = response
                .windows(2)
                .position(|pair| pair == b"\r\n")
                .unwrap()
                + 2;
            stream.write_all(&response[..status_line_end]).unwrap();
            stream.write_all(b"Connection: close\r\n").unwrap();
            stream.write_all(&response[status_line_end..]).unwrap();
        }
    });
    (format!("http://{address}/media.png"), requests)
}

const CUT_SHORT: &[u8] =
    b"HTTP/1.1 200 OK\r\nAccept-Ranges: bytes\r\nETag: \"v1\"\r\nContent-Length: 10\r\n\r\n0123";
const UNAVAILABLE_FOR_NOW: &[u8] = b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n";
const TAIL: &[u8] =
    b"HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 4-9/10\r\nContent-Length: 6\r\n\r\n456789";

#[test]
fn resumes_share_the_attempts_the_first_request_used() {
    // The first attempt fails, the second is cut short, and the third resumes in vain: three
    // attempts in all, so no fourth request is sent.
    let (url, requests) = serve_raw(vec![
        UNAVAILABLE_FOR_NOW.to_vec(),
        CUT_SHORT.to_vec(),
        UNAVAILABLE_FOR_NOW.to_vec(),
        TAIL.to_vec(),
    ]);

    let result = read_all(&ReqwestHttpTransport::with_retry_policy(FAST_RETRIES), &url);

    assert!(result.is_err());
    assert_eq!(requests.lock().unwrap().len(), 3);
}

#[test]
fn a_range_that_stops_short_of_the_media_is_not_spliced() {
    let (url, _) = serve_raw(vec![
        CUT_SHORT.to_vec(),
        b"HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 4-5/10\r\nContent-Length: 2\r\n\r\n45"
            .to_vec(),
    ]);

    let result = read_all(&ReqwestHttpTransport::with_retry_policy(FAST_RETRIES), &url);

    assert!(result.is_err(), "{result:?}");
}

#[test]
fn range_units_compare_regardless_of_case() {
    let (url, _) = serve_raw(vec![
        b"HTTP/1.1 200 OK\r\nAccept-Ranges: Bytes\r\nETag: \"v1\"\r\nContent-Length: 10\r\n\r\n0123"
            .to_vec(),
        b"HTTP/1.1 206 Partial Content\r\nContent-Range: BYTES 4-9/10\r\nContent-Length: 6\r\n\r\n456789"
            .to_vec(),
    ]);

    let body = read_all(&ReqwestHttpTransport::with_retry_policy(FAST_RETRIES), &url).unwrap();

    assert_eq!(body, b"0123456789");
}

#[test]
fn a_redirected_download_resumes_the_media_it_was_redirected_to() {
    let (url, requests) = serve_raw(vec![
        b"HTTP/1.1 302 Found\r\nLocation: /final.png\r\nContent-Length: 0\r\n\r\n".to_vec(),
        CUT_SHORT.to_vec(),
        TAIL.to_vec(),
    ]);

    let body = read_all(&ReqwestHttpTransport::with_retry_policy(FAST_RETRIES), &url).unwrap();

    assert_eq!(body, b"0123456789");
    let requests = requests.lock().unwrap();
    assert!(requests[2].starts_with("GET /final.png"), "{}", requests[2]);
}

#[test]
fn a_redirect_loop_is_not_retried() {
    let redirect = b"HTTP/1.1 302 Found\r\nLocation: /media.png\r\nContent-Length: 0\r\n\r\n";
    let (url, requests) = serve_raw(vec![redirect.to_vec(); 40]);

    let result = ReqwestHttpTransport::with_retry_policy(FAST_RETRIES).get_stream(&url);

    assert!(result.is_err());
    // One attempt follows the redirects reqwest allows; a retry would follow them again.
    assert!(
        requests.lock().unwrap().len() <= 11,
        "{}",
        requests.lock().unwrap().len()
    );
}

#[test]
fn a_resumed_body_ending_before_the_media_does_is_not_complete() {
    // The range claims the rest of the media, but its framing carries only two bytes.
    let (url, _) = serve_raw(vec![
        CUT_SHORT.to_vec(),
        b"HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 4-9/10\r\nContent-Length: 2\r\n\r\n45"
            .to_vec(),
    ]);

    let result = read_all(&ReqwestHttpTransport::with_retry_policy(FAST_RETRIES), &url);

    assert!(result.is_err(), "{result:?}");
}

#[test]
fn a_resume_caught_in_a_redirect_loop_is_not_retried() {
    let redirect = b"HTTP/1.1 302 Found\r\nLocation: /media.png\r\nContent-Length: 0\r\n\r\n";
    let mut responses = vec![CUT_SHORT.to_vec()];
    responses.extend(vec![redirect.to_vec(); 40]);
    let (url, requests) = serve_raw(responses);

    let result = read_all(&ReqwestHttpTransport::with_retry_policy(FAST_RETRIES), &url);

    assert!(result.is_err());
    // The first request, then one resume following the redirects reqwest allows.
    assert!(
        requests.lock().unwrap().len() <= 12,
        "{}",
        requests.lock().unwrap().len()
    );
}

/// Serves `media` with the ETag `"v1"`, or answers 304 Not Modified to a request that already
/// has that version, and records each request.
fn serve_versioned(requests: usize) -> (String, Arc<std::sync::Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let log = Arc::clone(&seen);
    thread::spawn(move || {
        for _ in 0..requests {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut request = [0_u8; 2048];
            let read = stream.read(&mut request).unwrap_or(0);
            let request = String::from_utf8_lossy(&request[..read]).to_ascii_lowercase();
            log.lock().unwrap().push(request.clone());
            let reply = if request.contains("if-none-match: \"v1\"") {
                "HTTP/1.1 304 Not Modified\r\nETag: \"v1\"\r\nConnection: close\r\n\r\n".to_owned()
            } else {
                "HTTP/1.1 200 OK\r\nETag: \"v1\"\r\nLast-Modified: Sun, 06 Nov 1994 08:49:37 GMT\r\nContent-Length: 5\r\nConnection: close\r\n\r\nmedia".to_owned()
            };
            stream.write_all(reply.as_bytes()).unwrap();
        }
    });
    (format!("http://{address}/Metadata.zip"), seen)
}

#[test]
fn a_conditional_request_downloads_only_what_changed() {
    let (url, requests) = serve_versioned(2);
    let transport = ReqwestHttpTransport::with_retry_policy(FAST_RETRIES);

    let Fetched::Changed {
        mut body,
        validators,
    } = transport
        .get_if_changed(&url, &Validators::default())
        .unwrap()
    else {
        panic!("a first request fetches the resource");
    };
    let mut media = String::new();
    body.read_to_string(&mut media).unwrap();
    let again = transport.get_if_changed(&url, &validators).unwrap();

    assert_eq!(media, "media");
    assert_eq!(validators.etag.as_deref(), Some("\"v1\""));
    assert_eq!(
        validators.last_modified.as_deref(),
        Some("Sun, 06 Nov 1994 08:49:37 GMT")
    );
    assert!(matches!(again, Fetched::Unchanged));
    let requests = requests.lock().unwrap();
    assert!(requests[1].contains("if-modified-since: sun, 06 nov 1994 08:49:37 gmt"));
}
