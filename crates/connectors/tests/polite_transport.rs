use std::{
    collections::{HashMap, VecDeque},
    io::{Cursor, Read},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use game_media_vault_application::PortError;
use game_media_vault_connectors::{HttpTransport, PaceClock, PoliteTransport, SiteManners};

/// A website served from fixtures: each URL answers its body, a missing one is unavailable as a
/// 404 answer is, and `failing` ones fail. Records every request with when it arrived, by its
/// `clock` if it has one.
struct Website {
    pages: HashMap<String, String>,
    failing: Vec<String>,
    clock: Option<Arc<TestClock>>,
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
            clock: None,
            requests: Mutex::new(Vec::new()),
        }
    }

    /// A website recording its requests by `clock`.
    fn keeping_time(pages: &[(&str, &str)], clock: &Arc<TestClock>) -> Self {
        Self {
            clock: Some(Arc::clone(clock)),
            ..Self::with(pages, &[])
        }
    }

    /// How long after `start` each request arrived.
    fn arrivals_after(&self, start: Instant) -> Vec<Duration> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .map(|(_, at)| *at - start)
            .collect()
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
        self.requests.lock().unwrap().push((
            url.to_owned(),
            self.clock
                .as_ref()
                .map_or_else(Instant::now, |clock| clock.now()),
        ));
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

/// A clock the test moves: a wait moves it on by the time waited, and by how late the thread
/// wakes from it, the next of its late wakes. A thread that does not wait wakes on time.
struct TestClock {
    now: Mutex<Instant>,
    late_wakes: Mutex<VecDeque<Duration>>,
}

impl TestClock {
    fn waking_late_by(late_wakes: &[u64]) -> Arc<Self> {
        Arc::new(Self {
            now: Mutex::new(Instant::now()),
            late_wakes: Mutex::new(
                late_wakes
                    .iter()
                    .map(|late| Duration::from_millis(*late))
                    .collect(),
            ),
        })
    }

    fn on_time() -> Arc<Self> {
        Self::waking_late_by(&[])
    }
}

impl PaceClock for TestClock {
    fn now(&self) -> Instant {
        *self.now.lock().unwrap()
    }

    fn sleep(&self, duration: Duration) {
        if duration.is_zero() {
            return;
        }
        let late = self
            .late_wakes
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_default();
        *self.now.lock().unwrap() += duration + late;
    }
}

/// Manners leaving `delay` between requests by `clock`.
fn manners(delay: u64, clock: &Arc<TestClock>) -> Arc<SiteManners> {
    Arc::new(SiteManners::with_clock(
        Duration::from_millis(delay),
        Box::new(Arc::clone(clock)),
    ))
}

fn millis(times: &[u64]) -> Vec<Duration> {
    times
        .iter()
        .map(|time| Duration::from_millis(*time))
        .collect()
}

const ROBOTS: &str = "https://example.org/robots.txt";

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
    let clock = TestClock::on_time();
    let start = clock.now();
    let site = Website::keeping_time(
        &[
            ("https://example.org/games/a.html", "game a"),
            ("https://example.org/games/b.html", "game b"),
        ],
        &clock,
    );
    let polite = PoliteTransport::with_manners(&site, manners(60, &clock));

    read(&polite, "https://example.org/games/a.html").unwrap();
    read(&polite, "https://example.org/games/b.html").unwrap();

    // robots.txt, then each page, each the delay after the previous request.
    assert_eq!(site.arrivals_after(start), millis(&[0, 60, 120]));
}

#[test]
fn a_crawl_delay_longer_than_the_delay_is_honored() {
    let clock = TestClock::on_time();
    let start = clock.now();
    let site = Website::keeping_time(
        &[
            (ROBOTS, "User-agent: *\nCrawl-delay: 0.5\n"),
            ("https://example.org/games/a.html", "game a"),
            ("https://example.org/games/b.html", "game b"),
        ],
        &clock,
    );
    let polite = PoliteTransport::with_manners(&site, manners(60, &clock));

    read(&polite, "https://example.org/games/a.html").unwrap();
    read(&polite, "https://example.org/games/b.html").unwrap();

    assert_eq!(site.arrivals_after(start), millis(&[0, 500, 1000]));
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
    let clock = TestClock::on_time();
    let start = clock.now();
    let site = Website::keeping_time(
        &[
            ("https://example.org/games/a.html", "game a"),
            ("https://example.org/games/b.html", "game b"),
        ],
        &clock,
    );
    let manners = manners(60, &clock);
    let first = PoliteTransport::with_manners(&site, Arc::clone(&manners));
    let second = PoliteTransport::with_manners(&site, manners);

    read(&first, "https://example.org/games/a.html").unwrap();
    read(&second, "https://example.org/games/b.html").unwrap();

    // One robots.txt, and one pace for both transports.
    assert_eq!(site.arrivals_after(start), millis(&[0, 60, 120]));
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
    let clock = TestClock::on_time();
    let start = clock.now();
    let site = Website {
        clock: Some(Arc::clone(&clock)),
        ..Website::with(&[], &[ROBOTS])
    };
    let polite = PoliteTransport::with_manners(&site, manners(60, &clock));

    assert!(read(&polite, "https://example.org/games/a.html").is_err());
    assert!(read(&polite, "https://example.org/games/a.html").is_err());

    // Each failed read of robots.txt was a request, and the next one waited its turn.
    assert_eq!(site.arrivals_after(start), millis(&[0, 60]));
}

#[test]
fn a_request_sent_late_puts_the_next_one_off_by_the_whole_delay() {
    // The first wait wakes 20 ms late, as a thread of a loaded machine may.
    let clock = TestClock::waking_late_by(&[20]);
    let start = clock.now();
    let site = Website::keeping_time(
        &[
            ("https://example.org/games/a.html", "game a"),
            ("https://example.org/games/b.html", "game b"),
        ],
        &clock,
    );
    let polite = PoliteTransport::with_manners(&site, manners(60, &clock));

    read(&polite, "https://example.org/games/a.html").unwrap();
    read(&polite, "https://example.org/games/b.html").unwrap();

    // robots.txt, then a page sent 20 ms past its turn, then the next a whole delay after it.
    assert_eq!(site.arrivals_after(start), millis(&[0, 80, 140]));
}
