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

use game_media_vault_connectors::{HttpTransport, ReqwestHttpTransport, RetryPolicy};

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
/// answers the next request with `resume_reply` (given the request it received).
fn serve_cut_short(
    headers: &'static str,
    body: &'static [u8],
    cut_after: usize,
    resume_reply: fn(&str) -> Vec<u8>,
) -> (String, Arc<std::sync::Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = Arc::clone(&requests);
    thread::spawn(move || {
        for attempt in 0..2 {
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
                stream.write_all(&resume_reply(&request)).unwrap();
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
        resumed_tail,
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
    for headers in ["ETag: \"v1\"", "Accept-Ranges: bytes"] {
        let (url, requests) = serve_cut_short(headers, b"0123456789", 4, resumed_tail);

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
        |_| b"HTTP/1.1 200 OK\r\nETag: \"v2\"\r\nContent-Length: 10\r\n\r\nabcdefghij".to_vec(),
    );

    let result = read_all(&ReqwestHttpTransport::with_retry_policy(FAST_RETRIES), &url);

    assert!(result.is_err());
}
