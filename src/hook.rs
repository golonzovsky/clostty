use crate::config::Config;
use anyhow::Result;
use serde::Deserialize;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Deserialize)]
struct HookInput {
    hook_event_name: Option<String>,
    tool_name: Option<String>,
    transcript_path: Option<String>,
    cwd: Option<String>,
    notification_type: Option<String>,
    session_id: Option<String>,
    agent_id: Option<String>,
}

pub fn run() -> Result<()> {
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf)?;

    dbg(&format!("stdin: {}", buf.trim()));

    let input: HookInput = match serde_json::from_str(&buf) {
        Ok(i) => i,
        Err(e) => {
            dbg(&format!("parse error: {e}"));
            return Ok(());
        }
    };

    let Some(event) = input.hook_event_name.as_deref() else {
        dbg("no hook_event_name -> noop");
        return Ok(());
    };

    let cfg = Config::load();
    let Some(icon) = pick_icon(
        &cfg,
        event,
        input.tool_name.as_deref(),
        input.notification_type.as_deref(),
    ) else {
        dbg(&format!("event={event} tool={:?} -> no icon, noop", input.tool_name));
        return Ok(());
    };

    let name = resolve_name(input.transcript_path.as_deref(), input.cwd.as_deref());
    let progress = subagent_progress(
        event,
        input.tool_name.as_deref(),
        input.session_id.as_deref(),
        input.agent_id.as_deref(),
    );
    let title = match &progress {
        Some(p) => format!("{icon} {p} {name}"),
        None => format!("{icon} {name}"),
    };
    dbg(&format!(
        "event={event} tool={:?} progress={progress:?} -> title={title:?}",
        input.tool_name
    ));
    set_title(&title)?;
    Ok(())
}

// Track parallel-subagent progress as `done/total` for the current turn, keyed
// by session. PreToolUse(Task) = launched, PostToolUse(Task) = finished; only
// the main agent's Tasks count (subagent-launched Tasks carry an agent_id). Each
// event drops a uniquely-named marker file so concurrent subagents never race on
// a shared counter; the count is read by listing those files.
fn subagent_progress(
    event: &str,
    tool: Option<&str>,
    session: Option<&str>,
    agent_id: Option<&str>,
) -> Option<String> {
    let dir = progress_dir(session?);
    let is_main = agent_id.is_none();

    match (event, tool) {
        ("UserPromptSubmit", _) | ("Stop", _) | ("SubagentStop", _) => {
            let _ = std::fs::remove_dir_all(&dir);
            return None;
        }
        ("PreToolUse", Some("Task" | "Agent")) if is_main => mark(&dir.join("launched")),
        ("PostToolUse", Some("Task" | "Agent")) if is_main => mark(&dir.join("done")),
        _ => {}
    }

    let (done, total) = counts(&dir);
    if total == 0 {
        return None;
    }
    if done >= total {
        // fan-out complete — clear so the count disappears once everything finished
        let _ = std::fs::remove_dir_all(&dir);
        return None;
    }
    Some(format!("{done}/{total}"))
}

fn progress_dir(session: &str) -> PathBuf {
    let safe: String = session
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    std::env::temp_dir().join("clostty-progress").join(safe)
}

fn mark(dir: &Path) {
    let _ = std::fs::create_dir_all(dir);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let _ = File::create(dir.join(format!("{}-{nanos}", std::process::id())));
}

fn counts(dir: &Path) -> (usize, usize) {
    let total = std::fs::read_dir(dir.join("launched"))
        .map(|r| r.count())
        .unwrap_or(0);
    let done = std::fs::read_dir(dir.join("done"))
        .map(|r| r.count())
        .unwrap_or(0);
    (done, total)
}

fn pick_icon<'a>(
    cfg: &'a Config,
    event: &str,
    tool: Option<&str>,
    notification_type: Option<&str>,
) -> Option<&'a str> {
    match event {
        "SessionStart" => Some(&cfg.icons.session_start),
        "UserPromptSubmit" => Some(&cfg.icons.user_prompt_submit),
        "PermissionRequest" => Some(&cfg.icons.permission_request),
        "PermissionDenied" => Some(&cfg.icons.permission_denied),
        "Stop" | "SubagentStop" => Some(&cfg.icons.stop),
        "PreToolUse" | "PostToolUse" => Some(cfg.tool_icon(tool.unwrap_or(""))),
        "Notification" => match notification_type? {
            "idle_prompt" => Some(&cfg.icons.idle_prompt),
            _ => None,
        },
        _ => None,
    }
}

fn resolve_name(transcript_path: Option<&str>, cwd: Option<&str>) -> String {
    if let Some(path) = transcript_path
        && let Some(title) = read_custom_title(Path::new(path))
    {
        return title;
    }
    if let Some(branch) = git_branch(cwd) {
        return branch;
    }
    cwd.and_then(|c| Path::new(c).file_name())
        .and_then(|n| n.to_str())
        .map(String::from)
        .unwrap_or_else(|| "claude".to_string())
}

#[derive(Deserialize)]
struct TranscriptLine {
    #[serde(rename = "type")]
    line_type: Option<String>,
    #[serde(rename = "customTitle")]
    custom_title: Option<String>,
}

fn read_custom_title(path: &Path) -> Option<String> {
    let file = File::open(path).ok()?;
    let reader = BufReader::new(file);
    let mut latest: Option<String> = None;
    for line in reader.lines().map_while(Result::ok) {
        let parsed: TranscriptLine = match serde_json::from_str(&line) {
            Ok(p) => p,
            Err(_) => continue,
        };
        if parsed.line_type.as_deref() == Some("custom-title")
            && let Some(title) = parsed.custom_title
            && !title.is_empty()
        {
            latest = Some(title);
        }
    }
    latest
}

fn git_branch(cwd: Option<&str>) -> Option<String> {
    let mut cmd = Command::new("git");
    if let Some(dir) = cwd {
        cmd.arg("-C").arg(dir);
    }
    let output = cmd.args(["branch", "--show-current"]).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let branch = String::from_utf8(output.stdout).ok()?.trim().to_string();
    if branch.is_empty() { None } else { Some(branch) }
}

fn set_title(title: &str) -> Result<()> {
    let Some(dev) = controlling_tty() else {
        dbg(&format!("set_title({title:?}): no controlling tty -> NOT WRITTEN"));
        return Ok(());
    };
    match OpenOptions::new().write(true).open(&dev) {
        Ok(mut tty) => {
            let r = write!(tty, "\x1b]2;{title}\x07");
            dbg(&format!(
                "set_title({title:?}): wrote to {dev}, result={r:?}, DISABLE_TITLE_env={:?}",
                std::env::var("CLAUDE_CODE_DISABLE_TERMINAL_TITLE").ok()
            ));
        }
        Err(e) => dbg(&format!("set_title({title:?}): open {dev} FAILED: {e}")),
    }
    Ok(())
}

// Claude Code spawns hooks detached from the controlling terminal, so /dev/tty
// fails (ENXIO). Fall back to the tty device of the nearest ancestor that still
// has one (the `claude` process itself).
fn controlling_tty() -> Option<String> {
    if OpenOptions::new().write(true).open("/dev/tty").is_ok() {
        dbg("controlling_tty: /dev/tty works directly");
        return Some("/dev/tty".into());
    }
    let mut pid = std::process::id();
    let mut chain = Vec::new();
    for _ in 0..8 {
        let tty = ps_field(pid, "tty=");
        chain.push(format!("{pid}:{}", tty.as_deref().unwrap_or("-")));
        if let Some(t) = tty
            && t != "??"
            && t != "?"
            && !t.is_empty()
        {
            dbg(&format!("controlling_tty: {} -> /dev/{t}", chain.join(" -> ")));
            return Some(format!("/dev/{t}"));
        }
        match ps_field(pid, "ppid=").and_then(|s| s.parse().ok()) {
            Some(p) => pid = p,
            None => break,
        }
    }
    dbg(&format!("controlling_tty: {} -> NONE", chain.join(" -> ")));
    None
}

// Opt-in debug logging: set CLOSTTY_LOG=/path to trace event/icon/tty decisions.
fn dbg(msg: &str) {
    let Ok(path) = std::env::var("CLOSTTY_LOG") else {
        return;
    };
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = writeln!(f, "[{ts} pid={}] {msg}", std::process::id());
    }
}

fn ps_field(pid: u32, field: &str) -> Option<String> {
    let out = Command::new("ps")
        .args(["-o", field, "-p", &pid.to_string()])
        .output()
        .ok()?;
    let s = String::from_utf8(out.stdout).ok()?.trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn parses_pre_tool_use_payload() {
        let json = r#"{
            "hook_event_name": "PreToolUse",
            "tool_name": "Bash",
            "tool_input": {"command": "ls"},
            "session_id": "abc",
            "transcript_path": "/tmp/x.jsonl",
            "cwd": "/tmp"
        }"#;
        let input: HookInput = serde_json::from_str(json).unwrap();
        assert_eq!(input.hook_event_name.as_deref(), Some("PreToolUse"));
        assert_eq!(input.tool_name.as_deref(), Some("Bash"));
        assert_eq!(input.cwd.as_deref(), Some("/tmp"));
    }

    #[test]
    fn parses_notification_payload() {
        let json = r#"{
            "hook_event_name": "Notification",
            "notification_type": "idle_prompt",
            "message": "Claude is waiting"
        }"#;
        let input: HookInput = serde_json::from_str(json).unwrap();
        assert_eq!(input.notification_type.as_deref(), Some("idle_prompt"));
    }

    #[test]
    fn parses_payload_with_unknown_fields() {
        let json = r#"{"hook_event_name":"Stop","extra":42,"nested":{"a":1}}"#;
        let input: HookInput = serde_json::from_str(json).unwrap();
        assert_eq!(input.hook_event_name.as_deref(), Some("Stop"));
    }

    #[test]
    fn icon_for_pretooluse_bash() {
        let cfg = Config::default();
        assert_eq!(pick_icon(&cfg, "PreToolUse", Some("Bash"), None), Some("⚡"));
        assert_eq!(pick_icon(&cfg, "PreToolUse", Some("Read"), None), Some("◉"));
        assert_eq!(pick_icon(&cfg, "PreToolUse", Some("Edit"), None), Some("✎"));
        assert_eq!(pick_icon(&cfg, "PreToolUse", Some("Task"), None), Some("⊜"));
        assert_eq!(pick_icon(&cfg, "PreToolUse", Some("WebFetch"), None), Some("◈"));
        assert_eq!(pick_icon(&cfg, "PreToolUse", Some("Unknown"), None), Some("⚙"));
    }

    #[test]
    fn icon_for_state_events() {
        let cfg = Config::default();
        assert_eq!(pick_icon(&cfg, "UserPromptSubmit", None, None), Some("🔵"));
        assert_eq!(pick_icon(&cfg, "PermissionRequest", None, None), Some("🔴"));
        assert_eq!(pick_icon(&cfg, "Stop", None, None), Some("🟢"));
        assert_eq!(pick_icon(&cfg, "SessionStart", None, None), Some("◆"));
    }

    #[test]
    fn post_tool_use_matches_pre_tool_use() {
        let cfg = Config::default();
        assert_eq!(
            pick_icon(&cfg, "PostToolUse", Some("WebSearch"), None),
            pick_icon(&cfg, "PreToolUse", Some("WebSearch"), None),
        );
        assert_eq!(pick_icon(&cfg, "PostToolUse", Some("Bash"), None), Some("⚡"));
    }

    #[test]
    fn icon_for_notification_idle_only() {
        let cfg = Config::default();
        assert_eq!(pick_icon(&cfg, "Notification", None, Some("idle_prompt")), Some("🟢"));
        assert_eq!(pick_icon(&cfg, "Notification", None, Some("auth_success")), None);
        assert_eq!(pick_icon(&cfg, "Notification", None, None), None);
    }

    #[test]
    fn icon_for_unknown_event() {
        let cfg = Config::default();
        assert_eq!(pick_icon(&cfg, "Mystery", None, None), None);
    }

    #[test]
    fn custom_config_overrides_icons() {
        let mut cfg = Config::default();
        cfg.icons.user_prompt_submit = "THINK".into();
        cfg.icons.tools.bash = "SHELL".into();
        assert_eq!(pick_icon(&cfg, "UserPromptSubmit", None, None), Some("THINK"));
        assert_eq!(pick_icon(&cfg, "PreToolUse", Some("Bash"), None), Some("SHELL"));
    }

    #[test]
    fn read_custom_title_picks_latest() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, r#"{{"type":"user","content":"hi"}}"#).unwrap();
        writeln!(f, r#"{{"type":"custom-title","customTitle":"first-name","sessionId":"a"}}"#).unwrap();
        writeln!(f, r#"{{"type":"assistant","content":"hello"}}"#).unwrap();
        writeln!(f, r#"{{"type":"custom-title","customTitle":"second-name","sessionId":"a"}}"#).unwrap();
        f.flush().unwrap();
        assert_eq!(read_custom_title(f.path()), Some("second-name".to_string()));
    }

    #[test]
    fn read_custom_title_returns_none_for_no_title() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, r#"{{"type":"user","content":"hi"}}"#).unwrap();
        f.flush().unwrap();
        assert_eq!(read_custom_title(f.path()), None);
    }

    #[test]
    fn read_custom_title_skips_garbage_lines() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "not json at all").unwrap();
        writeln!(f, r#"{{"type":"custom-title","customTitle":"good","sessionId":"a"}}"#).unwrap();
        f.flush().unwrap();
        assert_eq!(read_custom_title(f.path()), Some("good".to_string()));
    }

    #[test]
    fn read_custom_title_ignores_empty_title() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, r#"{{"type":"custom-title","customTitle":"real","sessionId":"a"}}"#).unwrap();
        writeln!(f, r#"{{"type":"custom-title","customTitle":"","sessionId":"a"}}"#).unwrap();
        f.flush().unwrap();
        assert_eq!(read_custom_title(f.path()), Some("real".to_string()));
    }

    #[test]
    fn subagent_progress_tracks_fanout() {
        let s = Some("test-fanout-a1b2c3");
        let pre = |t| subagent_progress("PreToolUse", Some(t), s, None);
        let post = |t| subagent_progress("PostToolUse", Some(t), s, None);

        subagent_progress("UserPromptSubmit", None, s, None); // reset turn
        assert_eq!(pre("Task"), Some("0/1".into()));
        assert_eq!(pre("Task"), Some("0/2".into()));
        assert_eq!(pre("Task"), Some("0/3".into()));
        assert_eq!(post("Task"), Some("1/3".into()));
        assert_eq!(post("Task"), Some("2/3".into()));
        // a subagent's own tool event keeps the count visible but doesn't inflate it
        assert_eq!(
            subagent_progress("PreToolUse", Some("Bash"), s, Some("agent-1")),
            Some("2/3".into())
        );
        // last subagent finishes -> count clears
        assert_eq!(post("Task"), None);
        // nothing lingers into later tool calls
        assert_eq!(pre("Read"), None);
        subagent_progress("Stop", None, s, None); // cleanup
    }

    #[test]
    fn subagent_progress_ignores_nested_subagent_tasks() {
        let s = Some("test-nested-d4e5f6");
        subagent_progress("UserPromptSubmit", None, s, None);
        // a Task spawned BY a subagent (has agent_id) must not be counted
        assert_eq!(
            subagent_progress("PreToolUse", Some("Task"), s, Some("agent-9")),
            None
        );
        subagent_progress("Stop", None, s, None);
    }

    #[test]
    fn subagent_progress_needs_session() {
        assert_eq!(subagent_progress("PreToolUse", Some("Task"), None, None), None);
    }

    #[test]
    fn subagent_progress_counts_agent_tool_alias() {
        // Claude reports the subagent-spawn tool as "Agent" (not "Task") in this build
        let s = Some("test-agent-alias-9z8y7x");
        subagent_progress("UserPromptSubmit", None, s, None);
        assert_eq!(subagent_progress("PreToolUse", Some("Agent"), s, None), Some("0/1".into()));
        assert_eq!(subagent_progress("PreToolUse", Some("Agent"), s, None), Some("0/2".into()));
        assert_eq!(subagent_progress("PostToolUse", Some("Agent"), s, None), Some("1/2".into()));
        assert_eq!(subagent_progress("PostToolUse", Some("Agent"), s, None), None);
        subagent_progress("Stop", None, s, None);
    }
}
