# Read transcripts from the command line

The macOS release binary can read retained dictations and recorded meetings
while Hex runs. Use `/Applications/Hex.app/Contents/MacOS/hex` if `hex` is not
on your PATH.

```sh
hex history --since 3d --app 'OpenCode Beta' --grep problems --json
hex history --since 2026-10-20 --until 2026-10-21 --raw --oldest-first --limit 500
hex history --since 7d --search 'problems board' --json
hex meetings list --since 7d --json
hex meetings show latest --md
hex meetings show 1790000500000-1002 --json
hex meetings search 'release schedule'
```

## History filters apply before the limit

`history` returns successful retained results, including Send and Voice Action.
Each result has a timestamp, application, recorded processing mode if present,
and final text. `--raw` adds the stored pre-processing transcript. Older entries
may have no app or mode; Hex does not infer missing metadata.

- `--since` is inclusive and `--until` is exclusive. A date alone means local
  midnight; `--until 2026-10-20` includes all of October 20.
- Times accept `3d`, `36h`, `90m`, `2w`, `today`, `yesterday`, `now`, dates,
  local date-times, RFC 3339, or Unix milliseconds (10-digit values are seconds).
  Relative times count backwards from the query time.
- `--app` matches the entire application name, ignoring case.
- `--grep` matches a case-insensitive literal substring in raw or final text,
  including raw text when `--raw` is absent. It is not a regular expression.
- `--search QUERY` ranks the remaining entries with the meeting search's
  stemmed, fuzzy BM25 over final and raw text, so `bord` finds `board` and
  `scheduling` finds `schedule`. Quoted phrases are required, exact (stemmed)
  matches. Unmatched entries are dropped. It combines with the other filters and
  cannot be used with `--oldest-first`. Statistics cover only the filtered,
  unexpired entries, so scores compare within one response, not across queries.
- Results sort by timestamp and then ID, newest first; with `--search`, by score
  and then newest first. `--oldest-first` reverses the time order. `--limit`
  defaults to 50 and accepts 1–10,000.
- The saved retention window filters expired entries even before the app's next
  cleanup. Missing settings use seven days. Invalid settings fail rather than
  silently widening the retention window. Off preserves existing entries and
  prevents new ones, matching the History pane.

## Meetings use the same queries as MCP

`meetings list` defaults to 20 entries (maximum 200). It accepts `--since` and
`--until`, using the same time syntax as history. Meetings qualify when their
duration overlaps the range; they sort newest first. Active recordings are
included and labeled. Unreadable folders are reported, not repaired.

`meetings show` accepts an exact ID, an unambiguous prefix of at least six
characters, or `latest` (the newest readable meeting, including one in progress).
The default and `--md` return full Markdown with metadata, stored notes, and
speaker turns. `--json` returns transcript entries with relative millisecond
offsets. No notes or transcripts are generated.

`meetings search QUERY` uses the MCP query's stemmed, fuzzy BM25 ranking over
transcripts and stored notes. Quoted phrases constrain the search. `--since`,
`--until`, and `--limit` (default 10, maximum 50) are supported; output is text.
Use `hex mcp` for bounded transcript paging around a search hit.

Debug builds retain `meeting record`, `watch`, `probe`, `transcribe`, and other
developer commands. Their `meeting list/show` commands use the read-only query
path too. The plural `meetings` commands are available in release builds.

## JSON version 1

Every JSON response has integer `version: 1`. Consumers should ignore unknown
fields; incompatible field changes require a new version. All `*_ms` fields are
integer milliseconds. Absolute timestamps use Unix time. Nullable fields are
present as `null`, except `raw_text`, which is omitted without `--raw`, and
`score` and `snippet`, which are omitted without `--search`.
JSON goes to stdout; errors go to stderr with a nonzero exit status. Empty
results are successful responses. `truncated` reports the query's output limit,
not data already expired or evicted from the store.

```ts
type History = {
  version: 1
  retention: "off" | "day" | "week" | "month" | "forever"
  truncated: boolean
  entries: {
    id: number
    timestamp_ms: number
    application: string | null
    mode: string | null
    kind: "dictation" | "send" | "voice_action"
    final_text: string
    raw_text?: string // present only with --raw
    score?: number // present only with --search; higher is more relevant
    snippet?: string // present only with --search; matches wrapped in **
  }[]
}

type Meeting = {
  id: string
  title: string
  timestamp_ms: number
  duration_ms: number
  application: string | null
  status: "starting" | "recording" | "transcribing" | "complete" | "interrupted" | "failed"
  in_progress: boolean
  transcript_source: "final" | "rolling" | "missing" // older builds also reported "draft"
  warnings: string[]
}
type MeetingList = {
  version: 1
  meetings: Meeting[]
  skipped: { id: string; reason: string }[]
  truncated: boolean
}
type MeetingShow = {
  version: 1
  meeting: Meeting
  notes: { title: string; summary: string; generated_at_ms: number } | null
  entries: {
    source: "microphone" | "system" // You / Them; remote speakers aren't separated
    start_ms: number // relative to the meeting start
    end_ms: number
    text: string
  }[]
}
```

## Reads never start or change the app

These commands dispatch before data-folder migration, logging, instance locking,
local API startup, permissions, GUI, and Hex Microphone. They open no microphone,
load no model, make no network request, and create no files or directories.
History reads one file descriptor, so an atomic replacement yields a complete
old or new snapshot. Malformed history produces an error without recovery writes.

Data comes from `~/Library/Application Support/hex`. Read-only resolution also
supports the `voice-control` legacy folder and compatibility symlink, using the
same conflict policy as the app. `HEX_APPLICATION_SUPPORT_DIR` overrides the
root for fixtures or a mirrored copy. These CLI commands currently target macOS;
Linux and Windows retain their existing command surfaces.

## Retention stays bounded in one store

The history cap is now 10,000 entries or 16 MiB of raw plus final text, whichever
is reached first, up from 2,000 / 4 MiB. Per-entry limits, owner-only files,
atomic replacement, expiry, explicit deletion, and Clear all remain shared with
the History pane. Older entries already evicted cannot be recovered by this change.

Keeping one store avoids a second set of expiry and deletion rules. A monthly
archive would need atomic rollover, ID deduplication, expiry while idle, and
deletion across every file. The larger cap is a bounded increment using the
existing store; it still rewrites the JSON file on each successful result. The
cap is not a guarantee to retain every result within a chosen window, and
Forever is also hard-capped. No separate archive is created.
