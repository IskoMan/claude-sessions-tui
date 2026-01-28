use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const DISPLAY_NAME_MAX_LEN: usize = 60;
const BYTES_PER_KB: u64 = 1024;
const BYTES_PER_MB: u64 = 1024 * 1024;
const SECONDS_PER_DAY: u64 = 86400;

#[derive(Debug, Clone, Copy)]
pub enum SortBy {
    Date,
    Size,
    Messages,
}

#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub path: PathBuf,
    pub size: u64,
    pub message_count: usize, // Estimation or 0 if lazy loaded
    pub first_message: String,
    pub modified: SystemTime,
    pub age_days: i64,
    pub custom_name: Option<String>,
    pub related_files: Vec<PathBuf>,
}

impl Session {
    pub fn size_str(&self) -> String {
        if self.size > BYTES_PER_MB {
            format!("{:.1}MB", self.size as f64 / BYTES_PER_MB as f64)
        } else {
            format!("{}KB", self.size / BYTES_PER_KB)
        }
    }

    pub fn display_name(&self) -> String {
        if let Some(name) = &self.custom_name {
            if !name.trim().is_empty() {
                return name.clone();
            }
        }
        
        let clean_msg = self.first_message.replace('\n', " ");
        if clean_msg.len() > DISPLAY_NAME_MAX_LEN {
            format!("{}...", &clean_msg[..DISPLAY_NAME_MAX_LEN])
        } else {
            clean_msg
        }
    }
}

pub struct SessionManager {
    claude_root: PathBuf,
    project_dir: PathBuf,
    history_file: PathBuf,
}

impl SessionManager {
    pub fn new() -> Self {
        let home = dirs::home_dir().expect("Could not find home directory");
        let claude_root = home.join(".claude");
        
        // Note: This project directory seems specific to your workspace. 
        // In a generic tool this might need to be dynamic or configured.
        let project_dir = claude_root.join("projects/-home-isko-workspace");
        
        Self {
            history_file: claude_root.join("history.jsonl"),
            claude_root,
            project_dir,
        }
    }

    /// Reads ~/.claude/history.jsonl to find all session IDs and their first prompt
    fn load_history_index(&self) -> HashMap<String, (String, SystemTime)> {
        let mut index = HashMap::new();
        
        if let Ok(content) = fs::read_to_string(&self.history_file) {
            for line in content.lines() {
                if let Ok(val) = serde_json::from_str::<Value>(line) {
                    if let Some(id) = val.get("sessionId").and_then(|s| s.as_str()) {
                        // Extract prompt from "message.content"
                        let prompt = val.get("message")
                            .and_then(|m| m.get("content"))
                            .and_then(|c| c.as_str())
                            .unwrap_or("(empty prompt)")
                            .to_string();

                        // Parse timestamp if available
                        let time = val.get("timestamp")
                             .and_then(|t| t.as_str())
                             .and_then(|ts| chrono::DateTime::parse_from_rfc3339(ts).ok())
                             .map(|dt| SystemTime::from(dt))
                             .unwrap_or_else(SystemTime::now);

                        // We might have multiple entries for one session. 
                        // We typically want the "first" prompt, but the "latest" timestamp.
                        // For simplicity, we just upsert.
                        index.entry(id.to_string())
                            .and_modify(|(_, t)| {
                                if time > *t { *t = time; }
                            })
                            .or_insert((prompt, time));
                    }
                }
            }
        }
        index
    }

    /// Finds all files related to this session ID (Agents, Todos, Debug, Env)
    fn find_related_files(&self, session_id: &str) -> Vec<PathBuf> {
        let mut files = Vec::new();
        let debug_file = self.claude_root.join(format!("debug/{}.txt", session_id));
        if debug_file.exists() { files.push(debug_file); }

        let env_dir = self.claude_root.join(format!("session-env/{}", session_id));
        if env_dir.exists() { files.push(env_dir); }

        // Find Todos and collect linked Agents
        // Todo format: {sessionId}-agent-{agentId}.json
        let todo_dir = self.claude_root.join("todos");
        if let Ok(entries) = fs::read_dir(&todo_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with(session_id) {
                    files.push(entry.path());

                    // If this is a linkage to an agent, add the agent log too
                    // name looks like: <sessId>-agent-<agentId>.json
                    if let Some(rest) = name.strip_prefix(&format!("{}-agent-", session_id)) {
                        if let Some(agent_id) = rest.strip_suffix(".json") {
                            let agent_file = self.project_dir.join(format!("agent-{}.jsonl", agent_id));
                            if agent_file.exists() {
                                files.push(agent_file);
                            }
                        }
                    }
                }
            }
        }

        files
    }

    /// Scans the actual session file for a "customTitle" field in the metadata
    fn extract_custom_title(path: &Path) -> Option<String> {
        // We scan specifically for a line containing "customTitle" to avoid full parse
        // optimization: Read parsing lines as JSON
        // Note: This matches the user request: jq -r ".customTitle // empty" | tail -n 1
        let content = fs::read_to_string(path).ok()?;
        let mut title = None;

        for line in content.lines() {
            if line.contains("customTitle") {
                 if let Ok(val) = serde_json::from_str::<Value>(line) {
                     if let Some(t) = val.get("customTitle").and_then(|s| s.as_str()) {
                         if !t.is_empty() {
                            title = Some(t.to_string());
                         }
                     }
                 }
            }
        }
        title
    }

    pub fn load_sessions(&self) -> io::Result<Vec<Session>> {
        let history_index = self.load_history_index();
        let mut sessions = Vec::new();

        for (id, (first_prompt, mod_time)) in history_index {
            let path = self.project_dir.join(format!("{}.jsonl", id));
            
            // Should we include sessions that are in history but file missing?
            // Currently: NO, only valid matching files.
            if !path.exists() {
                continue;
            }

            let metadata = match fs::metadata(&path) {
                Ok(m) => m,
                Err(_) => continue,
            };
            
            // Try to find custom title inside the file
            let custom_name = Self::extract_custom_title(&path);

            let age_days = SystemTime::now()
                .duration_since(mod_time)
                .map(|d| d.as_secs() / SECONDS_PER_DAY)
                .unwrap_or(0) as i64;
            
            // Find relations
            let related_files = self.find_related_files(&id);

            sessions.push(Session {
                id,
                path,
                size: metadata.len(),
                message_count: 0, // We skip full count for speed in this new method
                first_message: first_prompt,
                modified: mod_time,
                age_days,
                custom_name,
                related_files
            });
        }

        sessions.sort_by(|a, b| b.modified.cmp(&a.modified));
        Ok(sessions)
    }

    pub fn delete_session(&self, id: &str) -> io::Result<()> {
        // 1. Re-discover related files (to be safe)
        let related = self.find_related_files(id);
        
        // 2. Delete the main session file
        let main_path = self.project_dir.join(format!("{}.jsonl", id));
        if main_path.exists() {
            fs::remove_file(main_path)?;
        }

        // 3. Delete all related files/directories
        for path in related {
            if path.is_dir() {
                fs::remove_dir_all(&path)?;
            } else if path.exists() {
                fs::remove_file(&path)?;
            }
        }

        // 4. (Optional) We do NOT remove from history.jsonl usually, 
        // as that is a global log. But the "session" is effectively gone.
        Ok(())
    }

    pub fn rename_session(&self, id: &str, name: &str) -> io::Result<()> {
        // With the new file-embedded customTitle, we would technically need to 
        // append a new metadata line to the .jsonl file.
        // For now, let's append a metadata object to the end of the file.
        
        let path = self.project_dir.join(format!("{}.jsonl", id));
        if !path.exists() { return Ok(()); }

        let update_json = serde_json::json!({
            "type": "rename", // Just a marker
            "customTitle": name,
            "timestamp": chrono::Utc::now().to_rfc3339()
        });

        // Append to file
        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .append(true)
            .open(path)?;
        
        writeln!(file, "{}", update_json.to_string())?;

        Ok(())
    }

    pub fn get_conversation_excerpt(&self, id: &str, max_messages: usize) -> io::Result<String> {
        let path = self.project_dir.join(format!("{}.jsonl", id));
        let content = fs::read_to_string(path)?;

        let messages: Vec<String> = content
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter_map(|value| {
                let msg_type = value.get("type")?.as_str()?;
                if msg_type != "user" && msg_type != "assistant" { return None; }
                let content = value.get("message")?.get("content")?.as_str()?;
                Some(format!("\n[{}]\n{}\n", msg_type.to_uppercase(), content))
            })
            .take(max_messages)
            .collect();

        if messages.is_empty() {
            Ok("No messages found".to_string())
        } else {
            Ok(messages.join(""))
        }
    }
}
