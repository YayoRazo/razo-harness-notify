# harness-notify

A small, self-contained Rust CLI that fires a native OS desktop notification
when an AI coding harness (Claude Code, opencode, and others) finishes a task
or needs your input. It wires into each harness's own hook/plugin system so
the notification fires automatically as part of that harness's normal
lifecycle - there is no background daemon, no tray icon, and no autostart
entry. Settings (which events notify, sound, quiet hours, whether the
session name is included) live in one shared `config.toml` and can be
changed either by asking the harness agent to run `harness-notify config
set ...` on your behalf, or through a small on-demand native settings
window.

## Support tiers

Support level varies by harness, depending on how well-documented and
confirmed each one's hook surface is. This table reflects exactly what
`src/adapters/mod.rs`'s `all_adapters()` registers - nothing here is
aspirational.

| Tier | Harness | `notify` hook install | Status |
|---|---|---|---|
| A | Claude Code | `harness-notify install --harness claude-code` | Full, tested |
| A | opencode | `harness-notify install --harness opencode` | Full, tested |
| B | Antigravity | `harness-notify install --harness antigravity` | Shipped, **UNVERIFIED** |
| B | Kimi Code CLI | `harness-notify install --harness kimi` | Shipped, **UNVERIFIED** |
| C | Kilo | not supported | Config-command only (see below) |
| C | Kiro | not supported | Config-command only (see below) |
| D | Cursor | not supported | Config-command only (see below) |
| D | Windsurf | not supported | Config-command only (see below) |
| D | Cline | not supported | Config-command only (see below) |
| D | GitHub Copilot | not supported | Config-command only (see below) |

Notes on what these tiers actually mean in the shipped code:

- **Tier A** (`src/adapters/claude_code.rs`, `src/adapters/opencode.rs`):
  `install`/`uninstall` idempotently patch that harness's real hook or
  plugin file. Claude Code gets `Stop` -> `done`, `Notification` ->
  `attention`, `SubagentStop` -> `subagent-done`, and `SessionStart` -> a
  one-time OS-notification-capability check (see "Session-start check"
  below) as entries in `settings.json`. opencode gets a generated
  `plugin/harness-notify.js` file (under the project's `.opencode/` or the
  user's `<config_dir>/opencode/`) that listens on the generic `event` hook
  key (`session.idle`/`session.status` -> `done`, `permission.asked` ->
  `attention`, `session.created` -> the same check).
- **Tier B** (`src/adapters/antigravity.rs`, `src/adapters/kimi.rs`): the
  same idempotent-patch behavior as Tier A, but marked **UNVERIFIED**
  because the exact target path or event coverage has not been confirmed
  against a live install. Antigravity's adapter patches `hooks.json` and
  only wires the confirmed `Stop` -> `done` event - no `attention`-equivalent
  event name could be confirmed, so it is deliberately not invented (no
  session-start check either, for the same reason). Kimi's adapter patches
  `config.toml`'s `[[hooks]]` array with the same
  `Stop`/`Notification`/`SubagentStop`/`SessionStart` names Claude Code
  uses, targeting `~/.kimi-code/config.toml`; a third-party source
  elsewhere uses `~/.kimi/config.toml` instead, so confirm the path on a
  real install before relying on it.
- **Tiers C and D** (`src/adapters/unsupported.rs`): both are implemented by
  the exact same `UnsupportedAdapter` - there is no behavioral difference
  between them in the code today. `install` and `uninstall` always return a
  clear error (`"<harness> does not support automatic notify-hook install
  yet (open research question, see README)"`) and never write anything. The
  C/D split reflects how each harness
  was researched (Tier D's custom-command/skill surface is confirmed and
  documented; Tier C's is not), not a difference in what the tool does for
  them. All six get a generated config-command artifact instead (see
  "Configuring from inside a harness" below).

The spec this tool was built from also defines a conceptual "Tier E" (no
code, README-only). No shipped harness in `all_adapters()` falls into that
bucket, so it has no row in the table above - only the ten harness ids the
code actually registers are listed, in the order it registers them:
`claude-code`, `opencode`, `antigravity`, `kimi`, `kilo`, `kiro`, `cursor`,
`windsurf`, `cline`, `copilot`.

## Prerequisites

- **Linux**: a notification daemon such as `dunst` or `mako` must be running.
  The D-Bus `org.freedesktop.Notifications` service is the delivery mechanism.
- **macOS**: Notification Center is used; no extra deps needed.
- **Windows**: WinRT toast notifications; no extra deps needed.

## Install

Build and install from source with Cargo:

```sh
cargo install --path .
```

This builds the `gui` feature by default (see "GUI feature" below). To skip
the native settings window and its `eframe`/`egui` dependencies:

```sh
cargo install --path . --no-default-features
```

License: dual MIT OR Apache-2.0 (see `LICENSE-MIT` and `LICENSE-APACHE`).

## Command reference

| Command | Flags | What it does |
|---|---|---|
| `harness-notify notify` | `--event <name>` (required), `--harness <id>`, `--title <text>`, `--message <text>`, `--cwd <path>` | Fires a notification if the event is enabled and outside quiet hours. This is what an installed hook actually calls. An unrecognized `--event` (or any other malformed input) is a silent no-op, never a crash or a non-zero exit - it must never block the calling harness's hook chain. On Claude Code/Kimi, also reads the hook's JSON payload from stdin (if piped) to refine which event actually fires and to find the calling project's directory - see "Notification payload refinement" below. |
| `harness-notify install` | `--harness <id>` (required), `--project` | Installs the notify hook (and, on Tier A/B, a `SessionStart` OS-notification check) into the named harness's config. Defaults to the user-level location; `--project` targets the current project directory instead. Fails with a clear message on Tier C/D harnesses. |
| `harness-notify uninstall` | `--harness <id>` (required), `--project` | Removes only the hook entries this tool installed, leaving any other entries in the same file untouched. Fails with a clear message on Tier C/D harnesses. |
| `harness-notify test` | `--harness <id>` | Fires a sample "done" notification immediately, ignoring the harness's own hook wiring - useful for confirming the OS notification path itself works. It still goes through the same `should_fire()` config check as a real notify, and reports the outcome: fired, suppressed by config (`events.done=false` or active quiet hours), or the notifier's error with a non-zero exit when the OS call fails. |
| `harness-notify check` | `--hook <name>` | Checks whether the OS will actually display a notification (Windows and Linux - see "Session-start check" below) and prints a warning if it looks disabled. Called from a `SessionStart`/`session.created` hook, not run by hand. |
| `harness-notify config get <key>` | - | Prints one setting's current value. |
| `harness-notify config set <key> <value>` | - | Changes one setting and saves `config.toml`. |
| `harness-notify config list` | - | Prints every current setting. |
| `harness-notify config` (no subcommand) | - | Opens the on-demand native settings window. |
| `harness-notify` (no arguments) | - | Also opens the on-demand native settings window. |

Only `notify` is guaranteed to always exit `0`: because hooks call it
unattended, a malformed or missing flag is a silent no-op rather than a
clap error, so it can never block the calling harness's hook chain. The
other subcommands (`install`, `uninstall`, `test`, `config`) use clap's
normal error handling on malformed input - an informative message to
stderr and a non-zero exit code - since a human or an agent runs them
interactively, not an unattended hook. Runtime failures behave the same
way: an unknown harness, a Tier C/D `install`/`uninstall`, an unknown
config key or invalid value, and a failed config write all print to
stderr and exit non-zero.

### Config keys

| Key | Values | Meaning |
|---|---|---|
| `events.done` | `true`/`false` | notify when a task/session finishes (default `true`) |
| `events.attention` | `true`/`false` | notify when input/a decision is needed (default `true`) |
| `events.subagent_done` | `true`/`false` | notify when a subagent finishes (default `false`) |
| `session.include_name` | `true`/`false` | include which window sent the notification (default `false`) |
| `session.format` | `name` or `path` | how the session is labeled, if included (default `name`). `path` shows the full project directory - be aware this leaks local paths into notification previews on screen. |
| `sound.enabled` | `true`/`false` | play a sound with the notification (default `true`) |
| `dnd.enabled` | `true`/`false` | enable quiet hours (default `false`) |
| `dnd.start` | `HH:MM` | quiet hours start, local time (default `22:00`) |
| `dnd.end` | `HH:MM` | quiet hours end, local time, may cross midnight (default `08:00`) |

Config lives at `~/.harness-notify/config.toml` (a single shared file - v1
has no per-harness overrides; set the `HARNESS_NOTIFY_CONFIG_DIR`
environment variable to relocate the directory). A missing or corrupt file silently falls
back to the defaults above rather than erroring.

When `session.include_name` is on, the notification shows which project
directory sent it (the basename with `session.format = "name"`, the full
path with `"path"`) - not the harness id. Multiple windows of the same
harness open at once all say "claude-code" if only the harness id were
shown, which defeats the point; the real `cwd` is what actually
distinguishes them. The directory comes from the hook's own payload
(Claude Code, Kimi) or a `--cwd` flag the adapter passes explicitly
(opencode); if neither is available (e.g. a manual `notify` call with no
`--cwd`), it falls back to the harness id as before.

## Notification payload refinement

Claude Code's and Kimi's `Notification` hook is more overloaded than its
name suggests: its JSON payload carries a `notification_type` field with
values including `permission_prompt`, `idle_prompt`, `auth_success`,
`elicitation_dialog`, `elicitation_complete`, `elicitation_response`,
`agent_needs_input`, and `agent_completed` - all routed through the same
hook. Installing it with a single static `--event attention` would show
"Needs your input" for all eight, which misrepresents most of them:
`agent_needs_input`/`agent_completed` are subagent lifecycle notifications,
not something that needs the operator's attention.

`notify` reads the piped JSON payload (when one is piped - a manual
invocation with no stdin never blocks waiting for input) and refines the
event actually fired:

| `notification_type` | Fires as |
|---|---|
| `permission_prompt`, `idle_prompt`, `auth_success`, `elicitation_dialog`, `elicitation_complete`, `elicitation_response` | `attention` |
| `agent_needs_input`, `agent_completed` | `subagent-done` (respects `events.subagent_done`, off by default) |
| anything else, or no payload at all | whatever `--event` the hook was installed with |

This means the subagent-shaped notification types are silent by default,
the same as a real `SubagentStop` firing - not a separate toggle to learn.

## Session-start check

Because a toast/notification call can report success even when the OS
silently drops it, Tier A/B `install` also wires a `SessionStart`/
`session.created` hook that runs `harness-notify check` once per session.
If it detects notifications are off, it prints a plain warning that the
harness surfaces as context at the start of your next session - so a
broken setup gets caught by the harness telling you, not by silence.
`src/os_check.rs` has the confirmed signal per platform:

- **Windows**: the master "Notifications" toggle
  (`HKCU\...\PushNotifications\ToastEnabled`) being off. This is what a
  WinRT toast call reports success on even though nothing is ever shown.
- **Linux**: whether a notification daemon answers on the session D-Bus at
  all (`org.freedesktop.Notifications`, via `notify-rust`'s own already-
  linked client - no new dependency). This catches "nothing is listening"
  (no daemon installed/running); a specific daemon's own do-not-disturb
  state isn't checked, since that varies per daemon (dunst, mako, ...) with
  no common standard.
- **macOS**: not checked. The backend we use has the same
  "always looks like success" problem as Windows, but the reliable fix
  (`UNUserNotificationCenter`'s real authorization status) requires a
  proper bundle identifier a bare `cargo install` binary doesn't have -
  confirmed by reading `notify-rust`'s own source, which gates that API
  behind an experimental feature for exactly this reason. Left
  unimplemented rather than guessed, same as Antigravity's unconfirmed
  attention event.

## Platform notes

Notifications are delivered through [`notify-rust`](https://docs.rs/notify-rust):

- **Windows**: uses the WinRT toast notification API.
- **macOS**: uses Notification Center.
- **Linux**: uses the freedesktop D-Bus notification spec, which requires a
  running notification daemon (provided by most desktop environments; on a
  minimal or headless setup you may need to run one yourself, e.g.
  `dunst`).

`sound.enabled` maps to each platform's own mechanism: on Windows, `true`
plays the system default notification sound and `false` produces a silent
toast; on Linux, `true` leaves the daemon's own sound behavior untouched and
`false` sends the standard freedesktop `suppress-sound` hint. macOS
notifications are delivered without a sound either way - an audible one
needs a verified sound name plus a real bundle identifier that a bare
`cargo install` binary doesn't have, the same constraint that keeps the
session-start check unimplemented there.

## GUI feature

The on-demand native settings window (`src/gui.rs`, built on `eframe`/`egui`)
is a default-on Cargo feature named `gui`. It opens whenever `harness-notify`
is run with no arguments, or as `harness-notify config` with no further
subcommand, and lets you toggle every setting from the config table above
with a "Save" button - no command line needed. It is not a background
process: the window opens on demand and exits when closed.

Build without it with `cargo build --no-default-features` (or `cargo install
--path . --no-default-features`). A build made this way still works fully
from the command line; running it with no arguments prints a message telling
you to use `harness-notify config get/set/list` instead of opening a window.

## Configuring from inside a harness

Because settings live in one `config.toml` reachable only through the
`harness-notify config get/set/list` commands, the natural way to change a
setting from inside a harness session is to ask the coding agent in plain
language (e.g. "mute subagent notifications", "set quiet hours from 10pm to
8am") and let it translate that into the right `config` call. To make that
reliable, the `xtask` workspace member generates a small, harness-appropriate
command/skill artifact describing exactly that mapping, for **all ten**
harnesses - including the six that don't get a notify-hook install.

Generate the artifacts with:

```sh
cargo run -p xtask
```

This writes one file per harness under `dist/` (gitignored, regenerate as
needed - the canonical content lives in `xtask/templates/config-command.md`).
Copy the generated file into the matching harness-specific location in your
project or home directory:

| Harness | Generated artifact | Copy to |
|---|---|---|
| Claude Code | `dist/claude-code/.claude/commands/harness-notify-config.md` | `.claude/commands/` (project) or `~/.claude/commands/` (user) |
| opencode | `dist/opencode/.opencode/commands/harness-notify-config.md` | `.opencode/commands/` (project) or `<config_dir>/opencode/commands/` (user) |
| Cursor | `dist/cursor/.cursor/commands/harness-notify-config.md` | `.cursor/commands/` |
| Antigravity | `dist/antigravity/.agents/skills/harness-notify-config/SKILL.md` | `.agents/skills/harness-notify-config/` |
| Kimi Code CLI | `dist/kimi/.kimi/skills/harness-notify-config/SKILL.md` | `.kimi/skills/harness-notify-config/` |
| Kilo | `dist/kilo/.kilo/skills/harness-notify-config/SKILL.md` | `.kilo/skills/harness-notify-config/` |
| Kiro | `dist/kiro/.kiro/skills/harness-notify-config/SKILL.md` | `.kiro/skills/harness-notify-config/` |
| Windsurf | `dist/windsurf/.windsurf/skills/harness-notify-config/SKILL.md` | `.windsurf/skills/harness-notify-config/` |
| Cline | `dist/cline/.clinerules/harness-notify-skills/harness-notify-config/SKILL.md` | `.clinerules/harness-notify-skills/harness-notify-config/` |
| GitHub Copilot | `dist/copilot/.github/harness-notify-skills/harness-notify-config/SKILL.md` | `.github/harness-notify-skills/harness-notify-config/` |

Claude Code, opencode, and Cursor get a command-style artifact (front matter
with a `description` field); every other harness gets a skill-style artifact
(front matter with `name` and `description`). Both styles carry the same
body: the three `harness-notify config` subcommands, the full key/value
table above, and one worked example
(`"mute subagent notifications" -> harness-notify config set
events.subagent_done false`).

This works the same way regardless of tier - a Tier C/D harness with no
notify-hook support still gets full, agent-mediated configurability through
its command/skill artifact; only the automatic hook `install` is unavailable
for those six.
