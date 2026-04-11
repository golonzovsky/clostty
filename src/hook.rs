use crate::config::Config;
use anyhow::Result;
use serde::Deserialize;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::Command;

#[derive(Deserialize)]
struct HookInput {
    hook_event_name: Option<String>,
    tool_name: Option<String>,
    transcript_path: Option<String>,
    cwd: Option<String>,
    notification_type: Option<String>,
}

pub fn run() -> Result<()> {
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf)?;

    if let Ok(path) = std::env::var("CLOSTTY_LOG")
        && let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path)
    {
        let _ = writeln!(f, "{buf}");
    }

    let input: HookInput = serde_json::from_str(&buf)?;

    let Some(event) = input.hook_event_name.as_deref() else {
        return Ok(());
    };

    let cfg = Config::load();
    let Some(icon) = pick_icon(
        &cfg,
        event,
        input.tool_name.as_deref(),
        input.notification_type.as_deref(),
    ) else {
        return Ok(());
    };

    let name = resolve_name(input.transcript_path.as_deref(), input.cwd.as_deref());
    set_title(&format!("{icon} {name}"))?;
    Ok(())
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
    let Ok(mut tty) = OpenOptions::new().write(true).open("/dev/tty") else {
        return Ok(());
    };
    let _ = write!(tty, "\x1b]2;{title}\x07");
    Ok(())
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
        cfg.tools.bash = "SHELL".into();
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
}
