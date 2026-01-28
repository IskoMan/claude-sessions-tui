use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const DISPLAY_NAME_MAX_LEN: usize = 60;
const BYTES_PER_MB: u64 = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SortBy {
    Date,
    Size,
    Messages,
}

#[derive(Serialize, Deserialize, Default)]
pub struct Config {
    pub sort_by: Option<SortBy>,
    pub filter_query: Option<String>,
}

impl Config {
    fn path() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".config/claude-sessions-tui/config.json")
    }

    pub fn load() -> Self {
        fs::read_to_string(Self::path())
            .ok()
            .and_then(|c| serde_json::from_str(&c).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> io::Result<()> {
        let p = Self::path();
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(p, serde_json::to_string_pretty(self)?)
    }
}

#[derive(Clone, Debug)]
pub struct Session {
    pub id: String,
    pub path: PathBuf,
    pub project: String,
    pub size: u64,
    pub message_count: usize,
    pub first_message: String,
    pub modified: SystemTime,
    pub custom_name: Option<String>,
    pub related_files: Vec<PathBuf>,
}

impl Session {
    pub fn size_str(&self) -> String {
        if self.size > BYTES_PER_MB {
            format!("{:.1}MB", self.size as f64 / BYTES_PER_MB as f64)
        } else {
            format!("{}KB", self.size / 1024)
        }
    }

    pub fn display_name(&self) -> String {
        if let Some(ref name) = self.custom_name {
            if !name.trim().is_empty() { return name.clone(); }
        }
        let clean = self.first_message.replace('\n', " ");
        if clean.len() > DISPLAY_NAME_MAX_LEN {
            format!("{}...", &clean[..DISPLAY_NAME_MAX_LEN])
        } else {
            clean
        }
    }

    pub fn formatted_age(&self) -> String {
        let elapsed = SystemTime::now().duration_since(self.modified).unwrap_or_default().as_secs();
        if elapsed < 60 { format!("{}s", elapsed) }
        else if elapsed < 3600 { format!("{}m", elapsed / 60) }
        else if elapsed < 86400 { format!("{}h", elapsed / 3600) }
        else {
            let dt: chrono::DateTime<chrono::Local> = self.modified.into();
            dt.format("%d %b %y").to_string()
        }
    }

    pub fn get_todos(&self) -> Vec<String> {
        self.related_files.iter()
            .filter(|p| p.parent().map_or(false, |par| par.ends_with("todos")))
            .filter_map(|p| fs::read_to_string(p).ok())
            .filter_map(|c| serde_json::from_str::<Vec<Value>>(&c).ok())
            .flat_map(|arr| arr)
            .filter_map(|item| {
                item.get("title").or_else(|| item.get("content"))
                    .and_then(|v| v.as_str().map(String::from))
            })
            .collect()
    }
}

#[derive(Serialize, Deserialize, Clone)]
struct CachedMetadata {
    custom_name: Option<String>,
    message_count: usize,
    first_message: String,
    modified_ts: u64,
}

pub struct SessionManager {
    claude_root: PathBuf,
    cache_file: PathBuf,
    history_file: PathBuf,
}

impl SessionManager {
    pub fn new() -> Self {
        let home = dirs::home_dir().expect("Home dir not found");
        let claude_root = home.join(".claude");
        Self {
            history_file: claude_root.join("history.jsonl"),
            cache_file: claude_root.join("sessions_tui_cache.json"),
            claude_root,
        }
    }

    fn load_cache(&self) -> HashMap<String, CachedMetadata> {
        fs::File::open(&self.cache_file)
            .ok()
            .and_then(|f| serde_json::from_reader(f).ok())
            .unwrap_or_default()
    }

    pub fn load_sessions(&self) -> io::Result<Vec<Session>> {
        let projects_dir = self.claude_root.join("projects");
        if !projects_dir.exists() { return Ok(Vec::new()); }

        let cache = self.load_cache();
        let mut new_cache = HashMap::new();
        let mut sessions = Vec::new();

        for entry in fs::read_dir(projects_dir)?.flatten() {
            if !entry.path().is_dir() { continue; }
            let proj_name = entry.file_name().to_string_lossy().into_owned();

            for file in fs::read_dir(entry.path())?.flatten() {
                let path = file.path();
                if path.extension().and_then(|s| s.to_str()) != Some("jsonl") { continue; }
                
                let fname = path.file_stem().unwrap().to_string_lossy();
                if fname.starts_with("agent-") { continue; }
                let id = fname.into_owned();

                let meta = fs::metadata(&path)?;
                let mod_time = meta.modified().unwrap_or(SystemTime::now());
                let mod_ts = mod_time.duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_secs();

                let (custom_name, msg_count, first_msg) = if let Some(c) = cache.get(&id) {
                    if c.modified_ts == mod_ts {
                        new_cache.insert(id.clone(), c.clone());
                        (c.custom_name.clone(), c.message_count, c.first_message.clone())
                    } else {
                        Self::scan_and_cache(&path, &id, mod_ts, &mut new_cache)
                    }
                } else {
                    Self::scan_and_cache(&path, &id, mod_ts, &mut new_cache)
                };

                sessions.push(Session {
                    id: id.clone(),
                    path,
                    project: proj_name.clone(),
                    size: meta.len(),
                    message_count: msg_count,
                    first_message: first_msg,
                    modified: mod_time,
                    custom_name,
                    related_files: self.find_related(&id, &entry.path()),
                });
            }
        }
        
        if let Ok(f) = fs::File::create(&self.cache_file) {
            let _ = serde_json::to_writer(f, &new_cache);
        }
        
        sessions.sort_by(|a, b| b.modified.cmp(&a.modified));
        Ok(sessions)
    }

    fn scan_and_cache(path: &Path, id: &str, ts: u64, cache: &mut HashMap<String, CachedMetadata>) -> (Option<String>, usize, String) {
        let (title, count, first) = Self::scan_file(path).unwrap_or((None, 0, String::new()));
        cache.insert(id.to_string(), CachedMetadata {
            custom_name: title.clone(),
            message_count: count,
            first_message: first.clone(),
            modified_ts: ts,
        });
        (title, count, first)
    }

    fn scan_file(path: &Path) -> Option<(Option<String>, usize, String)> {
        let content = fs::read_to_string(path).ok()?;
        let mut count = 0;
        let mut first = None;
        let mut title = None;

        for line in content.lines() {
            if let Ok(val) = serde_json::from_str::<Value>(line) {
                if let Some(t) = val.get("type").and_then(|s| s.as_str()) {
                    if t == "user" {
                        if val.get("isMeta").and_then(|b| b.as_bool()).unwrap_or(false) { continue; }
                        let text = Self::extract_text(val.get("message")?.get("content")?);
                        if text.starts_with("Caveat:") || text.starts_with("<command") || text.starts_with("<local-command") { continue; }
                        count += 1;
                        if first.is_none() && !text.trim().is_empty() {
                            first = Some(text.replace('\n', " "));
                        }
                    }
                }
                if let Some(t) = val.get("customTitle").and_then(|s| s.as_str()) {
                    if !t.is_empty() { title = Some(t.to_string()); }
                }
            }
        }
        Some((title, count, first.unwrap_or_else(|| "(empty)".into())))
    }

    fn extract_text(v: &Value) -> String {
        if let Some(s) = v.as_str() { return s.to_string(); }
        if let Some(arr) = v.as_array() {
            return arr.iter()
                .filter(|i| i.get("type").and_then(|s| s.as_str()) == Some("text"))
                .filter_map(|i| i.get("text").and_then(|s| s.as_str()))
                .collect();
        }
        String::new()
    }

    fn find_related(&self, id: &str, proj: &Path) -> Vec<PathBuf> {
        let mut paths = vec![
            self.claude_root.join(format!("debug/{}.txt", id)),
            self.claude_root.join(format!("session-env/{}", id)),
            self.claude_root.join(format!("file-history/{}", id)),
        ];
        
        if let Ok(entries) = fs::read_dir(self.claude_root.join("todos")) {
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if name.starts_with(id) {
                    paths.push(e.path());
                    if let Some(aid) = name.strip_prefix(&format!("{}-agent-", id)).and_then(|s| s.strip_suffix(".json")) {
                        paths.push(proj.join(format!("agent-{}.jsonl", aid)));
                    }
                }
            }
        }
        paths.into_iter().filter(|p| p.exists()).collect()
    }

    pub fn delete_session(&self, session: &Session) -> io::Result<Vec<String>> {
        let mut files = session.related_files.clone();
        if session.path.exists() { files.push(session.path.clone()); }

        let mut deleted = Vec::new();
        for p in files {
            let name = p.strip_prefix(&self.claude_root).unwrap_or(&p).to_string_lossy().into_owned();
            if p.is_dir() { fs::remove_dir_all(&p)?; } else { fs::remove_file(&p)?; }
            deleted.push(name);
        }

        let mut cache = self.load_cache();
        if cache.remove(&session.id).is_some() {
            if let Ok(f) = fs::File::create(&self.cache_file) {
                 let _ = serde_json::to_writer(f, &cache);
            }
        }
        // Remove from history
        self.rewrite_history(|line| {
            serde_json::from_str::<Value>(line).ok()
                .and_then(|v| v.get("sessionId").and_then(|s| s.as_str()).map(|s| s == session.id))
                .unwrap_or(false)
        });

        Ok(deleted)
    }

    pub fn prune_history_orphans(&self) -> usize {
        let valid = self.get_phys_ids();
        self.rewrite_history(|line| {
            serde_json::from_str::<Value>(line).ok()
                .and_then(|v| v.get("sessionId").and_then(|s| s.as_str()).map(|s| !valid.contains(s)))
                .unwrap_or(false) // Drop if not valid
        })
    }

    fn rewrite_history<F>(&self, should_drop: F) -> usize where F: Fn(&str) -> bool {
        if !self.history_file.exists() { return 0; }
        let content = fs::read_to_string(&self.history_file).unwrap_or_default();
        let mut lines = Vec::new();
        let mut dropped = 0;
        for line in content.lines() {
            if should_drop(line) { dropped += 1; } else { lines.push(line); }
        }
        if dropped > 0 { fs::write(&self.history_file, lines.join("\n")).ok(); }
        dropped
    }

    fn get_phys_ids(&self) -> HashSet<String> {
        let mut ids = HashSet::new();
        if let Ok(projs) = fs::read_dir(self.claude_root.join("projects")) {
            for p in projs.flatten() {
                if let Ok(files) = fs::read_dir(p.path()) {
                    for f in files.flatten() {
                         let n = f.file_name().to_string_lossy().into_owned();
                         if n.ends_with(".jsonl") && !n.starts_with("agent-") {
                             ids.insert(n.replace(".jsonl", ""));
                         }
                    }
                }
            }
        }
        ids
    }

    pub fn find_orphans(&self) -> Vec<PathBuf> {
        let valid = self.get_phys_ids();
        let mut orphans = Vec::new();
        
        let mut check = |dir: &str, pred: &dyn Fn(&str) -> bool| {
             if let Ok(entries) = fs::read_dir(self.claude_root.join(dir)) {
                 for e in entries.flatten() {
                     let name = e.file_name().to_string_lossy().into_owned();
                     // Strip extension for debug
                     let stem = Path::new(&name).file_stem().and_then(|s| s.to_str()).unwrap_or(&name);
                     if pred(stem) { orphans.push(e.path()); }
                 }
             }
        };

        check("debug", &|n| !valid.contains(n) && n != "latest");
        check("session-env", &|n| !valid.contains(n));
        check("file-history", &|n| !valid.contains(n));
        check("todos", &|n| !valid.iter().any(|id| n.starts_with(id)));

        orphans
    }

    pub fn read_log(&self, path: &Path) -> String {
        fs::read_to_string(path).ok()
             .map(|c| c.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok())
                .filter_map(|v| {
                    let t = v.get("type")?.as_str()?;
                    if t != "user" && t != "assistant" { return None; }
                    let txt = Self::extract_text(v.get("message")?.get("content")?);
                    if txt.starts_with("Caveat:") || txt.starts_with("<command") || txt.starts_with("<local-command") { return None; }
                    if txt.trim().is_empty() { return None; }
                    Some(format!("\n[{}]\n{}\n", t.to_uppercase(), txt))
                }).collect::<String>())
             .unwrap_or_else(|| "Error reading log".into())
    }
}
