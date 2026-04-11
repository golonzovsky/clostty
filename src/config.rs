use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub icons: Icons,
    pub tools: Tools,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Icons {
    pub session_start: String,
    pub user_prompt_submit: String,
    pub permission_request: String,
    pub permission_denied: String,
    pub stop: String,
    pub idle_prompt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Tools {
    pub bash: String,
    pub read: String,
    pub edit: String,
    pub task: String,
    pub web: String,
    pub default: String,
}

impl Default for Icons {
    fn default() -> Self {
        Self {
            session_start: "◆".into(),
            user_prompt_submit: "🔵".into(),
            permission_request: "🔴".into(),
            permission_denied: "🟢".into(),
            stop: "🟢".into(),
            idle_prompt: "🟢".into(),
        }
    }
}

impl Default for Tools {
    fn default() -> Self {
        Self {
            bash: "⚡".into(),
            read: "◉".into(),
            edit: "✎".into(),
            task: "⊜".into(),
            web: "◈".into(),
            default: "⚙".into(),
        }
    }
}

impl Config {
    pub fn load() -> Self {
        let Ok(path) = config_path() else {
            return Self::default();
        };
        let Ok(text) = fs::read_to_string(&path) else {
            return Self::default();
        };
        serde_yaml::from_str(&text).unwrap_or_default()
    }

    pub fn tool_icon(&self, name: &str) -> &str {
        match name {
            "Bash" | "BashOutput" | "KillShell" => &self.tools.bash,
            "Read" | "Glob" | "Grep" | "NotebookRead" | "LS" => &self.tools.read,
            "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => &self.tools.edit,
            "Task" => &self.tools.task,
            "WebFetch" | "WebSearch" => &self.tools.web,
            _ => &self.tools.default,
        }
    }
}

pub fn config_path() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME not set")?;
    Ok(PathBuf::from(home).join(".config/clostty/config.yaml"))
}

pub fn default_yaml() -> String {
    let header = "# clostty config — icons per hook event\n\
                  # Edit freely. Missing fields fall back to these defaults.\n\n";
    let body = serde_yaml::to_string(&Config::default()).unwrap();
    format!("{header}{body}")
}

pub fn edit() -> Result<()> {
    let path = config_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create {}", parent.display()))?;
    }
    if !path.exists() {
        fs::write(&path, default_yaml())
            .with_context(|| format!("write {}", path.display()))?;
    }

    let (editor, args) = editor_with_args();
    let status = Command::new(&editor)
        .args(args)
        .arg(&path)
        .status()
        .with_context(|| format!("spawn editor {editor}"))?;
    if !status.success() {
        anyhow::bail!("editor {editor} exited with {status}");
    }
    Ok(())
}

fn editor_with_args() -> (String, Vec<String>) {
    let raw = std::env::var("EDITOR").unwrap_or_default();
    let parts: Vec<String> = raw.split_whitespace().map(String::from).collect();
    match parts.len() {
        0 => ("nano".into(), vec![]),
        1 => (parts.into_iter().next().unwrap(), vec![]),
        _ => {
            let mut iter = parts.into_iter();
            let cmd = iter.next().unwrap();
            (cmd, iter.collect())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_round_trips_through_yaml() {
        let original = Config::default();
        let yaml = serde_yaml::to_string(&original).unwrap();
        let parsed: Config = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed.icons.session_start, original.icons.session_start);
        assert_eq!(parsed.tools.bash, original.tools.bash);
    }

    #[test]
    fn partial_yaml_fills_in_defaults() {
        let yaml = "icons:\n  user_prompt_submit: \"X\"\ntools:\n  bash: \"B\"\n";
        let parsed: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(parsed.icons.user_prompt_submit, "X");
        assert_eq!(parsed.tools.bash, "B");
        // fields not overridden keep their defaults
        assert_eq!(parsed.icons.stop, "🟢");
        assert_eq!(parsed.tools.read, "◉");
    }

    #[test]
    fn tool_icon_routes_tool_names() {
        let cfg = Config::default();
        assert_eq!(cfg.tool_icon("Bash"), "⚡");
        assert_eq!(cfg.tool_icon("Read"), "◉");
        assert_eq!(cfg.tool_icon("Edit"), "✎");
        assert_eq!(cfg.tool_icon("Task"), "⊜");
        assert_eq!(cfg.tool_icon("WebSearch"), "◈");
        assert_eq!(cfg.tool_icon("Unknown"), "⚙");
    }

    #[test]
    fn default_yaml_includes_header_comment() {
        let text = default_yaml();
        assert!(text.starts_with("# clostty config"));
        assert!(text.contains("icons:"));
        assert!(text.contains("session_start:"));
        assert!(text.contains("tools:"));
    }
}
