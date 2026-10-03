use std::{
    collections::HashMap,
    io::{Cursor, Read},
    sync::Mutex,
    time::{Duration, Instant},
};

use game_media_vault_application::PortError;
use game_media_vault_connectors::{HttpTransport, PoliteTransport, SiteManners};

/// A website served from fixtures: each URL answers its body, a missing one is unavailable as a
/// 404 answer is, and `failing` ones fail. Records every request with when it arrived.
struct Website {
    pages: HashMap<String, String>,
    failing: Vec<String>,
    requests: Mutex<Vec<(String, Instant)>>,
}

impl Website {
    fn with(pages: &[(&str, &str)], failing: &[&str]) -> Self {
        Self {
            pages: pages
                .iter()
                .map(|(url, body)| ((*url).to_owned(), (*body).to_owned()))
                .collect(),
            failing: failing.iter().map(|url| (*url).to_owned()).collect(),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn requested(&self) -> Vec<String> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .map(|(url, _)| url.clone())
            .collect()
    }
}

impl HttpTransport for &Website {
    fn get_stream(&self, url: &str) -> Result<Box<dyn Read + Send>, PortError> {
        self.requests
            .lock()
            .unwrap()
            .push((url.to_owned(), Instant::now()));
        if self.failing.iter().any(|failing| failing == url) {
            return Err(PortError::new(format!("download of {url} failed")));
        }
        match self.pages.get(url) {
            Some(body) => Ok(Box::new(Cursor::new(body.clone().into_bytes()))),
            None => Err(PortError::unavailable(format!(
                "download returned HTTP 404 Not Found for {url}"
            ))),
        }
    }
}

const ROBOTS: &str = "https://example.org/robots.txt";

/// How much earlier than its turn a request may be recorded: the time of a request is taken once
/// the thread waiting for its turn wakes, which timers may round by a fraction of a millisecond.
const TIMER_SLACK: Duration = Duration::from_millis(5);

fn read(transport: &impl HttpTransport, url: &str) -> Result<String, PortError> {
    let mut body = String::new();
    transport
        .get_stream(url)?
        .read_to_string(&mut body)
        .unwrap();
    Ok(body)
}

#[test]
fn asks_for_nothing_a_site_disallows_and_reads_its_robots_txt_once() {
    let site = Website::with(
        &[
            (ROBOTS, "User-agent: *\nDisallow: /private/\n"),
            ("https://example.org/games/a.html", "game a"),
            ("https://example.org/games/b.html", "game b"),
            ("https://example.org/private/c.html", "secret"),
        ],
        &[],
    );
    let polite = PoliteTransport::with_delay(&site, Duration::ZERO);

    assert_eq!(
        read(&polite, "https://example.org/games/a.html").unwrap(),
        "game a"
    );
    let refused = read(&polite, "https://example.org/private/c.html").unwrap_err();
    assert_eq!(
        read(&polite, "https://example.org/games/b.html").unwrap(),
        "game b"
    );

    assert!(
        refused.message().contains("robots.txt"),
        "{}",
        refused.message()
    );
    assert_eq!(
        site.requested(),
        [
            ROBOTS,
            "https://example.org/games/a.html",
            "https://example.org/games/b.html"
        ]
    );
}

#[test]
fn rules_naming_this_application_apply_to_it() {
    let site = Website::with(
        &[
            (
                ROBOTS,
                "User-agent: *\nAllow: /\n\nUser-agent: game-media-vault\nDisallow: /\n",
            ),
            ("https://example.org/games/a.html", "game a"),
        ],
        &[],
    );
    let polite = PoliteTransport::with_delay(&site, Duration::ZERO);

    assert!(read(&polite, "https://example.org/games/a.html").is_err());
    assert_eq!(site.requested(), [ROBOTS]);
}

#[test]
fn a_site_without_robots_txt_allows_everything() {
    let site = Website::with(&[("https://example.org/games/a.html", "game a")], &[]);
    let polite = PoliteTransport::with_delay(&site, Duration::ZERO);

    assert_eq!(
        read(&polite, "https://example.org/games/a.html").unwrap(),
        "game a"
    );
}

#[test]
fn a_site_whose_robots_txt_cannot_be_read_is_asked_nothing() {
    let site = Website::with(&[("https://example.org/games/a.html", "game a")], &[ROBOTS]);
    let polite = PoliteTransport::with_delay(&site, Duration::ZERO);

    let error = read(&polite, "https://example.org/games/a.html").unwrap_err();

    assert!(
        error.message().contains("robots.txt"),
        "{}",
        error.message()
    );
    assert_eq!(site.requested(), [ROBOTS]);
}

#[test]
fn requests_to_a_site_are_spaced_by_the_delay() {
    let site = Website::with(
        &[
            ("https://example.org/games/a.html", "game a"),
            ("https://example.org/games/b.html", "game b"),
        ],
        &[],
    );
    let polite = PoliteTransport::with_delay(&site, Duration::from_millis(60));

    read(&polite, "https://example.org/games/a.html").unwrap();
    read(&polite, "https://example.org/games/b.html").unwrap();

    let requests = site.requests.lock().unwrap();
    let times: Vec<Instant> = requests.iter().map(|(_, at)| *at).collect();
    // robots.txt, then each page, each at least the delay after the previous request.
    assert_eq!(times.len(), 3);
    for pair in times.windows(2) {
        assert!(pair[1] - pair[0] + TIMER_SLACK >= Duration::from_millis(60));
    }
}

#[test]
fn a_crawl_delay_longer_than_the_delay_is_honored() {
    let site = Website::with(
        &[
            (ROBOTS, "User-agent: *\nCrawl-delay: 0.1\n"),
            ("https://example.org/games/a.html", "game a"),
            ("https://example.org/games/b.html", "game b"),
        ],
        &[],
    );
    let polite = PoliteTransport::with_delay(&site, Duration::ZERO);

    read(&polite, "https://example.org/games/a.html").unwrap();
    read(&polite, "https://example.org/games/b.html").unwrap();

    let requests = site.requests.lock().unwrap();
    assert!(requests[2].1 - requests[1].1 + TIMER_SLACK >= Duration::from_millis(100));
}

#[test]
fn a_site_asking_for_longer_pauses_than_this_application_takes_is_left_alone() {
    for crawl_delay in ["inf", "1e30", "3600"] {
        let site = Website::with(
            &[
                (
                    ROBOTS,
                    &format!("User-agent: *\nCrawl-delay: {crawl_delay}\n"),
                ),
                ("https://example.org/games/a.html", "game a"),
            ],
            &[],
        );
        let polite = PoliteTransport::with_delay(&site, Duration::ZERO);

        let error = read(&polite, "https://example.org/games/a.html").unwrap_err();

        assert!(
            error.message().contains("between requests"),
            "{crawl_delay}: {}",
            error.message()
        );
        assert_eq!(site.requested(), [ROBOTS], "{crawl_delay}");
    }
}

#[test]
fn transports_sharing_their_manners_read_robots_txt_once_and_keep_the_pace_together() {
    let site = Website::with(
        &[
            ("https://example.org/games/a.html", "game a"),
            ("https://example.org/games/b.html", "game b"),
        ],
        &[],
    );
    let manners = std::sync::Arc::new(SiteManners::new(Duration::from_millis(60)));
    let first = PoliteTransport::with_manners(&site, std::sync::Arc::clone(&manners));
    let second = PoliteTransport::with_manners(&site, manners);

    read(&first, "https://example.org/games/a.html").unwrap();
    read(&second, "https://example.org/games/b.html").unwrap();

    let requests = site.requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    assert!(requests[2].1 - requests[1].1 + TIMER_SLACK >= Duration::from_millis(60));
}

#[test]
fn rules_past_the_first_half_mebibyte_of_robots_txt_are_ignored() {
    let padded = format!(
        "{}User-agent: *\nDisallow: /\n",
        "# padding\n".repeat(60_000)
    );
    let site = Website::with(
        &[
            (ROBOTS, &padded),
            ("https://example.org/games/a.html", "game a"),
        ],
        &[],
    );
    let polite = PoliteTransport::with_delay(&site, Duration::ZERO);

    assert_eq!(
        read(&polite, "https://example.org/games/a.html").unwrap(),
        "game a"
    );
}

/// Answers every page at once, but `https://slow.example/robots.txt` only after a while.
struct SlowRobots;

impl HttpTransport for SlowRobots {
    fn get_stream(&self, url: &str) -> Result<Box<dyn Read + Send>, PortError> {
        if url == "https://slow.example/robots.txt" {
            std::thread::sleep(Duration::from_millis(400));
        }
        if url.ends_with("/robots.txt") {
            return Err(PortError::unavailable(format!("no {url}")));
        }
        Ok(Box::new(Cursor::new(b"page".to_vec())))
    }
}

#[test]
fn a_site_slow_to_answer_its_robots_txt_never_holds_back_another_site() {
    let polite = std::sync::Arc::new(PoliteTransport::with_delay(SlowRobots, Duration::ZERO));
    let slow = {
        let polite = std::sync::Arc::clone(&polite);
        std::thread::spawn(move || read(&*polite, "https://slow.example/a.html").unwrap())
    };
    std::thread::sleep(Duration::from_millis(50));

    let started = Instant::now();
    read(&*polite, "https://fast.example/a.html").unwrap();

    assert!(
        started.elapsed() < Duration::from_millis(200),
        "{:?}",
        started.elapsed()
    );
    slow.join().unwrap();
}

/// Fails the first read of robots.txt, as a site briefly unreachable does, then answers it.
struct FlakyRobots {
    robots_reads: std::sync::atomic::AtomicUsize,
}

impl HttpTransport for FlakyRobots {
    fn get_stream(&self, url: &str) -> Result<Box<dyn Read + Send>, PortError> {
        if url.ends_with("/robots.txt") {
            let reads = self
                .robots_reads
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if reads == 0 {
                return Err(PortError::new(format!(
                    "download returned HTTP 503 Service Unavailable for {url}"
                )));
            }
            return Ok(Box::new(Cursor::new(b"User-agent: *\nAllow: /\n".to_vec())));
        }
        Ok(Box::new(Cursor::new(b"page".to_vec())))
    }
}

#[test]
fn a_robots_txt_that_failed_to_load_is_read_again_by_the_next_request() {
    let polite = PoliteTransport::with_delay(
        FlakyRobots {
            robots_reads: std::sync::atomic::AtomicUsize::new(0),
        },
        Duration::ZERO,
    );

    let first = read(&polite, "https://example.org/games/a.html").unwrap_err();

    assert!(
        first.message().contains("robots.txt"),
        "{}",
        first.message()
    );
    assert_eq!(
        read(&polite, "https://example.org/games/a.html").unwrap(),
        "page"
    );
}

#[test]
fn a_failed_robots_txt_read_is_followed_by_the_pause_too() {
    let site = Website::with(&[], &[ROBOTS]);
    let polite = PoliteTransport::with_delay(&site, Duration::from_millis(60));

    assert!(read(&polite, "https://example.org/games/a.html").is_err());
    assert!(read(&polite, "https://example.org/games/a.html").is_err());

    // Each failed read of robots.txt was a request, and the next one waited its turn.
    let requests = site.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[1].1 - requests[0].1 + TIMER_SLACK >= Duration::from_millis(60));
}
