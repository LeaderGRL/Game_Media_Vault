use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
};

use game_media_vault_connectors::{HttpTransport, ReqwestHttpTransport};

/// Answers one request on a local port with `status_line` and no body.
fn serve_once(status_line: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0_u8; 1024];
        let _ = stream.read(&mut request);
        let response = format!("{status_line}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        stream.write_all(response.as_bytes()).unwrap();
    });
    format!("http://{address}/media.png")
}

#[test]
fn media_the_server_no_longer_has_is_unavailable() {
    for status_line in ["HTTP/1.1 404 Not Found", "HTTP/1.1 410 Gone"] {
        let url = serve_once(status_line);

        let error = ReqwestHttpTransport::default()
            .get_stream(&url)
            .err()
            .unwrap();

        assert!(error.is_unavailable(), "{status_line}: {}", error.message());
    }
}

#[test]
fn server_failures_stay_retryable() {
    let url = serve_once("HTTP/1.1 503 Service Unavailable");

    let error = ReqwestHttpTransport::default()
        .get_stream(&url)
        .err()
        .unwrap();

    assert!(!error.is_unavailable(), "{}", error.message());
}
