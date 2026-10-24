# Control the running app from a terminal

```ts
Agent or terminal                                  // control.cli; macOS release
├── hex status [--json] -> Listener, capture, meeting, model, features,
│   permissions, window, update, version, pid, memory
│   └── Hex not running -> exit 3, "Hex is not running; run `hex start`"
├── hex start -> Already running: nothing | else open Hex.app, wait ≤ 20 s
├── hex quit | restart [--force]
│   └── Meeting or dictation in progress, no --force -> refused, exit 1
├── hex dictation | voice-action start|stop|cancel|toggle
│   ├── Same shortcut state machine as the hotkey; start locks the capture
│   ├── Repeat a command -> unchanged, exit 0
│   └── voice-action with Voice Action off -> feature-off, exit 1
├── hex paste-last -> Option-Shift-V's path
├── hex meetings record [--title] | stop | copy-last
│   ├── Meetings off -> feature-off; recording shows in the menu bar and HUD
│   │   exactly like a menu-bar recording
│   └── No meetings cancel: Hex cannot discard a recording
├── hex settings list | get <key|prefix> | set <key> <value> [--yes]
│   ├── The window's save, then ReloadSettings -> live, no restart
│   ├── Unknown key, bad value, read-only key -> refused, nothing written
│   ├── Shortcut conflict -> refused, as the window's recorder would
│   └── commands_enabled vs release_microphone_while_idle -> needs --yes
├── hex model list | use <id> | install <id> | cancel
│   └── use -> installed models only; never downloads
├── hex commands enable [--yes] | disable | retry
├── hex window show [pane] | hide
│   └── meetings pane with Meetings off -> feature-off
├── hex history clear --yes -> without --yes refused before Hex is asked
├── hex check-for-updates -> Sparkle's manual check
├── hex launch-at-login [on|off] -> SMAppService; changes need Hex.app
└── hex logs [--process] [-n N] [-f] -> local tail; works with Hex stopped
```

Every command except `start`, `restart`, and `logs` talks only to the running
Hex through the owner-only discovery file and the bearer-authenticated
`POST /control` route; none of them launch Hex. Commands dispatch in `main`
before folder migration, logging, and the instance lock, so they never compete
with the running app for its files. Opt-ins stay explicit settings: no command
turns Voice Action, Commands, or Meetings on as a side effect.

Linux and Windows add only `hex settings`, which reports `unsupported`; the
Linux service socket keeps its own `dictation`, `voice-action`, `paste-last`,
`status`, `quit`, and `restart`.

Sources: [control.rs](../../src/control.rs) (protocol, status, quit rules),
[control/settings.rs](../../src/control/settings.rs) (settings registry),
[control/cli.rs](../../src/control/cli.rs) (clap, output, exit codes, start,
logs), [macos_rune/control.rs](../../src/macos_rune/control.rs) (the macOS host),
the `/control` route in [local_api.rs](../../src/local_api.rs), and early
dispatch in [main.rs](../../src/main.rs).
[Command and JSON contracts](../cli.md) document flags, exit codes, and schema.

Checks: `control::tests` round-trip every command and reply, refuse other
protocol versions, and cover the quit rules; `control::settings::tests` include
`every_persisted_setting_has_a_cli_key`, value parsing, shortcut conflicts, the
microphone/Commands confirmation, zero volume, and Voice Action's free-shortcut
rule; `control::cli::tests` pin exit codes and output; local API's
`control_requests_are_authenticated_versioned_and_typed` covers authentication,
bounds, and error codes; `tests::control_commands_parse_beside_the_read_commands`
covers parsing next to `history` and `meetings`.

Gaps: no command has run against a live app; the unit checks substitute the
host. A CLI volume change does not preview a tone as the window does. Onboarding
permission prompts, calendar connection, and meeting deletion or renaming have
no command. Linux socket and Windows hosts do not carry control commands yet.
