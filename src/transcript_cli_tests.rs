use super::*;
use clap::Parser;
use std::fs;
use std::path::PathBuf;

const NOW: u64 = 1_792_700_000_000;
const FIXTURE: &str = include_str!("../tests/fixtures/history.json");

fn args(extra: &[&str]) -> HistoryArgs {
    let cli =
        crate::Cli::try_parse_from(["hex", "history"].into_iter().chain(extra.iter().copied()))
            .unwrap();
    match cli.command.unwrap() {
        crate::Command::History(args) => args,
        _ => panic!("expected history"),
    }
}

fn temp() -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "hex-transcript-cli-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

fn entries() -> Vec<HistoryEntry> {
    serde_json::from_value(
        serde_json::from_str::<serde_json::Value>(FIXTURE).unwrap()["entries"].clone(),
    )
    .unwrap()
}

#[test]
fn history_filters_and_order_apply_before_the_limit() {
    let entries = entries();
    let query = args(&[
        "--since",
        "3d",
        "--app",
        "opencode beta",
        "--grep",
        "PROBLEMS",
        "--limit",
        "1",
    ]);
    let output = history_output(&entries, &query, HistoryRetention::Week, NOW).unwrap();
    assert_eq!(output.entries.iter().map(|e| e.id).collect::<Vec<_>>(), [2]);
    assert!(!output.truncated);
    let output = history_output(
        &entries,
        &args(&["--oldest-first", "--limit", "1"]),
        HistoryRetention::Week,
        NOW,
    )
    .unwrap();
    assert_eq!(output.entries[0].id, 2);
    assert!(output.truncated);
    let output = history_output(
        &entries,
        &args(&["--grep", "needle"]),
        HistoryRetention::Week,
        NOW,
    )
    .unwrap();
    assert_eq!(output.entries[0].id, 4);
    assert!(output.entries[0].raw_text.is_none());
    let output = history_output(
        &entries,
        &args(&["--until", "1792700000000"]),
        HistoryRetention::Week,
        NOW,
    )
    .unwrap();
    assert_eq!(
        output.entries.iter().map(|e| e.id).collect::<Vec<_>>(),
        [3, 2]
    );
    assert!(
        history_output(
            &entries,
            &args(&["--since", "now", "--until", "1d"]),
            HistoryRetention::Week,
            NOW
        )
        .is_err()
    );
}

#[test]
fn history_json_contract_snapshot() {
    let entries = entries();
    let output = history_output(
        &entries,
        &args(&["--grep", "problems", "--raw"]),
        HistoryRetention::Week,
        NOW,
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(output).unwrap(),
        serde_json::json!({
            "version":1,"retention":"week","truncated":false,"entries":[{
                "id":2,"timestamp_ms":1792540000000_u64,"application":"OpenCode Beta","mode":"Engineering",
                "kind":"dictation","final_text":"The Problems board.","raw_text":"the problems board"
            }]
        })
    );
    let mut entries = entries;
    entries[3].application = None;
    let output = history_output(
        &entries,
        &args(&["--limit", "1"]),
        HistoryRetention::Week,
        NOW,
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(output).unwrap(),
        serde_json::json!({
            "version":1,"retention":"week","truncated":true,"entries":[{
                "id":4,"timestamp_ms":1792700000000_u64,"application":null,"mode":null,
                "kind":"send","final_text":"Recent result"
            }]
        })
    );
}

#[test]
fn retention_defaults_fails_closed_and_off_preserves_existing_entries() {
    let root = temp();
    assert_eq!(retention(&root).unwrap(), HistoryRetention::Week);
    fs::write(root.join("settings.json"), "{}").unwrap();
    assert_eq!(retention(&root).unwrap(), HistoryRetention::Week);
    fs::write(root.join("settings.json"), r#"{"history_retention":"day"}"#).unwrap();
    let entries = entries();
    let output = history_output(&entries, &args(&[]), retention(&root).unwrap(), NOW).unwrap();
    assert_eq!(output.entries.iter().map(|e| e.id).collect::<Vec<_>>(), [4]);
    let output = history_output(&entries, &args(&[]), HistoryRetention::Off, NOW).unwrap();
    assert_eq!(output.entries.len(), 4);
    fs::write(root.join("settings.json"), "{broken").unwrap();
    assert!(retention(&root).is_err());
    assert_eq!(
        fs::read_to_string(root.join("settings.json")).unwrap(),
        "{broken"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn atomic_replace_read_never_observes_partial_history() {
    let root = temp();
    let path = root.join("history.json");
    fs::write(&path, FIXTURE).unwrap();
    let writer_path = path.clone();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let writer_barrier = barrier.clone();
    let writer = std::thread::spawn(move || {
        writer_barrier.wait();
        for index in 0..100 {
            let content = if index % 2 == 0 {
                FIXTURE.to_owned()
            } else {
                FIXTURE.replace("Recent result", "replacement")
            };
            fs::write(writer_path.with_extension("tmp"), content).unwrap();
            fs::rename(writer_path.with_extension("tmp"), &writer_path).unwrap();
        }
    });
    barrier.wait();
    for _ in 0..100 {
        let entries = crate::history::read_snapshot(&path).unwrap();
        assert_eq!(entries.len(), 4);
        assert!(matches!(
            entries[3].final_text.as_str(),
            "Recent result" | "replacement"
        ));
    }
    writer.join().unwrap();
    fs::write(&path, "{broken").unwrap();
    assert!(crate::history::read_snapshot(&path).is_err());
    assert_eq!(fs::read_to_string(&path).unwrap(), "{broken");
    assert!(!path.with_extension("json.corrupt").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn legacy_and_symlink_paths_are_read_without_migration() {
    use crate::app_paths::data_folder::{CURRENT, LEGACY, resolve};
    let root = temp();
    fs::create_dir(root.join(LEGACY)).unwrap();
    fs::write(root.join(LEGACY).join("history.json"), FIXTURE).unwrap();
    assert_eq!(
        crate::history::read_snapshot(&resolve(&root).join("history.json"))
            .unwrap()
            .len(),
        4
    );
    assert!(!root.join(CURRENT).exists());
    assert!(
        !root
            .join(LEGACY)
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink()
    );
    fs::rename(root.join(LEGACY), root.join(CURRENT)).unwrap();
    std::os::unix::fs::symlink(CURRENT, root.join(LEGACY)).unwrap();
    assert_eq!(resolve(&root), root.join(CURRENT));
    assert_eq!(
        crate::history::read_snapshot(&resolve(&root).join("history.json"))
            .unwrap()
            .len(),
        4
    );
    assert_eq!(
        fs::read_link(root.join(LEGACY)).unwrap(),
        Path::new(CURRENT)
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn meeting_json_contract_snapshot() {
    let library =
        Library::new(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/meetings"));
    let meeting = library.meeting("1790000500000-1002").unwrap();
    let json = meeting_json(&meeting, NOW);
    let expected: serde_json::Value =
        serde_json::from_str(include_str!("../tests/fixtures/meeting-cli.json")).unwrap();
    assert_eq!(json, expected);
    let scan = crate::meeting::query::Scan {
        meetings: vec![meeting],
        skipped: vec![],
    };
    assert_eq!(
        meeting_list_json(&scan, 20, NOW),
        serde_json::json!({
            "version": 1, "meetings": [expected["meeting"]], "skipped": [], "truncated": false,
        })
    );
}

#[test]
fn parses_release_commands_and_rejects_invalid_limits_and_times() {
    for command in [
        vec!["hex", "history"],
        vec!["hex", "meetings", "list", "--json"],
        vec!["hex", "meetings", "show", "latest", "--md"],
        vec!["hex", "meetings", "search", "problems"],
    ] {
        assert!(crate::Cli::try_parse_from(command).is_ok());
    }
    for command in [
        vec!["hex", "history", "--limit", "0"],
        vec!["hex", "history", "--limit", "10001"],
        vec!["hex", "meetings", "show", "latest", "--md", "--json"],
    ] {
        assert!(crate::Cli::try_parse_from(command).is_err());
    }
    for invalid in [
        "2026-02-30",
        "2026-00-01",
        "2026-10-20T23:00:99Z",
        "2026-10-20T23:00+99:00",
        "18446744073709551615d",
    ] {
        assert!(
            crate::meeting::query::parse_time(invalid, NOW, false).is_err(),
            "{invalid}"
        );
    }
    let start = crate::meeting::query::parse_time("2026-10-20", NOW, false).unwrap();
    let end = crate::meeting::query::parse_time("2026-10-20", NOW, true).unwrap();
    let range = TimeRange::parse(Some("2026-10-20"), Some("2026-10-20"), NOW).unwrap();
    assert!(range.contains(start));
    assert!(range.contains(end - 1));
    assert!(!range.contains(end));
    let second = crate::meeting::query::parse_time("2026-10-20T12:00:00Z", NOW, false).unwrap();
    assert_eq!(
        crate::meeting::query::parse_time("2026-10-20T12:00:00.123456Z", NOW, false).unwrap(),
        second + 123
    );
}
