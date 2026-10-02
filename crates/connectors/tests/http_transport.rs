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
