# clostty

A [Claude Code](https://claude.com/claude-code) hook that updates your terminal tab title with the current session state, so you can tell at a glance which session is thinking, waiting on you, or done.

```
🔵 claude-hooks    ← thinking / running tools
🔴 claude-hooks    ← waiting for permission
🟢 claude-hooks    ← done / idle
```

## Why

Running multiple Claude Code sessions in different tabs (or worktrees) means you constantly tab around to check which one needs your attention. clostty pushes that state into the tab title via OSC 2 escape sequences, so the terminal does it for you.

## States

| Icon | Event(s)                              | Meaning                  |
| ---- | ------------------------------------- | ------------------------ |
| ◆    | `SessionStart`                        | Session starting         |
| 🔵   | `UserPromptSubmit`, `PostToolUse`     | Thinking / processing    |
| ⚡   | `PreToolUse` Bash / BashOutput        | Running shell command    |
| ◉    | `PreToolUse` Read / Glob / Grep / LS  | Reading or searching     |
| ✎    | `PreToolUse` Edit / Write / MultiEdit | Editing files            |
| ⊜    | `PreToolUse` Task                     | Spawning subagent        |
| ◈    | `PreToolUse` WebFetch / WebSearch     | Web                      |
| ⚙    | `PreToolUse` (other)                  | Other tool               |
| 🔴   | `PermissionRequest`                   | Waiting for your approval |
| 🟢   | `Stop`, `SubagentStop`, idle          | Finished                 |

## Display name

clostty picks the tab name in this order:

1. The session's `customTitle` from the transcript JSONL (set by `/rename`)
2. The current git branch
3. The basename of the working directory

## Install

```bash
cargo install --path .
clostty install
```

`clostty install` writes the hook entries into `~/.claude/settings.json`, pointing to wherever the binary lives (`std::env::current_exe()`). It registers handlers for: `SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `PermissionRequest`, `PermissionDenied`, `Notification`, `Stop`. Existing clostty entries are removed and re-added; other hooks in `settings.json` are preserved.

You'll also want to disable Claude Code's own terminal title management so it doesn't fight with clostty:

```bash
export CLAUDE_CODE_DISABLE_TERMINAL_TITLE=1
```

(Add this to your shell config.)

## Uninstall

```bash
clostty uninstall
```

Removes any hook entries pointing at clostty.

## Hook protocol

`clostty hook` reads a single JSON object from stdin and writes the new title to `/dev/tty`. It expects fields from Claude Code's hook input schema:

- `hook_event_name` — required, dispatches the icon
- `tool_name` — used for `PreToolUse` to pick a tool-specific icon
- `notification_type` — used for `Notification` (only `idle_prompt` produces output)
- `transcript_path` — read for the most recent `custom-title` line
- `cwd` — used for git branch lookup and basename fallback

Unknown events and missing fields are silently ignored. Failure to open `/dev/tty` (e.g., when invoked outside a terminal) is also silent — no error, no exit code.

## Tests

```bash
cargo test
```
