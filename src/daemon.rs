use crate::config::Config;
use crate::hook;
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::collections::HashMap;
use std::io::Read;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

pub fn socket_path() -> PathBuf {
    std::env::temp_dir().join("clostty.sock")
}

const POLL: Duration = Duration::from_millis(1500);

// One global daemon owns the title bar of every terminal tab. Hooks from any
// session forward their raw payload here (so there is a single tty writer); a
// poll loop discovers sessions via `claude agents --json` and overlays workflow
// progress that no hook can report.
pub fn run() -> Result<()> {
    let path = socket_path();
    let listener = bind_singleton(&path)?;
    println!("clostty daemon listening on {}", path.display());

    let state = Arc::new(Mutex::new(State {
        cfg: Config::load(),
        sessions: HashMap::new(),
    }));

    let lstate = Arc::clone(&state);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut s = stream;
            let mut buf = String::new();
            if s.read_to_string(&mut buf).is_ok() && !buf.is_empty() {
                if let Ok(mut st) = lstate.lock() {
                    st.on_event(&buf);
                }
            }
        }
    });

    loop {
        let agents = poll_agents();
        if let Ok(mut st) = state.lock() {
            st.reconcile(agents);
            st.render_all();
        }
        std::thread::sleep(POLL);
    }
}

// Binding the socket *is* the singleton lock: a live socket means another daemon
// owns it; a stale file (nothing answers) is cleaned up and rebound.
fn bind_singleton(path: &PathBuf) -> Result<UnixListener> {
    match UnixListener::bind(path) {
        Ok(l) => Ok(l),
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            if UnixStream::connect(path).is_ok() {
                bail!("clostty daemon already running ({})", path.display());
            }
            std::fs::remove_file(path).ok();
            UnixListener::bind(path).context("rebind socket")
        }
        Err(e) => Err(e).context("bind socket"),
    }
}

#[derive(Deserialize, Default)]
struct Agent {
    pid: u32,
    cwd: Option<String>,
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
    name: Option<String>,
    kind: Option<String>,
    status: Option<String>,
}

fn poll_agents() -> Vec<Agent> {
    let out = match Command::new("claude").args(["agents", "--json"]).output() {
        Ok(o) if o.status.success() => o.stdout,
        _ => return Vec::new(),
    };
    serde_json::from_slice(&out).unwrap_or_default()
}

#[derive(Deserialize)]
struct Event {
    hook_event_name: Option<String>,
    tool_name: Option<String>,
    notification_type: Option<String>,
    session_id: Option<String>,
    cwd: Option<String>,
}

struct Sess {
    cwd: String,
    tty: Option<String>,
    name: String,
    status: String,
    hook_icon: Option<String>,
    last_written: Option<String>,
}

struct State {
    cfg: Config,
    sessions: HashMap<String, Sess>,
}

impl State {
    // Refresh which sessions exist (and their tty/name/status) from the poll.
    // Hook-set icons and last-written titles survive across ticks.
    fn reconcile(&mut self, agents: Vec<Agent>) {
        let mut seen = Vec::new();
        for a in agents {
            if a.kind.as_deref() != Some("interactive") {
                continue; // background agents have no terminal tab
            }
            let Some(sid) = a.session_id.clone() else { continue };
            seen.push(sid.clone());
            let cwd = a.cwd.clone().unwrap_or_default();
            let tty = hook::tty_for_pid(a.pid);
            let name = a.name.clone().unwrap_or_else(|| session_name(&cwd, &sid));
            let entry = self.sessions.entry(sid).or_insert_with(|| Sess {
                cwd: cwd.clone(),
                tty: tty.clone(),
                name: name.clone(),
                status: String::new(),
                hook_icon: None,
                last_written: None,
            });
            entry.cwd = cwd;
            entry.tty = tty;
            entry.name = name;
            entry.status = a.status.clone().unwrap_or_default();
        }
        self.sessions.retain(|k, _| seen.contains(k));
    }

    fn render_all(&mut self) {
        let busy = self.cfg.icons.user_prompt_submit.clone();
        let idle = self.cfg.icons.stop.clone();
        for (sid, sess) in self.sessions.iter_mut() {
            render(sess, sid, &busy, &idle);
        }
    }

    fn on_event(&mut self, raw: &str) {
        let Ok(ev) = serde_json::from_str::<Event>(raw) else { return };
        let Some(sid) = ev.session_id.clone() else { return };
        let icon = hook::pick_icon(
            &self.cfg,
            ev.hook_event_name.as_deref().unwrap_or(""),
            ev.tool_name.as_deref(),
            ev.notification_type.as_deref(),
        )
        .map(str::to_string);
        let busy = self.cfg.icons.user_prompt_submit.clone();
        let idle = self.cfg.icons.stop.clone();
        let sess = self.sessions.entry(sid.clone()).or_insert_with(|| {
            let cwd = ev.cwd.clone().unwrap_or_default();
            let name = session_name(&cwd, &sid);
            Sess { cwd, tty: None, name, status: String::new(), hook_icon: None, last_written: None }
        });
        if let Some(ic) = icon {
            sess.hook_icon = Some(ic);
        }
        render(sess, &sid, &busy, &idle); // best-effort immediate update if tty known
    }
}

fn render(sess: &mut Sess, sid: &str, busy: &str, idle: &str) {
    let Some(tty) = sess.tty.clone() else { return };
    let icon = sess
        .hook_icon
        .clone()
        .unwrap_or_else(|| if sess.status == "busy" { busy } else { idle }.to_string());
    // Only busy sessions can have a workflow running; skip the IO otherwise.
    let progress = (sess.status == "busy")
        .then(|| workflow_progress(&sess.cwd, sid))
        .flatten();
    let title = compose_title(&icon, progress.as_deref(), &sess.name);
    if sess.last_written.as_deref() != Some(&title) {
        hook::write_title(&tty, &title);
        sess.last_written = Some(title);
    }
}

fn compose_title(icon: &str, progress: Option<&str>, name: &str) -> String {
    match progress {
        Some(p) => format!("{icon} {p} {name}"),
        None => format!("{icon} {name}"),
    }
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
}

// Claude encodes the project dir as the cwd with `/` and `.` replaced by `-`.
fn project_dir(cwd: &str) -> PathBuf {
    let enc: String = cwd
        .chars()
        .map(|c| if c == '/' || c == '.' { '-' } else { c })
        .collect();
    home().join(".claude/projects").join(enc)
}

fn session_name(cwd: &str, sid: &str) -> String {
    let transcript = project_dir(cwd).join(format!("{sid}.jsonl"));
    hook::resolve_name(transcript.to_str(), Some(cwd))
}

fn workflow_progress(cwd: &str, sid: &str) -> Option<String> {
    let dir = project_dir(cwd).join(sid).join("workflows");
    let mut newest: Option<(SystemTime, PathBuf)> = None;
    for entry in std::fs::read_dir(&dir).ok()?.flatten() {
        let p = entry.path();
        let is_wf = p.extension().is_some_and(|e| e == "json")
            && p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("wf_"));
        if !is_wf {
            continue;
        }
        if let Ok(m) = entry.metadata().and_then(|m| m.modified())
            && newest.as_ref().is_none_or(|(t, _)| m > *t)
        {
            newest = Some((m, p));
        }
    }
    let txt = std::fs::read_to_string(newest?.1).ok()?;
    parse_workflow_progress(&txt)
}

#[derive(Deserialize)]
struct WfJournal {
    status: Option<String>,
    #[serde(rename = "agentCount")]
    agent_count: Option<u64>,
    #[serde(rename = "workflowProgress")]
    workflow_progress: Option<Vec<WfEntry>>,
}

#[derive(Deserialize)]
struct WfEntry {
    #[serde(rename = "type")]
    kind: Option<String>,
    state: Option<String>,
}

fn parse_workflow_progress(json: &str) -> Option<String> {
    let j: WfJournal = serde_json::from_str(json).ok()?;
    if matches!(j.status.as_deref(), Some("completed" | "failed" | "error")) {
        return None;
    }
    let empty = Vec::new();
    let agents: Vec<&WfEntry> = j
        .workflow_progress
        .as_ref()
        .unwrap_or(&empty)
        .iter()
        .filter(|w| w.kind.as_deref() == Some("workflow_agent"))
        .collect();
    let total = j
        .agent_count
        .map(|c| c as usize)
        .filter(|&c| c > 0)
        .unwrap_or(agents.len());
    if total == 0 {
        return None;
    }
    let done = agents
        .iter()
        .filter(|w| w.state.as_deref() == Some("done"))
        .count();
    if done >= total {
        return None; // finished — count disappears
    }
    Some(format!("{done}/{total}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compose_with_and_without_progress() {
        assert_eq!(compose_title("🔵", Some("2/5"), "main"), "🔵 2/5 main");
        assert_eq!(compose_title("🟢", None, "main"), "🟢 main");
    }

    #[test]
    fn project_dir_encodes_cwd() {
        let d = project_dir("/Users/ax/project/rust/clostty");
        assert!(d.ends_with("-Users-ax-project-rust-clostty"));
        let dotted = project_dir("/Users/ax/foo.bar");
        assert!(dotted.ends_with("-Users-ax-foo-bar"));
    }

    #[test]
    fn progress_running_counts_done_vs_total() {
        let j = r#"{
            "status":"running","agentCount":5,
            "workflowProgress":[
                {"type":"workflow_phase","index":1},
                {"type":"workflow_agent","state":"done"},
                {"type":"workflow_agent","state":"done"},
                {"type":"workflow_agent","state":"running"}
            ]}"#;
        assert_eq!(parse_workflow_progress(j), Some("2/5".into()));
    }

    #[test]
    fn progress_completed_is_none() {
        let j = r#"{"status":"completed","agentCount":5,
            "workflowProgress":[{"type":"workflow_agent","state":"done"}]}"#;
        assert_eq!(parse_workflow_progress(j), None);
    }

    #[test]
    fn progress_falls_back_to_entry_count_without_agentcount() {
        let j = r#"{"status":"running",
            "workflowProgress":[
                {"type":"workflow_agent","state":"done"},
                {"type":"workflow_agent","state":"queued"}
            ]}"#;
        assert_eq!(parse_workflow_progress(j), Some("1/2".into()));
    }

    #[test]
    fn progress_all_done_is_none() {
        let j = r#"{"status":"running","agentCount":2,
            "workflowProgress":[
                {"type":"workflow_agent","state":"done"},
                {"type":"workflow_agent","state":"done"}
            ]}"#;
        assert_eq!(parse_workflow_progress(j), None);
    }
}
