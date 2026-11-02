use crate::hook::{dbg, ps_field, resolve_name};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

// Ghostty keeps a surface's GHOSTTY_SURFACE_ID across window-save-state restore,
// so a per-surface file is enough to find the Claude session that ran in a tab.
#[derive(Debug, Serialize, Deserialize)]
pub struct Binding {
    pub surface_id: String,
    pub session_id: String,
    #[serde(default)]
    pub name: Option<String>,
    pub cwd: Option<String>,
    pub transcript_path: Option<String>,
    pub claude_pid: Option<u32>,
    pub claude_started: Option<String>,
    pub updated_at: u64,
}

fn dir() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME not set")?;
    Ok(PathBuf::from(home).join(".local/state/clostty/surfaces"))
}

fn surface_id() -> Option<String> {
    std::env::var("GHOSTTY_SURFACE_ID").ok().filter(|s| !s.is_empty())
}

fn path_for(surface: &str) -> Result<PathBuf> {
    Ok(dir()?.join(format!("{surface}.json")))
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn record_start(session_id: &str, cwd: Option<&str>, transcript_path: Option<&str>) -> Result<()> {
    let Some(surface) = surface_id() else {
        dbg("registry: no GHOSTTY_SURFACE_ID -> skip");
        return Ok(());
    };
    let (claude_pid, claude_started) = match claude_ancestor() {
        Some((p, s)) => (Some(p), Some(s)),
        None => (None, None),
    };
    let b = Binding {
        surface_id: surface.clone(),
        session_id: session_id.to_string(),
        name: Some(resolve_name(transcript_path, cwd)),
        cwd: cwd.map(String::from),
        transcript_path: transcript_path.map(String::from),
        claude_pid,
        claude_started,
        updated_at: now(),
    };
    let path = path_for(&surface)?;
    fs::create_dir_all(path.parent().unwrap())?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_string_pretty(&b)?)?;
    fs::rename(&tmp, &path)?;
    dbg(&format!("registry: {surface} -> {session_id}"));
    Ok(())
}

// Intentional exits drop the binding; anything else (crash, SIGTERM on reboot,
// reason "other") keeps it so the tab can be resumed after restore.
pub fn record_end(session_id: &str, reason: Option<&str>) -> Result<()> {
    let Some(surface) = surface_id() else { return Ok(()) };
    if reason == Some("other") {
        return Ok(());
    }
    let path = path_for(&surface)?;
    if let Some(b) = read(&path)
        && b.session_id == session_id
    {
        let _ = fs::remove_file(&path);
        dbg(&format!("registry: {surface} ended ({reason:?}) -> removed"));
    }
    Ok(())
}

pub enum Status {
    Live,
    Resumable,
    Stale,
}

pub fn status(b: &Binding) -> Status {
    if let (Some(pid), Some(started)) = (b.claude_pid, &b.claude_started)
        && ps_field(pid, "lstart=").as_deref() == Some(started.as_str())
    {
        return Status::Live;
    }
    // the transcript only appears after the first message, so check it after liveness
    if let Some(t) = &b.transcript_path
        && !Path::new(t).exists()
    {
        return Status::Stale;
    }
    Status::Resumable
}

/// Print the session id bound to this surface if it can be resumed; exit 1 otherwise.
pub fn resume() -> Result<()> {
    let found = surface_id()
        .and_then(|s| path_for(&s).ok())
        .and_then(|p| read(&p))
        .filter(|b| matches!(status(b), Status::Resumable));
    match found {
        Some(b) => {
            println!("{}", b.session_id);
            Ok(())
        }
        None => std::process::exit(1),
    }
}

pub fn sessions() -> Result<()> {
    let d = dir()?;
    let Ok(entries) = fs::read_dir(&d) else {
        println!("no bindings ({})", d.display());
        return Ok(());
    };
    let mut rows: Vec<Binding> = entries
        .flatten()
        .filter_map(|e| read(&e.path()))
        .collect();
    rows.sort_by_key(|b| std::cmp::Reverse(b.updated_at));
    let current = surface_id();
    for b in rows {
        let st = match status(&b) {
            Status::Live => "live",
            Status::Resumable => "resumable",
            Status::Stale => "stale",
        };
        let here = if current.as_deref() == Some(b.surface_id.as_str()) { "*" } else { " " };
        // /rename can happen after SessionStart, so prefer the transcript's current title
        let name = match &b.transcript_path {
            Some(t) if Path::new(t).exists() => resolve_name(Some(t), b.cwd.as_deref()),
            _ => b.name.clone().unwrap_or_default(),
        };
        println!(
            "{here} {st:<10} {name:<24} {:<40} {} {}",
            b.cwd.as_deref().unwrap_or(""),
            b.session_id,
            b.surface_id,
        );
    }
    Ok(())
}

fn read(path: &Path) -> Option<Binding> {
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

// Hooks are spawned by claude (possibly through a shell); walk up to the
// nearest ancestor whose executable is `claude` and remember how to tell
// whether that exact process is still alive later.
fn claude_ancestor() -> Option<(u32, String)> {
    let mut pid = std::process::id();
    for _ in 0..8 {
        pid = ps_field(pid, "ppid=")?.parse().ok()?;
        if pid <= 1 {
            return None;
        }
        let comm = ps_field(pid, "comm=")?;
        if Path::new(&comm).file_name().and_then(|n| n.to_str()) == Some("claude") {
            return Some((pid, ps_field(pid, "lstart=")?));
        }
    }
    None
}
