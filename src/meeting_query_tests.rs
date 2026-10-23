use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::*;

const NOW: u64 = 1_790_100_000_000;
const WEEKLY: &str = "1790000000000-1001";
const ROADMAP: &str = "1790000500000-1002";
const STANDUP: &str = "1790001000000-1003";

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/meetings")
}

fn library() -> Library {
    Library::new(fixtures())
}

fn list(library: &Library, include_in_progress: bool) -> String {
    library
        .list(
            &ListQuery {
                since: None,
                until: None,
                query: None,
                limit: 20,
                include_in_progress,
            },
            NOW,
        )
        .unwrap()
}

fn search(query: &str, fuzzy: bool) -> String {
    library()
        .search(
            &SearchQuery {
                query: query.into(),
                since: None,
                until: None,
                meeting_id: None,
                limit: 10,
                fuzzy,
                include_in_progress: true,
            },
            NOW,
        )
        .unwrap()
}

fn transcript(id: &str, configure: impl FnOnce(&mut TranscriptQuery)) -> String {
    let mut query = TranscriptQuery {
        meeting_id: id.into(),
        start: None,
        end: None,
        speaker: None,
        max_chars: 20_000,
        cursor: None,
    };
    configure(&mut query);
    library().transcript(&query, NOW).unwrap()
}

fn hit_lines(output: &str) -> Vec<&str> {
    output
        .lines()
        .filter(|line| line.starts_with('['))
        .collect()
}

#[test]
fn lists_meetings_skipping_unreadable_folders_and_labeling_unfinished_ones() {
    let output = list(&library(), true);
    let lines = output.lines().collect::<Vec<_>>();
    assert!(lines[0].starts_with("3 of 3 meetings"), "{output}");
    // Newest first.
    assert!(lines[1].starts_with(&format!("- {STANDUP}")), "{output}");
    assert!(lines[1].contains("interrupted · rolling transcript"));
    let weekly = lines.iter().find(|line| line.contains(WEEKLY)).unwrap();
    assert!(weekly.contains("\"Weekly sync\""));
    assert!(weekly.contains("· 00:40 · complete · final transcript"));
    assert!(weekly.contains("participants: You, Them (Zoom)"));
    assert!(weekly.contains("notes: yes"));
    assert!(output.contains("warning: skipped 1 unreadable lines in transcript.ndjson"));
    assert!(output.contains(
        "Skipped 1 unreadable folders:\n- 1790001500000-1004: could not decode manifest.json"
    ));
}

#[test]
fn list_filters_by_title_and_time() {
    let library = library();
    let query = |since: Option<&str>, until: Option<&str>, title: Option<&str>| {
        library
            .list(
                &ListQuery {
                    since: since.map(Into::into),
                    until: until.map(Into::into),
                    query: title.map(Into::into),
                    limit: 20,
                    include_in_progress: true,
                },
                NOW,
            )
            .unwrap()
    };
    let roadmap = query(None, None, Some("ROADMAP"));
    assert!(
        roadmap.starts_with("1 of 1 meetings") && roadmap.contains(ROADMAP),
        "{roadmap}"
    );
    let after = query(Some("1790000500000"), None, None);
    assert!(
        !after.contains(WEEKLY) && after.contains(ROADMAP),
        "{after}"
    );
    let before = query(None, Some("2026-09-21T00:00:00Z"), None);
    assert!(before.starts_with("No meetings match"), "{before}");
}

#[test]
fn search_ranks_names_and_tolerates_recognition_misspellings() {
    let output = search("Dax", true);
    let hits = hit_lines(&output);
    assert!(hits[0].contains(WEEKLY), "{output}");
    assert!(
        output.contains("hit: Sure. **Dax**, can you walk"),
        "{output}"
    );
    assert!(
        hits.iter().any(|line| line.contains("matched dack~dax")),
        "Dacks should match Dax: {output}"
    );
    let exact_rank = hits
        .iter()
        .position(|line| line.contains("matched dax"))
        .unwrap();
    let fuzzy_rank = hits
        .iter()
        .position(|line| line.contains("dack~dax"))
        .unwrap();
    assert!(exact_rank < fuzzy_rank, "{output}");

    let exact = search("Dacks", false);
    assert_eq!(hit_lines(&exact).len(), 1, "{exact}");
    assert!(exact.contains("**Dacks** thinks"));
}

#[test]
fn search_stems_folds_accents_and_requires_quoted_phrases() {
    let schedule = search("schedule", true);
    assert!(
        schedule.contains(WEEKLY) && schedule.contains(ROADMAP),
        "{schedule}"
    );
    assert!(
        schedule.contains("**schedule**, like a cron job"),
        "{schedule}"
    );

    let cafe = search("cafe", true);
    assert!(cafe.contains("the **Café** dashboard"), "{cafe}");

    let phrase = search("\"scheduled sessions\"", true);
    assert!(!phrase.contains(ROADMAP), "{phrase}");
    let hits = hit_lines(&phrase);
    assert!(hits.iter().any(|line| line.contains("Notes")), "{phrase}");
    assert!(
        phrase.contains("the **scheduled** **sessions** idea"),
        "{phrase}"
    );
    assert!(hits.iter().all(|line| line.contains(WEEKLY)), "{phrase}");

    let missing = search("\"sessions scheduled\"", true);
    assert!(missing.starts_with("0 matching passages"), "{missing}");
}

#[test]
fn search_reports_context_and_labels_unfinished_meetings() {
    let output = search("cron", true);
    assert!(
        output.contains(
            "before (You 00:04): Sure. Dax, can you walk us through the scheduled sessions idea?"
        ),
        "{output}"
    );
    assert!(
        output.contains("interrupted (rolling transcript)"),
        "{output}"
    );
    assert!(
        output.contains("Rolling text mentions **cron** again."),
        "{output}"
    );
}

#[test]
fn transcript_merges_turns_and_continues_from_a_cursor() {
    let full = transcript(WEEKLY, |_| {});
    assert!(full.contains(
        "] Them: Yeah, so the plan is that agents can run on a schedule, like a cron job, \
without anyone watching. Recurring sessions would wake up in the background and report back."
    ));
    assert!(full.ends_with("(end of range)\n"), "{full}");

    let first = transcript(WEEKLY, |query| query.max_chars = 500);
    assert!(!first.contains("Great, I'll write the spec."));
    let cursor = first
        .lines()
        .find_map(|line| line.strip_prefix("next_cursor: "))
        .and_then(|rest| rest.split_whitespace().next())
        .unwrap()
        .to_string();
    let second = transcript(WEEKLY, |query| {
        query.max_chars = 500;
        query.cursor = Some(cursor);
    });
    assert!(second.contains("Great, I'll write the spec."), "{second}");
    assert!(!second.contains("thanks for joining"), "{second}");

    let you = transcript(WEEKLY, |query| query.speaker = Some("you".into()));
    assert!(
        !you.contains("Them:") && you.contains("You: Sure. Dax"),
        "{you}"
    );
    let window = transcript(WEEKLY, |query| {
        query.start = Some("0:20".into());
        query.end = Some("30".into());
    });
    assert!(window.contains("Showing 00:16–00:31"), "{window}");
    assert!(!window.contains("thanks for joining") && !window.contains("write the spec"));
}

#[test]
fn transcript_of_an_unfinished_meeting_is_labeled_as_a_draft() {
    let output = transcript(STANDUP, |_| {});
    assert!(output.contains("No final transcript was published; this is the unfinished draft."));
    assert!(
        output.contains("Them: Rolling text about the background worker."),
        "{output}"
    );
    assert!(output.contains("You: I agree with that."), "{output}");
}

#[test]
fn notes_are_read_and_their_absence_is_explicit() {
    // An older manifest with action items and decisions reads as title and summary.
    let notes = library().notes(WEEKLY).unwrap();
    assert!(notes.contains("Title: Scheduled Sessions Plan"), "{notes}");
    assert!(notes.contains("Summary: Dax proposed scheduled sessions for agents."));
    assert!(!notes.contains("Prototype the runner") && !notes.contains("Decisions"));
    assert!(notes.contains("Read the transcript with get_transcript"));

    let missing = library().notes(ROADMAP).unwrap();
    assert!(missing.contains("No Apple Intelligence notes are stored for this meeting."));
    assert!(missing.contains("Last notes attempt failed: Apple Intelligence is not available"));
    assert!(missing.contains("Use get_transcript"));

    let unknown = library().notes("nope").unwrap_err().to_string();
    assert!(unknown.contains("list_meetings"), "{unknown}");
    // Unique prefixes resolve; path-like ids never escape the folder.
    assert!(library().notes("1790000500").is_ok());
    assert!(library().notes("../meetings").is_err());
}

#[test]
fn a_held_lock_marks_the_meeting_in_progress() {
    let root = std::env::temp_dir().join(format!(
        "voice-control-meeting-query-{}-{}",
        std::process::id(),
        crate::events::now_ms()
    ));
    let directory = root.join(STANDUP);
    fs::create_dir_all(&directory).unwrap();
    for entry in fs::read_dir(fixtures().join(STANDUP)).unwrap() {
        let path = entry.unwrap().path();
        fs::copy(&path, directory.join(path.file_name().unwrap())).unwrap();
    }
    let lock = File::create(directory.join("active.lock")).unwrap();
    let library = Library::new(&root);
    assert!(list(&library, true).contains("interrupted"));

    fs2::FileExt::lock_exclusive(&lock).unwrap();
    let output = list(&library, true);
    assert!(
        output.contains("recording (in progress) · rolling transcript"),
        "{output}"
    );
    assert!(list(&library, false).starts_with("No meetings match"));
    let notes = library.notes(STANDUP).unwrap();
    assert!(
        notes.contains("No notes yet: the meeting is still recording."),
        "{notes}"
    );

    fs2::FileExt::unlock(&lock).unwrap();
    assert!(list(&library, true).contains("interrupted"));
    fs::remove_dir_all(root).unwrap();
}

fn snapshot(root: &Path) -> Vec<(PathBuf, u64, SystemTime)> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let metadata = fs::metadata(&directory).unwrap();
        files.push((
            directory.clone(),
            metadata.len(),
            metadata.modified().unwrap(),
        ));
        for entry in fs::read_dir(&directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else {
                let metadata = fs::metadata(&path).unwrap();
                files.push((path, metadata.len(), metadata.modified().unwrap()));
            }
        }
    }
    files.sort();
    files
}

#[test]
fn queries_never_write_to_the_meetings_folder() {
    let before = snapshot(&fixtures());
    let library = library();
    list(&library, true);
    search("scheduled sessions cron", true);
    for id in [WEEKLY, ROADMAP, STANDUP] {
        transcript(id, |_| {});
        library.notes(id).unwrap();
        library.markdown(id, NOW).unwrap();
    }
    assert_eq!(before, snapshot(&fixtures()));
}

#[test]
fn markdown_has_metadata_notes_and_turns() {
    let markdown = library().markdown(WEEKLY, NOW).unwrap();
    assert!(markdown.starts_with("# Weekly sync\n\n- Meeting: 1790000000000-1001\n"));
    assert!(markdown.contains("- App: Zoom"));
    assert!(markdown.contains("## Summary"));
    assert!(markdown.contains("## Transcript\n\n**Them** · 00:00\nHey Kit, thanks for joining."));
}

#[test]
fn parses_times_offsets_and_words() {
    assert_eq!(
        parse_time("2026-10-15T21:58:31Z", NOW, false).unwrap(),
        1_792_101_511_000
    );
    assert_eq!(
        parse_time("2026-10-15T14:58:31-07:00", NOW, false).unwrap(),
        1_792_101_511_000
    );
    assert_eq!(
        parse_time("1792101511335", NOW, false).unwrap(),
        1_792_101_511_335
    );
    assert_eq!(parse_time("7d", NOW, false).unwrap(), NOW - 7 * 86_400_000);
    let day = parse_time("2026-10-15", NOW, false).unwrap();
    assert_eq!(
        parse_time("2026-10-15", NOW, true).unwrap() - day,
        86_400_000
    );
    assert_eq!(
        parse_time("yesterday", NOW, true).unwrap(),
        parse_time("today", NOW, false).unwrap()
    );
    assert!(parse_time("last tuesday", NOW, false).is_err());

    assert_eq!(parse_offset("12:34").unwrap(), 754_000);
    assert_eq!(parse_offset("1:02:03").unwrap(), 3_723_000);
    assert_eq!(parse_offset("90").unwrap(), 90_000);
    assert_eq!(parse_offset("1.5m").unwrap(), 90_000);
    assert!(parse_offset("soon").is_err());

    for (word, expected) in [
        ("scheduled", "schedul"),
        ("schedule", "schedul"),
        ("scheduling", "schedul"),
        ("sessions", "session"),
        ("recurring", "recur"),
        ("recur", "recur"),
        ("automated", "automat"),
        ("dax", "dax"),
        ("class", "class"),
    ] {
        assert_eq!(stem(word), expected, "{word}");
    }
    let vocabulary = [
        "schedul", "settl", "steal", "still", "dack", "deck", "docker",
    ]
    .map(String::from);
    let matches = |term: &str| {
        expand(term, vocabulary.iter(), true)
            .into_iter()
            .map(|(stem, _)| stem)
            .collect::<Vec<_>>()
    };
    // Sound-alikes apply to short, name-like words only.
    assert_eq!(matches("schedul"), ["schedul"]);
    assert_eq!(matches("dax"), ["dack", "deck"]);
    assert_eq!(soundex("dax"), soundex("dack"));
    assert_ne!(soundex("dax"), soundex("max"));
    assert_eq!(fold("Café DAX"), "cafe dax");
    assert_eq!(edit_distance("cron", "corn", 1), Some(1));
    assert_eq!(edit_distance("cron", "crane", 1), None);
}
