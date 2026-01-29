# Claude Sessions TUI

A Terminal User Interface (TUI) for browsing, managing, and maintaining Claude Code conversation sessions stored locally.

## Features

- **Session Browser**: Browse all Claude Code sessions with a clean, responsive TUI
- **Smart Filtering**: Real-time search across session names, IDs, and projects
- **Multi-Sort**: Sort by date, size, or message count (persistent preference)
- **Multi-Selection**: Select multiple sessions for batch operations
- **Session Management**:
  - Delete single or multiple sessions
  - View full conversation history with pagination
  - Export sessions to text files
- **Maintenance Tools**:
  - Detect and prune orphaned files (debug logs, environments, todos)
  - Remove empty sessions (0 messages)
  - Clean history index
- **Performance**: Intelligent caching with timestamp-based invalidation
- **Persistent Configuration**: Sort order and filter state saved across sessions

## Installation

### From Source

```bash
git clone https://github.com/yourusername/claude-sessions-tui.git
cd claude-sessions-tui
cargo build --release
```

Install to `~/.cargo/bin`:

```bash
cargo install --path .
```

### Requirements

- Rust 1.70 or later
- Claude Code sessions stored in `~/.claude/`

## Usage

```bash
claude-sessions-tui
```

### Keybindings

#### Normal Mode

| Key | Action |
|-----|--------|
| `↑`/`k` | Navigate up |
| `↓`/`j` | Navigate down |
| `Space` | Toggle session selection |
| `Enter` | View full conversation |
| `s` | Cycle sort mode (Date → Size → Messages) |
| `/` | Enter filter mode |
| `d` | Delete selected session(s) |
| `e` | Export selected session(s) to `./exports/` |
| `p` | Prune menu (empty sessions, orphaned files, history) |
| `q` | Quit application |

#### Expanded View (Conversation Reader)

| Key | Action |
|-----|--------|
| `↑`/`k` | Scroll up |
| `↓`/`j` | Scroll down |
| `PgUp` | Page up (20 lines) |
| `PgDn` | Page down (20 lines) |
| `Esc`/`q` | Return to session list |

#### Filter Mode

| Key | Action |
|-----|--------|
| `Type` | Enter search text |
| `Enter` | Apply filter |
| `Esc` | Cancel |
| `Backspace` | Delete character |

#### Confirm Mode

| Key | Action |
|-----|--------|
| `y`/`Y` | Confirm action |
| `n`/`N`/`Esc` | Cancel |

#### Prune Selection Menu

| Key | Action |
|-----|--------|
| `1` | Delete empty sessions (0 messages) |
| `2` | Delete orphaned files |
| `3` | Delete both empty + orphaned |
| `4` | Clean history.jsonl of orphaned entries |
| `Esc` | Cancel |

## Architecture

### Data Model

**Session**: Represents a single Claude Code conversation
- **ID**: Unique session identifier
- **Path**: Filesystem path to session file
- **Project**: Project name (directory)
- **Size**: File size (formatted as KB/MB)
- **Message Count**: Number of user messages (cached)
- **First Message**: Initial user prompt (used as default display name)
- **Modified**: Last modification timestamp
- **Custom Name**: User-defined title (if any)
- **Related Files**: Debug logs, environment snapshots, file history, agent logs

**SessionManager**: Handles I/O and session operations
- Discovers sessions from `~/.claude/history.jsonl`
- Cross-references with actual session files in `projects/`
- Manages related files across multiple directories
- Implements smart caching with timestamp validation
- Handles delete, export, and prune operations

**Config**: Persistent user preferences
- Sort order (Date/Size/Messages)
- Filter query
- Stored in `~/.config/claude-sessions-tui/config.json`

### File Locations

The application reads Claude Code sessions from these paths:

```
~/.claude/
├── history.jsonl                          # Global session index
├── sessions_tui_cache.json                # Metadata cache
├── projects/                              # All projects
│   └── {project-name}/                    # e.g., -home-isko-workspace
│       ├── {session-id}.jsonl             # Session logs
│       └── agent-{agent-id}.jsonl         # Agent logs
├── debug/                                 # Debug logs
│   └── {session-id}.txt
├── session-env/                           # Environment snapshots
│   └── {session-id}/
├── file-history/                          # File history
│   └── {session-id}/
└── todos/                                 # Todo and agent tracking
    └── {session-id}-agent-*.json
```

### Caching Strategy

**Purpose**: Avoid re-parsing JSONL files on every launch

**Cache File**: `~/.claude/sessions_tui_cache.json`

**Validation**:
- Compares file modification timestamp with cached timestamp
- Only re-parses if timestamps differ
- Significantly reduces I/O for large session collections

**Cache Structure**:
```json
{
  "session_id": {
    "custom_name": "My Session Name",
    "message_count": 42,
    "first_message": "Hello, Claude...",
    "modified_ts": 1704067200
  }
}
```

### Operations

**Delete**:
- Removes session `.jsonl` file
- Removes all related files (debug logs, environment, file history, todos)
- Updates cache
- Optionally removes from `history.jsonl` (via prune option 4)

**Export**:
- Creates `./exports/` directory if not exists
- Parses session JSONL
- Formats as human-readable text: `[USER]\n{content}\n\n[ASSISTANT]\n{content}`
- Writes to `{session-id}.txt`

**Prune**:
1. **Empty sessions**: Deletes sessions with 0 user messages
2. **Orphaned files**: Removes files in debug/session-env/file-history/todos without corresponding sessions
3. **Both**: Combines options 1 and 2
4. **History orphans**: Removes entries from `history.jsonl` for deleted sessions

**Filter**:
- Case-insensitive substring search across:
  - Display name (custom or first message)
  - Session ID
  - Project name

## Technical Stack

- **Language**: Rust (Edition 2021)
- **TUI Framework**: [ratatui](https://github.com/ratatui-org/ratatui) 0.29
- **Terminal I/O**: [crossterm](https://github.com/crossterm-rs/crossterm) 0.28
- **Serialization**: [serde](https://serde.rs/) 1.0 + [serde_json](https://github.com/serde-rs/json) 1.0
- **Datetime**: [chrono](https://github.com/chronotope/chrono) 0.4
- **Paths**: [dirs](https://github.com/dirs-dev/dirs-rs) 5.0

## Project Structure

```
claude-sessions-tui/
├── src/
│   ├── main.rs          # UI rendering, event loop, application state (423 lines)
│   └── sessions.rs      # Session loading, caching, file operations (368 lines)
├── Cargo.toml           # Dependencies and metadata
├── .gitignore
└── README.md
```

**Total**: ~791 lines of Rust

### Code Organization

**Domain Layer** (`sessions.rs`):
- `SessionManager`: Core business logic (discovery, caching, operations)
- `Session`: Domain entity with display formatting
- `Config`: Persistent user preferences

**Application Layer** (`main.rs`):
- `App`: Application state and orchestration
- `Mode`: State machine (Normal, Filter, Confirm, Message, PruneSelection, Expanded)
- `Action`: Command pattern for destructive operations

**Presentation Layer** (`main.rs`):
- `ui()`: Pure rendering function (ratatui widgets)
- `run_app()`: Event loop with mode-based key handling

## Development

### Building

```bash
# Debug build
cargo build

# Release build (optimized)
cargo build --release

# Run in development
cargo run
```

### Key Implementation Details

**Message Filtering** (session parsing):
- Skips messages with `isMeta: true`
- Skips messages starting with "Caveat:", "<command", or "<local-command"
- Counts only genuine user messages

**Orphan Detection**:
- Collects all valid session IDs from `projects/` directories
- Scans `debug/`, `session-env/`, `file-history/`, `todos/`
- Identifies files without matching session IDs

**Index-Based Selection**:
- `filtered: Vec<usize>` contains indices into `sessions` vector
- `selected: Vec<usize>` contains indices into `sessions` vector
- Avoids cloning large Session structs during filtering/selection

**Lazy Loading**:
- Session logs only loaded on-demand (Enter key)
- Cached in memory while viewing
- Cleared on exit

## Known Limitations

- **Hard-coded paths**: Currently reads from `~/.claude/`, not configurable
- **No horizontal scroll**: Long lines in expanded view may wrap or truncate
- **No regex filtering**: Only substring matching
- **Silent error handling**: Some file operation failures not reported to user
- **No range selection**: Cannot select multiple sessions with Shift+arrows
- **Cache schema**: No versioning, format changes break cache

## Security Considerations

This tool performs destructive file operations. Key safeguards:

1. **Confirmation dialogs**: All deletions require explicit confirmation
2. **Scope validation**: Only operates within `~/.claude/` directory
3. **Path validation**: Session IDs used in filenames should be sanitized
4. **Error handling**: IO errors are propagated, not silently ignored

**Note**: As of v1.0, there are known security issues that should be addressed in future releases. See the security audit report for details.

## Future Improvements

- Configurable Claude root directory
- Regex support in filtering
- Horizontal scroll in expanded view
- Search within conversation logs
- Export format options (JSON, Markdown)
- Range selection support
- Better error reporting in UI
- Cache schema versioning

## Version History

### v1.0.0 (2026-01-28)

Initial release featuring:
- Session browsing with filtering and sorting
- Multi-selection and batch operations
- Smart caching for performance
- Comprehensive maintenance tools (prune empty/orphaned)
- Export functionality
- Persistent configuration
- Full conversation viewer with pagination

## License

MIT

## Author

Isko

## Contributing

This is a personal tool built for managing Claude Code sessions. Contributions are welcome - please open an issue to discuss changes before submitting a PR.

## Acknowledgments

Built with:
- [ratatui](https://github.com/ratatui-org/ratatui) - Terminal UI framework
- [crossterm](https://github.com/crossterm-rs/crossterm) - Terminal manipulation
