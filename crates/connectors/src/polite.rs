//! Reaching public websites as a well-behaved client: honoring each site's robots.txt and
//! leaving time between requests.

use std::{
    collections::HashMap,
    io::Read,
    sync::{Arc, Mutex, OnceLock, PoisonError},
    thread,
    time::{Duration, Instant},
};

use game_media_vault_application::{ApiKey, PortError};
use texting_robots::Robot;
use url::Url;

use crate::{Fetched, HttpTransport, Validators};

/// The name robots.txt rules address this application by, the product token of its requests.
pub const ROBOTS_USER_AGENT: &str = "game-media-vault";

/// The least time left between two requests to one site, unless its robots.txt asks for more.
pub const DEFAULT_SITE_DELAY: Duration = Duration::from_secs(1);

/// The longest pause between two requests to one site this application takes; a site asking for
/// more is left alone rather than read faster than it wants.
const MAX_SITE_DELAY: Duration = Duration::from_secs(30);

/// How much of a robots.txt is read; rules past it are ignored, as major crawlers do.
const MAX_ROBOTS_BYTES: u64 = 512 * 1024;

/// What each site asks of this application, and when it may next be asked something: shared by
/// every transport holding it, so that several connectors, or several executions in one process,
/// keep one pace per site.
pub struct SiteManners {
    delay: Duration,
    sites: Mutex<HashMap<String, Arc<Site>>>,
}

struct Site {
    /// What the site asks, once its robots.txt gave a definite answer. A request finding none
    /// reads it, which the other requests to the site wait for, and only they.
    asked: Mutex<Option<Arc<Asked>>>,
    /// When the next request to the site may leave.
    next: Mutex<Instant>,
}

struct Asked {
    /// Its rules, none when it has no robots.txt, or why the site is left alone.
    rules: Result<Option<Robot>, String>,
    delay: Duration,
}

impl SiteManners {
    pub fn new(delay: Duration) -> Self {
        Self {
            delay,
            sites: Mutex::new(HashMap::new()),
        }
    }

    /// The manners of this whole process, at the default pace.
    pub fn shared() -> Arc<Self> {
        static SHARED: OnceLock<Arc<SiteManners>> = OnceLock::new();
        Arc::clone(SHARED.get_or_init(|| Arc::new(Self::new(DEFAULT_SITE_DELAY))))
    }

    /// Waits for the turn of a request to `url`, once its site's robots.txt, read through
    /// `transport` the first time the site is asked something, allows it.
    fn admit(&self, url: &str, transport: &dyn HttpTransport) -> Result<(), PortError> {
        let location = Url::parse(url)
            .map_err(|error| PortError::new(format!("{url} is not a URL: {error}")))?;
        let origin = location.origin().ascii_serialization();
        let site = {
            let mut sites = self.sites.lock().unwrap_or_else(PoisonError::into_inner);
            Arc::clone(sites.entry(origin.clone()).or_insert_with(|| {
                Arc::new(Site {
                    asked: Mutex::new(None),
                    next: Mutex::new(Instant::now()),
                })
            }))
        };
        let asked = {
            let mut asked = site.asked.lock().unwrap_or_else(PoisonError::into_inner);
            match &*asked {
                Some(known) => Arc::clone(known),
                None => {
                    // A failure to read it is kept for no one: the next request reads it again.
                    let read = Arc::new(self.visit(&origin, transport)?);
                    // Reading robots.txt was a request too.
                    *site.next.lock().unwrap_or_else(PoisonError::into_inner) =
                        Instant::now() + read.delay;
                    *asked = Some(Arc::clone(&read));
                    read
                }
            }
        };
        match &asked.rules {
            Err(reason) => {
                return Err(PortError::new(format!("{origin} is left alone: {reason}")));
            }
            // The site does not want it: asking again cannot help.
            Ok(Some(robot)) if !robot.allowed(url) => {
                return Err(PortError::unavailable(format!(
                    "the robots.txt of {origin} disallows {url}"
                )));
            }
            Ok(_) => {}
        }
        let wait = {
            let mut next = site.next.lock().unwrap_or_else(PoisonError::into_inner);
            let now = Instant::now();
            let turn = (*next).max(now);
            *next = turn + asked.delay;
            turn - now
        };
        thread::sleep(wait);
        Ok(())
    }

    /// What `origin` asks of this application: its rules and the pause between requests, which is
    /// the longer of these manners' and its `Crawl-delay`; an error when its robots.txt could not
    /// be read this time.
    fn visit(&self, origin: &str, transport: &dyn HttpTransport) -> Result<Asked, PortError> {
        let mut rules = read_rules(origin, transport)?;
        let mut delay = self.delay;
        let asked = rules
            .as_ref()
            .ok()
            .and_then(Option::as_ref)
            .and_then(|robot| robot.delay)
            .map(f64::from);
        match asked {
            Some(seconds) if seconds.is_nan() || seconds > MAX_SITE_DELAY.as_secs_f64() => {
                rules = Err(format!(
                    "its robots.txt asks for {seconds} s between requests, more than the {} s this application waits at most",
                    MAX_SITE_DELAY.as_secs()
                ));
            }
            Some(seconds) if seconds > 0.0 => {
                delay = delay.max(Duration::from_secs_f64(seconds));
            }
            _ => {}
        }
        Ok(Asked { rules, delay })
    }
}

/// The robots.txt rules of `origin` for this application, from its first half mebibyte: none
/// when it has no robots.txt, or why it is left alone when its robots.txt makes no sense. An
/// error when it could not be read this time, which a later request may.
fn read_rules(
    origin: &str,
    transport: &dyn HttpTransport,
) -> Result<Result<Option<Robot>, String>, PortError> {
    let not_read = |reason: String| {
        PortError::new(format!(
            "the robots.txt of {origin} could not be read: {reason}"
        ))
    };
    match transport.get_stream(&format!("{origin}/robots.txt")) {
        Ok(stream) => {
            let mut body = Vec::new();
            stream
                .take(MAX_ROBOTS_BYTES)
                .read_to_end(&mut body)
                .map_err(|error| not_read(error.to_string()))?;
            Ok(Robot::new(ROBOTS_USER_AGENT, &body)
                .map(Some)
                .map_err(|error| format!("its robots.txt makes no sense: {error}")))
        }
        // No robots.txt: the site sets no rule.
        Err(error) if error.is_unavailable() => Ok(Ok(None)),
        Err(error) => Err(not_read(error.message().to_owned())),
    }
}

/// Requests a site's pages and media only as its robots.txt allows `game-media-vault`, read once
/// per site, and leaves at least the delay, or the site's longer `Crawl-delay`, between two
/// requests to it. A site without robots.txt allows everything; while its robots.txt cannot be
/// read, the site is asked nothing, and the next request reads it again. Each call is admitted
/// once, so the wrapped transport should send one
/// request per call, as `ReqwestHttpTransport::for_public_sites` does.
pub struct PoliteTransport<T> {
    inner: T,
    manners: Arc<SiteManners>,
}

impl<T> PoliteTransport<T> {
    /// Keeps the pace of this whole process.
    pub fn new(inner: T) -> Self {
        Self::with_manners(inner, SiteManners::shared())
    }

    /// Keeps a pace of its own, `delay` between requests at least.
    pub fn with_delay(inner: T, delay: Duration) -> Self {
        Self::with_manners(inner, Arc::new(SiteManners::new(delay)))
    }

    pub fn with_manners(inner: T, manners: Arc<SiteManners>) -> Self {
        Self { inner, manners }
    }
}

impl<T: HttpTransport> HttpTransport for PoliteTransport<T> {
    fn get_stream(&self, url: &str) -> Result<Box<dyn Read + Send>, PortError> {
        self.manners.admit(url, &self.inner)?;
        self.inner.get_stream(url)
    }

    fn get_if_changed(&self, url: &str, known: &Validators) -> Result<Fetched, PortError> {
        self.manners.admit(url, &self.inner)?;
        self.inner.get_if_changed(url, known)
    }

    fn get_authorized(&self, url: &str, api_key: &ApiKey) -> Result<Vec<u8>, PortError> {
        self.manners.admit(url, &self.inner)?;
        self.inner.get_authorized(url, api_key)
    }

    fn get_with_query_key(
        &self,
        url: &str,
        parameter: &str,
        api_key: &ApiKey,
    ) -> Result<Vec<u8>, PortError> {
        self.manners.admit(url, &self.inner)?;
        self.inner.get_with_query_key(url, parameter, api_key)
    }
}
