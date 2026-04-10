use anyhow::{Context, Result};
use serde_json::{Map, Value, json};
use std::fs;
use std::path::PathBuf;

const EVENTS: &[&str] = &[
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "PermissionRequest",
    "PermissionDenied",
    "Notification",
    "Stop",
];

fn settings_path() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME not set")?;
    Ok(PathBuf::from(home).join(".claude/settings.json"))
}

fn load_settings(path: &PathBuf) -> Result<Value> {
    if !path.exists() {
        return Ok(json!({}));
    }
    let text = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    if text.trim().is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_str(&text).with_context(|| format!("parse {}", path.display()))
}

fn save_settings(path: &PathBuf, value: &Value) -> Result<()> {
    let text = serde_json::to_string_pretty(value)?;
    fs::write(path, text + "\n")?;
    Ok(())
}

fn current_exe_string() -> Result<String> {
    let exe = std::env::current_exe()?;
    Ok(exe.to_string_lossy().into_owned())
}

fn is_clostty_command(cmd: &str) -> bool {
    cmd.contains("clostty") && cmd.contains("hook")
}

/// Remove any existing clostty entries from the hooks tree, then return it.
fn strip_clostty(hooks: &mut Map<String, Value>) {
    for event_value in hooks.values_mut() {
        let Some(matchers) = event_value.as_array_mut() else {
            continue;
        };
        matchers.retain(|matcher| {
            let Some(inner) = matcher.get("hooks").and_then(|h| h.as_array()) else {
                return true;
            };
            let has_clostty = inner.iter().any(|h| {
                h.get("command")
                    .and_then(|c| c.as_str())
                    .map(is_clostty_command)
                    .unwrap_or(false)
            });
            !has_clostty
        });
    }
    hooks.retain(|_, v| v.as_array().map(|a| !a.is_empty()).unwrap_or(true));
}

pub fn install() -> Result<()> {
    let path = settings_path()?;
    let mut settings = load_settings(&path)?;
    let exe = current_exe_string()?;
    let cmd = format!("{exe} hook");

    let root = settings
        .as_object_mut()
        .context("settings.json root must be an object")?;

    let hooks_value = root.entry("hooks").or_insert_with(|| json!({}));
    let hooks = hooks_value
        .as_object_mut()
        .context("hooks must be an object")?;

    strip_clostty(hooks);

    for event in EVENTS {
        let entry = hooks
            .entry(event.to_string())
            .or_insert_with(|| Value::Array(vec![]));
        let arr = entry
            .as_array_mut()
            .with_context(|| format!("{event} must be an array"))?;
        arr.push(json!({
            "matcher": "",
            "hooks": [
                { "type": "command", "command": cmd }
            ]
        }));
    }

    save_settings(&path, &settings)?;
    println!("Installed clostty hook → {}", path.display());
    println!("Command: {cmd}");
    Ok(())
}

pub fn uninstall() -> Result<()> {
    let path = settings_path()?;
    if !path.exists() {
        println!("No settings.json at {}", path.display());
        return Ok(());
    }
    let mut settings = load_settings(&path)?;
    let Some(root) = settings.as_object_mut() else {
        return Ok(());
    };
    if let Some(hooks) = root.get_mut("hooks").and_then(|h| h.as_object_mut()) {
        strip_clostty(hooks);
    }
    save_settings(&path, &settings)?;
    println!("Removed clostty hooks from {}", path.display());
    Ok(())
}
