use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use serde::{Deserialize, Serialize};
use url::Url;

use crate::app_settings::{KNOWN_MEETING_PROVIDERS, MeetingPolicy, MeetingProviderChoice};
use crate::context::ContextSnapshot;
use crate::microphone_activity::ActiveMicrophoneApplication;

const STABLE_OBSERVATIONS: u8 = 4;
/// A browser using the microphone on a page that is not a known call site
/// must keep it for about 20 s: voice notes and dictation fields open the
/// microphone for a few seconds, calls hold it for minutes.
const BROWSER_STABLE_OBSERVATIONS: u8 = 40;
/// An offered call stays snoozed until its app has been off the microphone
/// for about a minute of 500 ms observations, so one call is offered once.
const ABSENT_OBSERVATIONS_TO_RESET: u8 = 120;
/// About three seconds without the call app on the microphone.
const CALL_ENDED_OBSERVATIONS: u8 = 6;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MeetingCandidate {
    pub key: String,
    pub title: String,
    pub source: MeetingSource,
    /// Canonical bundle identifier of the application using the microphone.
    #[serde(default)]
    pub application: Option<String>,
    /// Whether `title` names this meeting rather than its app.
    #[serde(default)]
    pub named: bool,
    /// Website host of the browser tab, when the browser adapter knows it.
    #[serde(default)]
    pub site: Option<String>,
    /// Display name of the application using the microphone.
    #[serde(default)]
    pub application_name: Option<String>,
}

fn known_provider_label(key: &str) -> Option<&'static str> {
    KNOWN_MEETING_PROVIDERS
        .iter()
        .find(|(known, _)| *known == key)
        .map(|(_, label)| *label)
}

/// A website or app whose calls share one Ask / Always record / Never offer
/// choice.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MeetingProvider {
    pub key: String,
    pub label: String,
}

impl MeetingProvider {
    pub fn choose(self, policy: MeetingPolicy) -> MeetingProviderChoice {
        MeetingProviderChoice {
            key: self.key,
            label: self.label,
            policy,
        }
    }
}

/// What a stable detected call leads to.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Detection {
    Offer(MeetingCandidate),
    /// The provider is set to Always record.
    Record(MeetingCandidate),
}

impl MeetingCandidate {
    pub fn manual() -> Self {
        Self {
            key: "manual".into(),
            title: MeetingSource::Manual.label().into(),
            source: MeetingSource::Manual,
            application: None,
            named: false,
            site: None,
            application_name: None,
        }
    }

    /// A manual recording, named `title` when one is given.
    pub fn manual_titled(title: Option<String>) -> Self {
        match title.map(|title| title.trim().to_owned()) {
            Some(title) if !title.is_empty() => Self {
                title,
                named: true,
                ..Self::manual()
            },
            _ => Self::manual(),
        }
    }

    pub fn recording_start(&self) -> crate::meeting::MeetingStart {
        crate::meeting::MeetingStart {
            title: self.named.then(|| self.title.clone()),
            app: self.app_label(),
            calendar: None,
        }
    }

    /// What the call is in: the service, else the website, else the browser.
    fn app_label(&self) -> Option<String> {
        match self.source {
            MeetingSource::Manual => None,
            MeetingSource::BrowserCall => {
                self.site.clone().or_else(|| self.application_name.clone())
            }
            source => Some(source.label().into()),
        }
    }

    /// The offer capsule's headline.
    pub fn offer_title(&self) -> String {
        if self.named {
            return self.title.clone();
        }
        match self.source {
            MeetingSource::BrowserCall => match (&self.site, &self.application_name) {
                (Some(site), _) => format!("Call on {site}"),
                (None, Some(application)) => format!("Call in {application}"),
                (None, None) => "Call detected".into(),
            },
            MeetingSource::SlackHuddle => "Slack huddle detected".into(),
            source => format!("{} call detected", source.label()),
        }
    }

    /// Whose call this is, for the user's Ask / Always record / Never offer
    /// choice: the website when known, else the app. Known services use the
    /// name Settings lists them under.
    pub fn provider(&self) -> Option<MeetingProvider> {
        let key = self.site.clone().or_else(|| self.application.clone())?;
        let label = known_provider_label(&key)
            .map(str::to_string)
            .or_else(|| self.site.clone())
            .or_else(|| self.application_name.clone())
            .unwrap_or_else(|| key.clone());
        Some(MeetingProvider { key, label })
    }

    /// Whether one click may set this provider to Always record: not for a
    /// browser whose page is unknown, which would record every page.
    pub fn can_always_record(&self) -> bool {
        self.provider().is_some()
            && !(self.source == MeetingSource::BrowserCall && self.site.is_none())
    }

    /// The user's choice for this call: the website's when it has one, else
    /// the app's (a browser's choice covers its other sites), else Ask.
    pub fn policy(&self, choices: &[MeetingProviderChoice]) -> MeetingPolicy {
        let chosen = |key: Option<&str>| {
            let key = key?;
            choices
                .iter()
                .find(|choice| choice.key == key)
                .map(|choice| choice.policy)
        };
        chosen(self.site.as_deref())
            .or_else(|| chosen(self.application.as_deref()))
            .unwrap_or_default()
    }

    fn required_observations(&self) -> u8 {
        match self.source {
            MeetingSource::BrowserCall => BROWSER_STABLE_OBSERVATIONS,
            _ => STABLE_OBSERVATIONS,
        }
    }
}

/// Notices when the call application behind a recording stops using the
/// microphone, so the user can choose to stop. It never stops anything.
#[derive(Default)]
pub struct CallPresence {
    absent: u8,
    prompted: bool,
}

impl CallPresence {
    /// Returns true once per absence that lasts long enough.
    pub fn observe(&mut self, present: bool) -> bool {
        if present {
            self.absent = 0;
            self.prompted = false;
            return false;
        }
        self.absent = self.absent.saturating_add(1);
        if self.absent >= CALL_ENDED_OBSERVATIONS && !self.prompted {
            self.prompted = true;
            return true;
        }
        false
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MeetingSource {
    Manual,
    GoogleMeet,
    Zoom,
    Teams,
    SlackHuddle,
    FaceTime,
    BrowserCall,
}

impl MeetingSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Manual => "Meeting",
            Self::GoogleMeet => "Google Meet",
            Self::Zoom => "Zoom",
            Self::Teams => "Microsoft Teams",
            Self::SlackHuddle => "Slack Huddle",
            Self::FaceTime => "FaceTime",
            Self::BrowserCall => "Browser call",
        }
    }
}

#[derive(Default)]
pub struct MeetingDetector {
    pending: Option<(String, u8)>,
    offered: Option<String>,
    offered_application: Option<String>,
    absent_observations: u8,
    calendar: Option<CalendarSnooze>,
}

/// A call the user already answered a calendar reminder for: recorded it,
/// or dismissed it.
#[derive(Clone, Debug, Eq, PartialEq)]
struct CalendarSnooze {
    /// App bundle IDs and website hosts of the meeting's call service;
    /// empty when the invitation had no call link, which covers the next
    /// call alone.
    keys: Vec<String>,
    /// When the snooze lapses, in wall-clock milliseconds: the meeting's
    /// end, or shortly after its start when it has no call link.
    until_ms: i64,
}

impl CalendarSnooze {
    fn covers(&self, candidate: &MeetingCandidate) -> bool {
        self.keys.is_empty()
            || [candidate.site.as_deref(), candidate.application.as_deref()]
                .into_iter()
                .flatten()
                .any(|key| {
                    self.keys
                        .iter()
                        .any(|known| crate::calendar::key_matches(key, known))
                })
    }
}

impl MeetingDetector {
    /// A calendar reminder was answered: its call, when detected before
    /// `until_ms`, is treated as already offered. `keys` names the call
    /// service; empty covers the next call detected.
    pub fn snooze_calendar_call(&mut self, keys: Vec<String>, until_ms: i64) {
        self.calendar = Some(CalendarSnooze { keys, until_ms });
    }

    pub fn is_application_snoozed(&self, canonical_bundle_id: &str) -> bool {
        self.offered_application.as_deref() == Some(canonical_bundle_id)
    }

    /// One observation under the user's choices. A Never offer call shows
    /// nothing but still counts as present; Always record ones pass the same
    /// count and snooze as an offer, so a stopped or dismissed call never
    /// starts again until its app leaves the microphone.
    pub fn detect(
        &mut self,
        candidate: Option<MeetingCandidate>,
        choices: &[MeetingProviderChoice],
    ) -> Option<Detection> {
        self.detect_at(candidate, choices, crate::calendar::wall_clock_ms())
    }

    fn detect_at(
        &mut self,
        candidate: Option<MeetingCandidate>,
        choices: &[MeetingProviderChoice],
        now_ms: i64,
    ) -> Option<Detection> {
        if candidate
            .as_ref()
            .is_some_and(|candidate| candidate.policy(choices) == MeetingPolicy::NeverOffer)
        {
            // Still on the microphone: an earlier offer stays snoozed.
            self.pending = None;
            self.absent_observations = 0;
            return None;
        }
        let candidate = self.observe_at(candidate, now_ms)?;
        Some(match candidate.policy(choices) {
            MeetingPolicy::AlwaysRecord => Detection::Record(candidate),
            _ => Detection::Offer(candidate),
        })
    }

    #[cfg(test)]
    fn observe(&mut self, candidate: Option<MeetingCandidate>) -> Option<MeetingCandidate> {
        self.observe_at(candidate, 0)
    }

    fn observe_at(
        &mut self,
        candidate: Option<MeetingCandidate>,
        now_ms: i64,
    ) -> Option<MeetingCandidate> {
        let Some(candidate) = candidate else {
            self.pending = None;
            self.absent_observations = self.absent_observations.saturating_add(1);
            if self.absent_observations >= ABSENT_OBSERVATIONS_TO_RESET {
                self.offered = None;
                self.offered_application = None;
            }
            return None;
        };
        self.absent_observations = 0;
        if self.offered.as_deref() == Some(&candidate.key)
            || (candidate.application.is_some()
                && self.offered_application == candidate.application)
        {
            self.pending = None;
            return None;
        }
        let observations = match &mut self.pending {
            Some((key, count)) if key == &candidate.key => {
                *count = count.saturating_add(1);
                *count
            }
            _ => {
                self.pending = Some((candidate.key.clone(), 1));
                1
            }
        };
        if observations < candidate.required_observations() {
            return None;
        }
        self.pending = None;
        self.offered = Some(candidate.key.clone());
        self.offered_application = candidate.application.clone();
        if self
            .calendar
            .as_ref()
            .is_some_and(|snooze| now_ms >= snooze.until_ms)
        {
            self.calendar = None;
        }
        if let Some(snooze) = &self.calendar
            && snooze.covers(&candidate)
        {
            tracing::info!(
                source = candidate.source.label(),
                "detected call already answered from its calendar reminder"
            );
            if snooze.keys.is_empty() {
                self.calendar = None;
            }
            return None;
        }
        Some(candidate)
    }
}

pub fn candidate_from_context(context: &ContextSnapshot) -> Option<MeetingCandidate> {
    let application = context.application.as_deref()?;
    let application_lower = application.to_ascii_lowercase();
    let title = context.window_title.as_deref().unwrap_or("").trim();
    let title_lower = title.to_ascii_lowercase();

    if is_browser_application(&application_lower)
        && let Some(url) = &context.browser_url
        && is_google_meet_call(url)
    {
        return Some(named(candidate(
            MeetingSource::GoogleMeet,
            url.as_str(),
            &clean_google_meet_title(title),
        )));
    }
    if application_lower.contains("zoom")
        && (title_lower.contains("meeting") || title_lower.contains("webinar"))
    {
        return Some(candidate(MeetingSource::Zoom, title, title));
    }
    if application_lower.contains("teams")
        && (title_lower.contains("meeting") || title_lower.contains("call"))
    {
        return Some(candidate(MeetingSource::Teams, title, title));
    }
    if application_lower == "slack" && title_lower.contains("huddle") {
        return Some(candidate(MeetingSource::SlackHuddle, title, title));
    }
    if application_lower == "facetime" && !title.is_empty() && title_lower != "facetime" {
        return Some(named(candidate(MeetingSource::FaceTime, title, title)));
    }
    None
}

fn named(mut candidate: MeetingCandidate) -> MeetingCandidate {
    candidate.named = candidate.title != candidate.source.label();
    candidate
}

pub fn candidate_from_microphone(
    application: &ActiveMicrophoneApplication,
    context: &ContextSnapshot,
) -> Option<MeetingCandidate> {
    let canonical_bundle = application.canonical_bundle_id()?;
    let application_name = crate::microphone_activity::canonical_application_name(canonical_bundle)
        .map(str::to_string)
        .unwrap_or_else(|| application.name.clone());
    if application.is_browser() {
        let matches = context_matches_browser(canonical_bundle, &application.name, context);
        let site = matches
            .then(|| context.browser_host())
            .flatten()
            .map(|host| {
                host.strip_prefix("www.")
                    .unwrap_or(host)
                    .to_ascii_lowercase()
            });
        if matches && let Some(mut candidate) = candidate_from_context(context) {
            candidate.application = Some(canonical_bundle.into());
            candidate.application_name = Some(application_name);
            candidate.site = site;
            return Some(candidate);
        }
        let mut candidate = candidate(MeetingSource::BrowserCall, canonical_bundle, "");
        candidate.application = Some(canonical_bundle.into());
        candidate.application_name = Some(application_name);
        candidate.site = site;
        return Some(candidate);
    }
    let (source, title) = match canonical_bundle {
        "us.zoom.xos" => (MeetingSource::Zoom, "Zoom meeting"),
        "com.microsoft.teams2" => (MeetingSource::Teams, "Microsoft Teams meeting"),
        "com.tinyspeck.slackmacgap" => (MeetingSource::SlackHuddle, "Slack huddle"),
        "com.apple.FaceTime" => (MeetingSource::FaceTime, "FaceTime call"),
        _ => return None,
    };
    let mut candidate = candidate(source, canonical_bundle, title);
    candidate.application = Some(canonical_bundle.into());
    candidate.application_name = Some(application_name);
    Some(candidate)
}

fn is_browser_application(application_lower: &str) -> bool {
    ["brave", "chrome", "safari", "firefox", "edge"]
        .iter()
        .any(|browser| application_lower.contains(browser))
}

fn context_matches_browser(
    canonical_bundle: &str,
    application_name: &str,
    context: &ContextSnapshot,
) -> bool {
    let Some(foreground) = context.application.as_deref() else {
        return false;
    };
    if foreground.contains(application_name) || application_name.contains(foreground) {
        return true;
    }
    let lower = foreground.to_ascii_lowercase();
    match canonical_bundle {
        "com.brave.Browser" => lower.contains("brave"),
        "com.google.Chrome" => lower.contains("chrome"),
        "com.apple.Safari" => lower.contains("safari"),
        "org.mozilla.firefox" => lower.contains("firefox"),
        "com.microsoft.edgemac" => lower.contains("edge"),
        _ => false,
    }
}

fn candidate(source: MeetingSource, identity: &str, title: &str) -> MeetingCandidate {
    MeetingCandidate {
        key: opaque_key(source, identity),
        title: if title.is_empty() {
            source.label().into()
        } else {
            title.into()
        },
        source,
        application: None,
        named: false,
        site: None,
        application_name: None,
    }
}

fn opaque_key(source: MeetingSource, identity: &str) -> String {
    let mut hasher = DefaultHasher::new();
    source.hash(&mut hasher);
    identity.hash(&mut hasher);
    format!("meeting-{:016x}", hasher.finish())
}

fn is_google_meet_call(url: &Url) -> bool {
    if url.host_str() != Some("meet.google.com") {
        return false;
    }
    let path = url.path().trim_matches('/');
    !path.is_empty() && path != "landing" && path != "new"
}

fn clean_google_meet_title(title: &str) -> String {
    let mut cleaned = clean_title(
        title,
        &[
            " - Audio playing - Brave",
            " - Brave",
            " - Brave Browser",
            " - Google Chrome",
            " — Mozilla Firefox",
            " - Microsoft Edge",
            " - Safari",
        ],
    );
    cleaned = clean_title(cleaned, &[" - Google Meet", " | Google Meet"]);
    for prefix in ["Meet - ", "Meet – ", "Meet — "] {
        if let Some(rest) = cleaned.strip_prefix(prefix) {
            cleaned = rest.trim();
            break;
        }
    }
    if cleaned.eq_ignore_ascii_case("Google Meet")
        || cleaned.eq_ignore_ascii_case("Meet")
        || is_google_meet_code(cleaned)
    {
        return String::new();
    }
    cleaned.to_string()
}

fn is_google_meet_code(value: &str) -> bool {
    let mut parts = value.split('-');
    let (Some(first), Some(second), Some(third), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    first.len() == 3
        && second.len() == 4
        && third.len() == 3
        && [first, second, third]
            .into_iter()
            .all(|part| part.bytes().all(|byte| byte.is_ascii_lowercase()))
}

fn clean_title<'a>(title: &'a str, suffixes: &[&str]) -> &'a str {
    suffixes
        .iter()
        .find_map(|suffix| title.strip_suffix(suffix))
        .unwrap_or(title)
        .trim()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context(application: &str, url: Option<&str>, title: &str) -> ContextSnapshot {
        ContextSnapshot {
            application: Some(application.into()),
            browser_url: url.map(|url| Url::parse(url).unwrap()),
            window_title: Some(title.into()),
            selected_text: None,
            input_revision: None,
        }
    }

    #[test]
    fn recognizes_calls_without_treating_landing_pages_as_meetings() {
        assert!(
            candidate_from_context(&context(
                "Brave Browser",
                Some("https://meet.google.com/abc-defg-hij"),
                "Design sync - Google Meet"
            ))
            .is_some()
        );
        assert!(
            candidate_from_context(&context(
                "Brave Browser",
                Some("https://meet.google.com/landing"),
                "Google Meet"
            ))
            .is_none()
        );
        assert!(candidate_from_context(&context("zoom.us", None, "Zoom Workplace")).is_none());
        assert!(candidate_from_context(&context("zoom.us", None, "Zoom Meeting")).is_some());
    }

    #[test]
    fn requires_stability_and_offers_once_per_session() {
        let candidate = candidate_from_context(&context(
            "Brave Browser",
            Some("https://meet.google.com/abc-defg-hij"),
            "Design sync - Google Meet",
        ));
        let mut detector = MeetingDetector::default();
        assert!(detector.observe(candidate.clone()).is_none());
        assert!(detector.observe(candidate.clone()).is_none());
        assert!(detector.observe(candidate.clone()).is_none());
        assert!(detector.observe(candidate.clone()).is_some());
        assert!(detector.observe(candidate.clone()).is_none());
        for _ in 0..ABSENT_OBSERVATIONS_TO_RESET {
            assert!(detector.observe(None).is_none());
        }
        assert!(detector.observe(candidate.clone()).is_none());
        assert!(detector.observe(candidate.clone()).is_none());
        assert!(detector.observe(candidate.clone()).is_none());
        assert!(detector.observe(candidate).is_some());
    }

    #[test]
    fn a_brief_microphone_release_does_not_re_offer_the_same_call() {
        let candidate = candidate(MeetingSource::Zoom, "us.zoom.xos", "Zoom meeting");
        let mut detector = MeetingDetector::default();
        for _ in 0..STABLE_OBSERVATIONS - 1 {
            assert!(detector.observe(Some(candidate.clone())).is_none());
        }
        assert!(detector.observe(Some(candidate.clone())).is_some());
        // Dismissed or ignored: twenty seconds off the microphone keeps it snoozed.
        for _ in 0..40 {
            assert!(detector.observe(None).is_none());
        }
        for _ in 0..10 {
            assert!(detector.observe(Some(candidate.clone())).is_none());
        }
    }

    #[test]
    fn named_meetings_keep_their_title_and_app_calls_use_the_app() {
        let meet = candidate_from_context(&context(
            "Brave Browser",
            Some("https://meet.google.com/abc-defg-hij"),
            "Design sync - Google Meet",
        ))
        .unwrap();
        let start = meet.recording_start();
        assert_eq!(start.title.as_deref(), Some("Design sync"));
        assert_eq!(start.app.as_deref(), Some("Google Meet"));

        let zoom = candidate(MeetingSource::Zoom, "us.zoom.xos", "Zoom meeting").recording_start();
        assert_eq!(zoom.title, None);
        assert_eq!(zoom.app.as_deref(), Some("Zoom"));

        let manual = MeetingCandidate::manual().recording_start();
        assert_eq!((manual.title, manual.app), (None, None));
    }

    #[test]
    fn call_end_is_reported_once_per_sustained_absence() {
        let mut presence = CallPresence::default();
        assert!(!presence.observe(true));
        for _ in 0..CALL_ENDED_OBSERVATIONS - 1 {
            assert!(!presence.observe(false));
        }
        assert!(presence.observe(false));
        assert!(!presence.observe(false));
        // Rejoining and leaving again asks again.
        assert!(!presence.observe(true));
        for _ in 0..CALL_ENDED_OBSERVATIONS - 1 {
            assert!(!presence.observe(false));
        }
        assert!(presence.observe(false));
    }

    #[test]
    fn classifies_helper_and_host_microphone_applications() {
        let brave_helper = ActiveMicrophoneApplication {
            pid: 2352,
            bundle_id: "com.brave.Browser.helper".into(),
            name: "Brave Browser Helper".into(),
        };
        let meet_context = context(
            "Brave Browser",
            Some("https://meet.google.com/abc-defg-hij"),
            "Meet - Design sync - Brave",
        );
        let meet = candidate_from_microphone(&brave_helper, &meet_context).unwrap();
        assert_eq!(meet.source, MeetingSource::GoogleMeet);
        assert_eq!(meet.title, "Design sync");
        assert!(meet.named);
        assert_eq!(meet.application.as_deref(), Some("com.brave.Browser"));

        let slack_helper = ActiveMicrophoneApplication {
            pid: 31807,
            bundle_id: "com.tinyspeck.slackmacgap.helper".into(),
            name: "Slack Helper".into(),
        };
        let huddle = candidate_from_microphone(&slack_helper, &ContextSnapshot::default()).unwrap();
        assert_eq!(huddle.source, MeetingSource::SlackHuddle);
        assert_eq!(
            huddle.application.as_deref(),
            Some("com.tinyspeck.slackmacgap")
        );

        let zoom_helper = ActiveMicrophoneApplication {
            pid: 9010,
            bundle_id: "us.zoom.CptHost".into(),
            name: "CptHost".into(),
        };
        let zoom = candidate_from_microphone(&zoom_helper, &ContextSnapshot::default()).unwrap();
        assert_eq!(zoom.source, MeetingSource::Zoom);
        assert_eq!(zoom.application.as_deref(), Some("us.zoom.xos"));

        let safari_gpu = ActiveMicrophoneApplication {
            pid: 7100,
            bundle_id: "com.apple.WebKit.GPU".into(),
            name: "Safari Graphics and Media".into(),
        };
        let browser = candidate_from_microphone(&safari_gpu, &ContextSnapshot::default()).unwrap();
        assert_eq!(browser.source, MeetingSource::BrowserCall);
        assert_eq!(browser.application.as_deref(), Some("com.apple.Safari"));
    }

    #[test]
    fn cleans_google_meet_titles_and_ignores_raw_meet_codes() {
        let code_only = candidate_from_context(&context(
            "Brave Browser",
            Some("https://meet.google.com/abc-defg-hij"),
            "Meet - abc-defg-hij - Brave",
        ))
        .unwrap();
        assert_eq!(code_only.source, MeetingSource::GoogleMeet);
        assert_eq!(code_only.title, "Google Meet");
        assert!(!code_only.named);
        assert_eq!(code_only.recording_start().title, None);

        let named_prefix = candidate_from_context(&context(
            "Google Chrome",
            Some("https://meet.google.com/abc-defg-hij"),
            "Meet – Weekly Architecture - Google Chrome",
        ))
        .unwrap();
        assert_eq!(named_prefix.title, "Weekly Architecture");
        assert!(named_prefix.named);
    }

    #[test]
    fn switching_tabs_during_a_snoozed_browser_call_does_not_re_offer() {
        let brave = ActiveMicrophoneApplication {
            pid: 1592,
            bundle_id: "com.brave.Browser".into(),
            name: "Brave Browser".into(),
        };
        let meet_context = context(
            "Brave Browser",
            Some("https://meet.google.com/abc-defg-hij"),
            "Design sync - Google Meet",
        );
        let meet = candidate_from_microphone(&brave, &meet_context).unwrap();
        let fallback = candidate_from_microphone(&brave, &ContextSnapshot::default()).unwrap();

        let mut detector = MeetingDetector::default();
        for _ in 0..STABLE_OBSERVATIONS - 1 {
            assert!(detector.observe(Some(meet.clone())).is_none());
        }
        assert!(detector.observe(Some(meet)).is_some());
        assert!(detector.is_application_snoozed("com.brave.Browser"));
        for _ in 0..STABLE_OBSERVATIONS + 2 {
            assert!(detector.observe(Some(fallback.clone())).is_none());
        }
    }

    fn brave() -> ActiveMicrophoneApplication {
        ActiveMicrophoneApplication {
            pid: 49686,
            bundle_id: "com.brave.Browser".into(),
            name: "Brave Browser".into(),
        }
    }

    #[test]
    fn offer_titles_name_the_service_site_or_browser_without_repeating_call() {
        let problems = context(
            "Brave Browser",
            Some("https://www.problems.kitlangton.com/board"),
            "Problems",
        );
        let site = candidate_from_microphone(&brave(), &problems).unwrap();
        assert_eq!(site.source, MeetingSource::BrowserCall);
        assert_eq!(site.offer_title(), "Call on problems.kitlangton.com");
        assert_eq!(
            site.recording_start().app.as_deref(),
            Some("problems.kitlangton.com")
        );

        let unknown = candidate_from_microphone(&brave(), &ContextSnapshot::default()).unwrap();
        assert_eq!(unknown.offer_title(), "Call in Brave Browser");

        let meet = candidate_from_microphone(
            &brave(),
            &context(
                "Brave Browser",
                Some("https://meet.google.com/abc-defg-hij"),
                "Meet - abc-defg-hij - Brave",
            ),
        )
        .unwrap();
        assert_eq!(meet.offer_title(), "Google Meet call detected");
        assert_eq!(
            candidate(MeetingSource::SlackHuddle, "slack", "Slack huddle").offer_title(),
            "Slack huddle detected"
        );
    }

    fn choice(key: &str, policy: MeetingPolicy) -> MeetingProviderChoice {
        MeetingProviderChoice {
            key: key.into(),
            label: key.into(),
            policy,
        }
    }

    #[test]
    fn choices_apply_to_the_site_when_known_and_the_app_otherwise() {
        let problems = context(
            "Brave Browser",
            Some("https://problems.kitlangton.com/"),
            "Problems",
        );
        let site = candidate_from_microphone(&brave(), &problems).unwrap();
        let provider = site.provider().unwrap();
        assert_eq!(provider.key, "problems.kitlangton.com");
        assert_eq!(
            site.policy(&[provider.choose(MeetingPolicy::NeverOffer)]),
            MeetingPolicy::NeverOffer
        );
        assert!(site.can_always_record());

        let unknown = candidate_from_microphone(&brave(), &ContextSnapshot::default()).unwrap();
        let app = unknown.provider().unwrap();
        assert_eq!(
            (app.key.as_str(), app.label.as_str()),
            ("com.brave.Browser", "Brave Browser")
        );
        assert!(!unknown.can_always_record());
        let never_site = [choice("problems.kitlangton.com", MeetingPolicy::NeverOffer)];
        assert_eq!(unknown.policy(&never_site), MeetingPolicy::Ask);
        // A browser's choice covers its sites unless the site has its own.
        let never_brave = choice("com.brave.Browser", MeetingPolicy::NeverOffer);
        assert_eq!(
            site.policy(std::slice::from_ref(&never_brave)),
            MeetingPolicy::NeverOffer
        );
        assert_eq!(
            site.policy(&[
                never_brave.clone(),
                choice("problems.kitlangton.com", MeetingPolicy::AlwaysRecord)
            ]),
            MeetingPolicy::AlwaysRecord
        );
        // A site set back to Ask asks, whatever its browser's choice.
        assert_eq!(
            site.policy(&[
                never_brave,
                choice("problems.kitlangton.com", MeetingPolicy::Ask)
            ]),
            MeetingPolicy::Ask
        );

        // Known services carry the name Settings lists them under.
        let meet = candidate_from_microphone(
            &brave(),
            &context(
                "Brave Browser",
                Some("https://meet.google.com/abc-defg-hij"),
                "Design sync - Google Meet",
            ),
        )
        .unwrap();
        let provider = meet.provider().unwrap();
        assert_eq!(
            (provider.key.as_str(), provider.label.as_str()),
            ("meet.google.com", "Google Meet")
        );
        let slack = slack_huddle();
        assert_eq!(slack.provider().unwrap().label, "Slack huddles");
        assert!(slack.can_always_record());
    }

    fn slack_huddle() -> MeetingCandidate {
        let helper = ActiveMicrophoneApplication {
            pid: 31807,
            bundle_id: "com.tinyspeck.slackmacgap.helper".into(),
            name: "Slack Helper".into(),
        };
        candidate_from_microphone(&helper, &ContextSnapshot::default()).unwrap()
    }

    #[test]
    fn always_record_starts_after_the_same_stability_count_as_an_offer() {
        let slack = slack_huddle();
        let always = [choice(
            "com.tinyspeck.slackmacgap",
            MeetingPolicy::AlwaysRecord,
        )];
        let mut detector = MeetingDetector::default();
        for _ in 0..STABLE_OBSERVATIONS - 1 {
            assert_eq!(detector.detect(Some(slack.clone()), &always), None);
        }
        assert_eq!(
            detector.detect(Some(slack.clone()), &always),
            Some(Detection::Record(slack.clone()))
        );
        // Stopped mid-call: the call stays snoozed while it uses the microphone.
        for _ in 0..20 {
            assert_eq!(detector.detect(Some(slack.clone()), &always), None);
        }

        // Browser pages still need about 20 s on the microphone.
        let problems = candidate_from_microphone(
            &brave(),
            &context(
                "Brave Browser",
                Some("https://problems.kitlangton.com/"),
                "Problems",
            ),
        )
        .unwrap();
        let always = [choice(
            "problems.kitlangton.com",
            MeetingPolicy::AlwaysRecord,
        )];
        let mut detector = MeetingDetector::default();
        for _ in 0..12 {
            assert_eq!(detector.detect(Some(problems.clone()), &always), None);
        }
        assert_eq!(detector.detect(None, &always), None);
        for _ in 0..BROWSER_STABLE_OBSERVATIONS - 1 {
            assert_eq!(detector.detect(Some(problems.clone()), &always), None);
        }
        assert_eq!(
            detector.detect(Some(problems.clone()), &always),
            Some(Detection::Record(problems))
        );
    }

    #[test]
    fn never_offer_and_snoozed_calls_never_start() {
        let slack = slack_huddle();
        let never = [choice(
            "com.tinyspeck.slackmacgap",
            MeetingPolicy::NeverOffer,
        )];
        let mut detector = MeetingDetector::default();
        for _ in 0..STABLE_OBSERVATIONS * 4 {
            assert_eq!(detector.detect(Some(slack.clone()), &never), None);
        }

        // Offered and dismissed, then switched to Always record mid-call.
        let mut detector = MeetingDetector::default();
        for _ in 0..STABLE_OBSERVATIONS - 1 {
            assert_eq!(detector.detect(Some(slack.clone()), &[]), None);
        }
        assert_eq!(
            detector.detect(Some(slack.clone()), &[]),
            Some(Detection::Offer(slack.clone()))
        );
        let always = [choice(
            "com.tinyspeck.slackmacgap",
            MeetingPolicy::AlwaysRecord,
        )];
        for _ in 0..40 {
            assert_eq!(detector.detect(None, &always), None);
        }
        for _ in 0..STABLE_OBSERVATIONS * 4 {
            assert_eq!(detector.detect(Some(slack.clone()), &always), None);
        }
        // Never offer, then Always record, while the call stays on the
        // microphone well past the snooze reset: still nothing.
        let never = [choice(
            "com.tinyspeck.slackmacgap",
            MeetingPolicy::NeverOffer,
        )];
        for _ in 0..ABSENT_OBSERVATIONS_TO_RESET as usize * 2 {
            assert_eq!(detector.detect(Some(slack.clone()), &never), None);
        }
        for _ in 0..STABLE_OBSERVATIONS * 4 {
            assert_eq!(detector.detect(Some(slack.clone()), &always), None);
        }
        // The next call, after a minute off the microphone, records.
        for _ in 0..ABSENT_OBSERVATIONS_TO_RESET {
            assert_eq!(detector.detect(None, &always), None);
        }
        for _ in 0..STABLE_OBSERVATIONS - 1 {
            assert_eq!(detector.detect(Some(slack.clone()), &always), None);
        }
        assert_eq!(
            detector.detect(Some(slack.clone()), &always),
            Some(Detection::Record(slack))
        );
    }

    #[test]
    fn a_browser_voice_note_is_not_offered_but_a_sustained_browser_call_is() {
        let problems = context(
            "Brave Browser",
            Some("https://problems.kitlangton.com/"),
            "Problems",
        );
        let candidate = candidate_from_microphone(&brave(), &problems).unwrap();
        let mut detector = MeetingDetector::default();
        // A six-second voice note: 12 observations, then the mic closes.
        for _ in 0..12 {
            assert!(detector.observe(Some(candidate.clone())).is_none());
        }
        assert!(detector.observe(None).is_none());
        for _ in 0..BROWSER_STABLE_OBSERVATIONS - 1 {
            assert!(detector.observe(Some(candidate.clone())).is_none());
        }
        assert!(detector.observe(Some(candidate)).is_some());
    }

    /// Observes `candidate` until it is stable; returns what the last
    /// observation detected.
    fn stable(
        detector: &mut MeetingDetector,
        candidate: &MeetingCandidate,
        choices: &[MeetingProviderChoice],
        now_ms: i64,
    ) -> Option<Detection> {
        for _ in 0..candidate.required_observations() - 1 {
            assert_eq!(
                detector.detect_at(Some(candidate.clone()), choices, now_ms),
                None
            );
        }
        detector.detect_at(Some(candidate.clone()), choices, now_ms)
    }

    fn leave(detector: &mut MeetingDetector, now_ms: i64) {
        for _ in 0..ABSENT_OBSERVATIONS_TO_RESET {
            assert_eq!(detector.detect_at(None, &[], now_ms), None);
        }
    }

    #[test]
    fn a_call_answered_from_its_calendar_reminder_is_not_offered_again() {
        let zoom = candidate(MeetingSource::Zoom, "us.zoom.xos", "Zoom meeting");
        let zoom = MeetingCandidate {
            application: Some("us.zoom.xos".into()),
            ..zoom
        };
        let zoom_keys = crate::calendar::CallProvider::Zoom
            .detection_keys()
            .iter()
            .map(|key| key.to_string())
            .collect::<Vec<_>>();
        let mut detector = MeetingDetector::default();
        detector.snooze_calendar_call(zoom_keys.clone(), 1_000);
        assert_eq!(stable(&mut detector, &zoom, &[], 10), None);
        // Rejoining the same meeting later is still covered.
        leave(&mut detector, 20);
        assert_eq!(stable(&mut detector, &zoom, &[], 30), None);
        // Always record doesn't restart a call the user already stopped.
        leave(&mut detector, 40);
        let always = [choice("us.zoom.xos", MeetingPolicy::AlwaysRecord)];
        assert_eq!(stable(&mut detector, &zoom, &always, 50), None);
        // Another service's call is offered as usual.
        let slack = slack_huddle();
        leave(&mut detector, 60);
        assert_eq!(
            stable(&mut detector, &slack, &[], 70),
            Some(Detection::Offer(slack.clone()))
        );
        // Once the meeting ends, its service's calls are offered again.
        leave(&mut detector, 2_000);
        assert_eq!(
            stable(&mut detector, &zoom, &[], 2_000),
            Some(Detection::Offer(zoom.clone()))
        );

        // A Meet link covers the site in any browser.
        let meet = candidate_from_microphone(
            &brave(),
            &context(
                "Brave Browser",
                Some("https://meet.google.com/abc-defg-hij"),
                "Design sync - Google Meet",
            ),
        )
        .unwrap();
        let mut detector = MeetingDetector::default();
        detector.snooze_calendar_call(vec!["meet.google.com".into()], 1_000);
        assert_eq!(stable(&mut detector, &meet, &[], 10), None);
    }

    #[test]
    fn a_reminder_without_a_call_link_covers_only_the_next_call() {
        let slack = slack_huddle();
        let mut detector = MeetingDetector::default();
        detector.snooze_calendar_call(Vec::new(), 1_000);
        assert_eq!(stable(&mut detector, &slack, &[], 10), None);
        leave(&mut detector, 20);
        assert_eq!(
            stable(&mut detector, &slack, &[], 30),
            Some(Detection::Offer(slack))
        );
    }
}
