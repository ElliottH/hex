//! Read-only, release-available transcript commands. No app services or migrations.

use std::io::{self, Read, Write};
use std::path::Path;

use clap::{Args, Subcommand};
use color_eyre::eyre::{Result, WrapErr, eyre};
use serde::{Deserialize, Serialize};

use crate::history::{HistoryEntry, HistoryKind, HistoryRetention};
use crate::meeting::query::{
    Library, ListQuery, Meeting, SearchQuery, TimeRange, TranscriptSource,
};

#[derive(Args, Debug)]
pub struct HistoryArgs {
    /// Lower time bound: 3d, today, YYYY-MM-DD, or RFC 3339.
    #[arg(long)]
    since: Option<String>,
    /// Exclusive upper bound; a date includes that entire local day.
    #[arg(long)]
    until: Option<String>,
    /// Exact application name, ignoring case.
    #[arg(long)]
    app: Option<String>,
    /// Case-insensitive substring in final or raw text.
    #[arg(long)]
    grep: Option<String>,
    #[arg(long, default_value_t = 50, value_parser = clap::value_parser!(u32).range(1..=10_000))]
    limit: u32,
    /// Include the stored pre-processing transcript.
    #[arg(long)]
    raw: bool,
    #[arg(long)]
    json: bool,
    #[arg(long)]
    oldest_first: bool,
}

#[derive(Args, Debug)]
pub struct MeetingListArgs {
    /// Include meetings overlapping this time or later.
    #[arg(long)]
    since: Option<String>,
    #[arg(long)]
    until: Option<String>,
    #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(1..=200))]
    limit: u32,
    #[arg(long)]
    json: bool,
}

impl Default for MeetingListArgs {
    fn default() -> Self {
        Self {
            since: None,
            until: None,
            limit: 20,
            json: false,
        }
    }
}

#[derive(Subcommand, Debug)]
pub enum MeetingsCommand {
    /// List recorded meetings, newest first, including recordings in progress.
    List(MeetingListArgs),
    /// Read a full transcript and stored notes, by id, unique prefix, or latest.
    Show {
        id: String,
        /// Markdown output (the default).
        #[arg(long, conflicts_with = "json")]
        md: bool,
        #[arg(long)]
        json: bool,
    },
    /// Search transcripts and notes with the same stemmed, fuzzy BM25 query as MCP.
    Search {
        query: String,
        #[arg(long)]
        since: Option<String>,
        #[arg(long)]
        until: Option<String>,
        #[arg(long, default_value_t = 10, value_parser = clap::value_parser!(u32).range(1..=50))]
        limit: u32,
    },
}

#[derive(Serialize)]
struct HistoryRow<'a> {
    id: u64,
    timestamp_ms: u64,
    application: Option<&'a str>,
    mode: Option<&'a str>,
    kind: HistoryKind,
    final_text: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    raw_text: Option<&'a str>,
}

#[derive(Serialize)]
struct HistoryOutput<'a> {
    version: u32,
    retention: HistoryRetention,
    entries: Vec<HistoryRow<'a>>,
    truncated: bool,
}

fn history_output<'a>(
    entries: &'a [HistoryEntry],
    args: &HistoryArgs,
    retention: HistoryRetention,
    now: u64,
) -> Result<HistoryOutput<'a>> {
    let range = TimeRange::parse(args.since.as_deref(), args.until.as_deref(), now)?;
    let cutoff = retention.max_age_ms().map(|age| now.saturating_sub(age));
    let app = args.app.as_ref().map(|s| s.to_lowercase());
    let grep = args.grep.as_ref().map(|s| s.to_lowercase());
    let mut matching: Vec<_> = entries
        .iter()
        .filter(|entry| cutoff.is_none_or(|cutoff| entry.timestamp_ms >= cutoff))
        .filter(|entry| range.contains(entry.timestamp_ms))
        .filter(|entry| {
            app.as_ref().is_none_or(|app| {
                entry
                    .application
                    .as_ref()
                    .is_some_and(|name| name.to_lowercase() == *app)
            })
        })
        .filter(|entry| {
            grep.as_ref().is_none_or(|grep| {
                entry.final_text.to_lowercase().contains(grep)
                    || entry.raw_text.to_lowercase().contains(grep)
            })
        })
        .collect();
    matching.sort_by_key(|entry| (entry.timestamp_ms, entry.id));
    if !args.oldest_first {
        matching.reverse();
    }
    let truncated = matching.len() > args.limit as usize;
    let entries = matching
        .into_iter()
        .take(args.limit as usize)
        .map(|entry| HistoryRow {
            id: entry.id,
            timestamp_ms: entry.timestamp_ms,
            application: entry.application.as_deref(),
            mode: entry.processing.as_ref().map(|p| p.profile.as_str()),
            kind: entry.kind,
            final_text: &entry.final_text,
            raw_text: args.raw.then_some(entry.raw_text.as_str()),
        })
        .collect();
    Ok(HistoryOutput {
        version: 1,
        retention,
        entries,
        truncated,
    })
}

/// Read only this setting, without loading AppSettings (which migrates and writes).
fn retention(root: &Path) -> Result<HistoryRetention> {
    #[derive(Default, Deserialize)]
    struct Settings {
        #[serde(default)]
        history_retention: HistoryRetention,
    }
    let file = match std::fs::File::open(root.join("settings.json")) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(HistoryRetention::default());
        }
        Err(error) => return Err(error).wrap_err("could not read history retention"),
    };
    let mut bytes = Vec::new();
    file.take(4 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > 4 * 1024 * 1024 {
        return Err(eyre!("settings file exceeds the read limit"));
    }
    let settings: Settings = serde_json::from_slice(&bytes)
        .wrap_err("could not read history retention; settings.json is invalid")?;
    Ok(settings.history_retention)
}

pub fn history(args: &HistoryArgs) -> Result<()> {
    let root = crate::app_paths::support_dir()?;
    let entries = crate::history::read_snapshot(&root.join("history.json"))
        .wrap_err("could not read history.json")?;
    let output = history_output(&entries, args, retention(&root)?, crate::events::now_ms())?;
    let mut out = io::stdout().lock();
    if args.json {
        return write_json(&mut out, &output);
    }
    for row in &output.entries {
        write!(
            out,
            "{} · {}",
            crate::meeting::query::iso_local(row.timestamp_ms),
            row.application.unwrap_or("Unknown app")
        )?;
        if let Some(mode) = row.mode {
            write!(out, " · {mode}")?;
        }
        writeln!(out, "\n{}", row.final_text)?;
        if let Some(raw) = row.raw_text {
            writeln!(out, "Raw: {raw}")?;
        }
    }
    if output.entries.is_empty() {
        writeln!(out, "No history matches.")?;
    }
    if output.truncated {
        writeln!(out, "More matches; raise --limit or narrow the time range.")?;
    }
    Ok(())
}

#[derive(Serialize)]
struct MeetingRow<'a> {
    id: &'a str,
    title: String,
    timestamp_ms: u64,
    duration_ms: u64,
    application: Option<&'a str>,
    status: crate::meeting::MeetingStatus,
    in_progress: bool,
    transcript_source: &'static str,
    warnings: &'a [String],
}

fn meeting_row(meeting: &Meeting, now: u64) -> MeetingRow<'_> {
    MeetingRow {
        id: &meeting.id,
        title: meeting.title(),
        timestamp_ms: meeting.start_ms(),
        duration_ms: meeting.duration_ms(now),
        application: meeting.manifest.app.as_deref(),
        status: meeting.status,
        in_progress: meeting.in_progress,
        transcript_source: match meeting.source {
            TranscriptSource::Final => "final",
            TranscriptSource::Rolling => "rolling",
            TranscriptSource::Draft => "draft",
            TranscriptSource::Missing => "missing",
        },
        warnings: &meeting.warnings,
    }
}

fn meeting_json(meeting: &Meeting, now: u64) -> serde_json::Value {
    let notes = meeting.manifest.notes.as_ref().map(|notes| serde_json::json!({
        "title": notes.title, "summary": notes.summary, "generated_at_ms": notes.generated_at_ms,
    }));
    serde_json::json!({
        "version": 1, "meeting": meeting_row(meeting, now), "notes": notes,
        "entries": meeting.entries,
    })
}

fn meeting_list_json(
    scan: &crate::meeting::query::Scan,
    limit: usize,
    now: u64,
) -> serde_json::Value {
    let rows: Vec<_> = scan
        .meetings
        .iter()
        .take(limit)
        .map(|meeting| meeting_row(meeting, now))
        .collect();
    let skipped: Vec<_> = scan
        .skipped
        .iter()
        .map(|item| serde_json::json!({"id": item.id, "reason": item.reason}))
        .collect();
    serde_json::json!({"version": 1, "meetings": rows, "skipped": skipped, "truncated": scan.meetings.len() > limit})
}

pub fn meetings(command: &MeetingsCommand) -> Result<()> {
    let library = Library::new(crate::app_paths::support_dir()?.join("meetings"));
    let now = crate::events::now_ms();
    let mut out = io::stdout().lock();
    match command {
        MeetingsCommand::List(args) => {
            let query = ListQuery {
                since: args.since.clone(),
                until: args.until.clone(),
                query: None,
                limit: args.limit as usize,
                include_in_progress: true,
            };
            if args.json {
                let scan = library.list_matches(&query, now)?;
                write_json(
                    &mut out,
                    &meeting_list_json(&scan, args.limit as usize, now),
                )
            } else {
                write!(out, "{}", library.list(&query, now)?)?;
                Ok(())
            }
        }
        MeetingsCommand::Show { id, json, .. } => {
            let meeting = if id == "latest" {
                library
                    .scan()?
                    .meetings
                    .into_iter()
                    .next()
                    .ok_or_else(|| eyre!("No meetings found."))?
            } else {
                library.meeting(id)?
            };
            if *json {
                write_json(&mut out, &meeting_json(&meeting, now))
            } else {
                write!(out, "{}", library.markdown(&meeting.id, now)?)?;
                Ok(())
            }
        }
        MeetingsCommand::Search {
            query,
            since,
            until,
            limit,
        } => {
            write!(
                out,
                "{}",
                library.search(
                    &SearchQuery {
                        query: query.clone(),
                        since: since.clone(),
                        until: until.clone(),
                        meeting_id: None,
                        limit: *limit as usize,
                        fuzzy: true,
                        include_in_progress: true,
                    },
                    now
                )?
            )?;
            Ok(())
        }
    }
}

fn write_json(out: &mut impl Write, value: &impl Serialize) -> Result<()> {
    serde_json::to_writer_pretty(&mut *out, value)?;
    writeln!(out)?;
    Ok(())
}

#[cfg(test)]
#[path = "transcript_cli_tests.rs"]
mod tests;
