use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::{
    Terminal,
    backend::Backend,
    widgets::{ListState, TableState},
};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

use std::collections::HashMap;

use crate::ai::ChatMessage;
use crate::migration::generator::ResolvedHclMap;
use crate::migration::mapper::map_resource;
use crate::migration::types::MigrationResult;
use crate::pricing::{CostEntry, compute_costs};
use crate::terraform::parser::parse_tf_file;
use crate::terraform::scanner::find_tf_files;
use crate::terraform::types::PassthroughBlock;
use crate::todo::{TodoItem, TodoStatus, apply_resolution, scan_output_todos};
use crate::zones::{ZONES, find_zone_idx};

#[derive(Debug, Clone, PartialEq)]
pub enum View {
    Splash,
    FileBrowser,
    Scanner,
    Resources,
    Generator,
    DiffReview,
    TodoReview,
    Chat,
    Pricing,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GenStep {
    AskZone,
    AskOutputDir,
    Generating,
    Done,
}

pub enum AppMessage {
    FileFound(String),
    ScanComplete(Vec<MigrationResult>, Vec<PassthroughBlock>),
    GenerateLog(String),
    GenerateDone(usize, ResolvedHclMap),
    AiSuggestion(usize, String),
    AiError(usize, String),
    ChatResponse(String),
    ChatError(String),
    Error(String),
}

pub struct App {
    pub view: View,
    pub should_quit: bool,
    pub tick: u64,

    // Input
    pub input_buf: String,

    // Paths
    pub scan_path: Option<PathBuf>,
    pub output_path: Option<PathBuf>,

    // Scan state
    pub scan_files: Vec<String>,
    pub scan_current: Option<String>,
    pub scan_complete: bool,
    pub resources: Vec<String>, // just for count display during scan

    // Migration results
    pub migration_results: Vec<MigrationResult>,
    pub passthroughs: Vec<PassthroughBlock>,

    // Table navigation
    pub table_state: TableState,
    /// Whether the right preview panel has focus (true) or the left table (false)
    pub resources_focus_preview: bool,
    /// Scroll offset for the right preview panel
    pub preview_scroll: usize,

    // Generator state
    pub gen_step: GenStep,
    pub target_zone: String,
    pub zone_idx: usize,
    pub gen_log: Vec<String>,
    pub is_generating: bool,
    pub gen_complete: bool,
    pub gen_complete_tick: u64,
    pub gen_files_count: usize,
    pub resolved_hcl_map: ResolvedHclMap,

    // File browser state
    pub fb_cwd: PathBuf,
    pub fb_entries: Vec<(String, bool)>, // (name, is_dir)
    pub fb_state: ListState,

    // Diff review state
    pub diff_idx: usize,
    pub diff_scroll: usize,

    // TODO review state
    pub todos: Vec<TodoItem>,
    pub todo_idx: usize,
    pub todo_input: String,
    pub todo_input_active: bool,
    pub api_key: Option<String>,

    // Chat / AI advisor state
    pub chat_messages: Vec<ChatMessage>,
    pub chat_input: String,
    pub chat_scroll: usize,
    /// Last computed max scroll from the render pass (updated via interior mutability).
    pub chat_scroll_max: std::cell::Cell<u16>,
    pub chat_loading: bool,
    /// Concatenated content of generated .tf files, built once on entering the chat view.
    pub chat_tf_context: String,

    // Pricing calculator state
    pub pricing_scroll: usize,
    pub pricing_costs: Vec<CostEntry>,

    // Async channel
    tx: mpsc::Sender<AppMessage>,
    rx: mpsc::Receiver<AppMessage>,
}

impl App {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel(256);
        let mut table_state = TableState::default();
        table_state.select(Some(0));
        Self {
            view: View::Splash,
            should_quit: false,
            tick: 0,
            input_buf: String::new(),
            scan_path: None,
            output_path: None,
            scan_files: Vec::new(),
            scan_current: None,
            scan_complete: false,
            resources: Vec::new(),
            migration_results: Vec::new(),
            passthroughs: Vec::new(),
            table_state,
            resources_focus_preview: false,
            preview_scroll: 0,
            gen_step: GenStep::AskZone,
            target_zone: "de-fra1".into(),
            zone_idx: find_zone_idx("de-fra1"),
            gen_log: Vec::new(),
            is_generating: false,
            gen_complete: false,
            gen_complete_tick: 0,
            gen_files_count: 0,
            resolved_hcl_map: HashMap::new(),
            fb_cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")),
            fb_entries: Vec::new(),
            fb_state: ListState::default(),
            diff_idx: 0,
            diff_scroll: 0,
            todos: Vec::new(),
            todo_idx: 0,
            todo_input: String::new(),
            todo_input_active: false,
            api_key: std::env::var("LLM_API_KEY").ok(),
            chat_messages: Vec::new(),
            chat_input: String::new(),
            chat_scroll: 9999,
            chat_scroll_max: std::cell::Cell::new(0),
            chat_loading: false,
            chat_tf_context: String::new(),
            pricing_scroll: 0,
            pricing_costs: Vec::new(),
            tx,
            rx,
        }
    }

    pub async fn run<B: Backend>(&mut self, terminal: &mut Terminal<B>) -> Result<()> {
        let tick_rate = Duration::from_millis(50);
        let mut last_tick = Instant::now();

        loop {
            terminal.draw(|f| crate::ui::render(f, self))?;

            // Poll for crossterm events
            let timeout = tick_rate.saturating_sub(last_tick.elapsed());
            if event::poll(timeout)?
                && let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
            {
                self.handle_key(key.code).await;
            }

            // Process async messages
            while let Ok(msg) = self.rx.try_recv() {
                self.handle_message(msg);
            }

            // Tick
            if last_tick.elapsed() >= tick_rate {
                self.tick = self.tick.wrapping_add(1);
                last_tick = Instant::now();

                // Auto-advance scanner → resources when done
                if self.view == View::Scanner && self.scan_complete {
                    // Wait a moment then advance
                    if self.tick.is_multiple_of(20) {
                        self.view = View::Resources;
                    }
                }
            }

            if self.should_quit {
                break;
            }
        }

        Ok(())
    }

    async fn handle_key(&mut self, code: KeyCode) {
        match self.view {
            View::Splash => self.handle_splash_key(code).await,
            View::FileBrowser => self.handle_filebrowser_key(code).await,
            View::Scanner => self.handle_scanner_key(code),
            View::Resources => self.handle_resources_key(code).await,
            View::Generator => self.handle_generator_key(code).await,
            View::DiffReview => self.handle_diff_key(code).await,
            View::TodoReview => self.handle_todo_key(code).await,
            View::Chat => self.handle_chat_key(code).await,
            View::Pricing => self.handle_pricing_key(code),
        }
    }

    async fn handle_splash_key(&mut self, code: KeyCode) {
        match code {
            // Quit only when the input is empty; otherwise the char goes into the path.
            KeyCode::Esc => self.should_quit = true,
            KeyCode::Char('q') | KeyCode::Char('Q') if self.input_buf.is_empty() => {
                self.should_quit = true;
            }
            // File browser shortcut only when input is empty.
            KeyCode::Char('f') | KeyCode::Char('F') if self.input_buf.is_empty() => {
                self.fb_load_dir(None);
                self.view = View::FileBrowser;
            }
            KeyCode::Char(c) => self.input_buf.push(c),
            KeyCode::Backspace => {
                self.input_buf.pop();
            }
            KeyCode::Enter => {
                if !self.input_buf.is_empty() {
                    let path = PathBuf::from(self.input_buf.trim());
                    self.scan_path = Some(path.clone());
                    self.input_buf.clear();
                    self.view = View::Scanner;
                    self.start_scan(path).await;
                }
            }
            _ => {}
        }
    }

    async fn handle_filebrowser_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('q') | KeyCode::Char('Q') => self.should_quit = true,
            KeyCode::Esc => {
                self.view = View::Splash;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let len = self.fb_entries.len();
                if len > 0 {
                    let cur = self.fb_state.selected().unwrap_or(0);
                    let next = if cur == 0 { len - 1 } else { cur - 1 };
                    self.fb_state.select(Some(next));
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let len = self.fb_entries.len();
                if len > 0 {
                    let cur = self.fb_state.selected().unwrap_or(0);
                    let next = if cur + 1 >= len { 0 } else { cur + 1 };
                    self.fb_state.select(Some(next));
                }
            }
            KeyCode::Enter => {
                if let Some(idx) = self.fb_state.selected()
                    && let Some((name, is_dir)) = self.fb_entries.get(idx).cloned()
                    && is_dir
                {
                    let new_path = if name == "[..]" {
                        self.fb_cwd
                            .parent()
                            .map(|p| p.to_path_buf())
                            .unwrap_or_else(|| self.fb_cwd.clone())
                    } else {
                        self.fb_cwd.join(&name)
                    };
                    self.fb_load_dir(Some(new_path));
                }
            }
            KeyCode::Backspace => {
                let parent = self
                    .fb_cwd
                    .parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|| self.fb_cwd.clone());
                self.fb_load_dir(Some(parent));
            }
            KeyCode::Char('s') | KeyCode::Char('S') => {
                let path = self.fb_cwd.clone();
                self.scan_path = Some(path.clone());
                self.view = View::Scanner;
                self.start_scan(path).await;
            }
            _ => {}
        }
    }

    pub fn fb_load_dir(&mut self, path: Option<PathBuf>) {
        if let Some(p) = path {
            self.fb_cwd = p;
        }
        self.fb_entries.clear();
        // Add parent dir entry unless we're at root
        if self.fb_cwd.parent().is_some() {
            self.fb_entries.push(("[..]".to_string(), true));
        }

        if let Ok(read) = std::fs::read_dir(&self.fb_cwd) {
            let mut dirs: Vec<String> = Vec::new();
            let mut tf_files: Vec<String> = Vec::new();

            for entry in read.flatten() {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with('.') {
                    continue;
                }
                if path.is_dir() {
                    dirs.push(name);
                } else if name.ends_with(".tf") {
                    tf_files.push(name);
                }
            }

            dirs.sort_by_key(|a| a.to_lowercase());
            tf_files.sort();

            for d in dirs {
                self.fb_entries.push((d, true));
            }
            for f in tf_files {
                self.fb_entries.push((f, false));
            }
        }

        self.fb_state.select(if self.fb_entries.is_empty() {
            None
        } else {
            Some(0)
        });
    }

    fn handle_scanner_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('q') | KeyCode::Char('Q') => self.should_quit = true,
            _ => {}
        }
    }

    async fn handle_resources_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('q') | KeyCode::Char('Q') => self.should_quit = true,
            KeyCode::Char('g') | KeyCode::Char('G') | KeyCode::Tab => {
                self.view = View::Generator;
                self.input_buf.clear();
                self.output_path = None;
                self.gen_step = GenStep::AskZone;
                self.gen_complete = false;
                self.zone_idx = find_zone_idx(&self.target_zone);
            }
            // Panel focus switching
            KeyCode::Right | KeyCode::Char('l') if !self.resources_focus_preview => {
                self.resources_focus_preview = true;
            }
            KeyCode::Left | KeyCode::Char('h') if self.resources_focus_preview => {
                self.resources_focus_preview = false;
            }
            // Preview panel scroll
            KeyCode::Down | KeyCode::Char('j') if self.resources_focus_preview => {
                self.preview_scroll = self.preview_scroll.saturating_add(1);
            }
            KeyCode::Up | KeyCode::Char('k') if self.resources_focus_preview => {
                self.preview_scroll = self.preview_scroll.saturating_sub(1);
            }
            // Table navigation
            KeyCode::Down | KeyCode::Char('j') => {
                let i = match self.table_state.selected() {
                    Some(i) => {
                        if i >= self.migration_results.len().saturating_sub(1) {
                            0
                        } else {
                            i + 1
                        }
                    }
                    None => 0,
                };
                self.table_state.select(Some(i));
                self.preview_scroll = 0;
                self.resources_focus_preview = false;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let i = match self.table_state.selected() {
                    Some(i) => {
                        if i == 0 {
                            self.migration_results.len().saturating_sub(1)
                        } else {
                            i - 1
                        }
                    }
                    None => 0,
                };
                self.table_state.select(Some(i));
                self.preview_scroll = 0;
                self.resources_focus_preview = false;
            }
            _ => {}
        }
    }

    async fn handle_generator_key(&mut self, code: KeyCode) {
        // When the output directory input is active, ALL keys go to the input field.
        // Commands are blocked to prevent 'q', 'd', 't', etc. from firing while typing.
        if self.gen_step == GenStep::AskOutputDir {
            match code {
                KeyCode::Char(c) => {
                    self.input_buf.push(c);
                }
                KeyCode::Backspace => {
                    self.input_buf.pop();
                }
                KeyCode::Enter => {
                    if !self.input_buf.trim().is_empty() {
                        let out = PathBuf::from(self.input_buf.trim());
                        self.output_path = Some(out.clone());
                        self.input_buf.clear();
                        self.gen_step = GenStep::Generating;
                        self.start_generation(out).await;
                    }
                }
                KeyCode::Esc => {
                    // Cancel back to zone selection.
                    self.input_buf.clear();
                    self.gen_step = GenStep::AskZone;
                }
                _ => {}
            }
            return;
        }

        match code {
            KeyCode::Char('q') | KeyCode::Char('Q') => self.should_quit = true,
            KeyCode::Tab | KeyCode::BackTab => self.view = View::Resources,
            KeyCode::Char('t') | KeyCode::Char('T') if self.gen_complete => {
                if let Some(output_dir) = &self.output_path {
                    self.todos = scan_output_todos(output_dir);
                    self.todo_idx = 0;
                    self.todo_input.clear();
                    self.view = View::TodoReview;
                }
            }
            KeyCode::Char('d') | KeyCode::Char('D') if self.gen_complete => {
                self.diff_idx = 0;
                self.diff_scroll = 0;
                self.view = View::DiffReview;
            }
            KeyCode::Char('c') | KeyCode::Char('C') if self.gen_complete => {
                self.enter_chat_view().await;
            }
            KeyCode::Char('p') | KeyCode::Char('P') if self.gen_complete => {
                self.pricing_costs = compute_costs(
                    &self.migration_results,
                    &self.resolved_hcl_map,
                    &self.passthroughs,
                );
                self.pricing_scroll = 0;
                self.view = View::Pricing;
            }
            // Zone picker navigation
            KeyCode::Up | KeyCode::Char('k') if self.gen_step == GenStep::AskZone => {
                if self.zone_idx > 0 {
                    self.zone_idx -= 1;
                }
            }
            KeyCode::Down | KeyCode::Char('j') if self.gen_step == GenStep::AskZone => {
                if self.zone_idx + 1 < ZONES.len() {
                    self.zone_idx += 1;
                }
            }

            KeyCode::Enter if self.gen_step == GenStep::AskZone => {
                self.target_zone = ZONES[self.zone_idx].slug.to_string();
                self.gen_step = GenStep::AskOutputDir;
            }

            _ => {}
        }
    }

    async fn handle_todo_key(&mut self, code: KeyCode) {
        // When in text-input mode, ALL keys go to the input field.
        // Esc cancels; Enter applies. Tab still navigates away.
        if self.todo_input_active {
            match code {
                KeyCode::Esc => {
                    self.todo_input.clear();
                    self.todo_input_active = false;
                }
                KeyCode::Tab | KeyCode::BackTab => {
                    self.todo_input_active = false;
                    self.view = View::Generator;
                }
                KeyCode::Backspace => {
                    self.todo_input.pop();
                }
                KeyCode::Char(c) => {
                    self.todo_input.push(c);
                }
                KeyCode::Enter => {
                    self.apply_todo_resolution().await;
                    self.todo_input_active = false;
                }
                _ => {}
            }
            return;
        }

        // Input is empty — single-char commands are active.
        match code {
            KeyCode::Char('q') | KeyCode::Char('Q') => self.should_quit = true,
            KeyCode::Tab | KeyCode::BackTab => self.view = View::Generator,

            KeyCode::Down | KeyCode::Char('j') => {
                if !self.todos.is_empty() {
                    self.todo_idx = (self.todo_idx + 1) % self.todos.len();
                    self.todo_input.clear();
                    self.todo_input_active = false;
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if !self.todos.is_empty() {
                    self.todo_idx = if self.todo_idx == 0 {
                        self.todos.len() - 1
                    } else {
                        self.todo_idx - 1
                    };
                    self.todo_input.clear();
                    self.todo_input_active = false;
                }
            }
            // [N] jump to next pending (unresolved/unskipped) todo
            KeyCode::Char('n') | KeyCode::Char('N') => {
                if !self.todos.is_empty() {
                    let len = self.todos.len();
                    let start = (self.todo_idx + 1) % len;
                    for offset in 0..len {
                        let idx = (start + offset) % len;
                        if self.todos[idx].status == TodoStatus::Pending {
                            self.todo_idx = idx;
                            self.todo_input.clear();
                            self.todo_input_active = false;
                            break;
                        }
                    }
                }
            }

            // AI suggestion
            KeyCode::Char('a') | KeyCode::Char('A') => {
                if let (Some(api_key), Some(item)) =
                    (self.api_key.clone(), self.todos.get_mut(self.todo_idx))
                    && (item.status == TodoStatus::Pending || item.status == TodoStatus::Resolved)
                {
                    item.status = TodoStatus::Loading;
                    let item_clone = item.clone();
                    let idx = self.todo_idx;
                    let tx = self.tx.clone();
                    tokio::spawn(async move {
                        match crate::ai::get_todo_suggestion(&item_clone, &api_key).await {
                            Ok(s) => {
                                let _ = tx.send(AppMessage::AiSuggestion(idx, s)).await;
                            }
                            Err(e) => {
                                let _ = tx.send(AppMessage::AiError(idx, e.to_string())).await;
                            }
                        }
                    });
                }
            }

            // Skip
            KeyCode::Char('s') | KeyCode::Char('S') => {
                if let Some(item) = self.todos.get_mut(self.todo_idx) {
                    item.status = TodoStatus::Skipped;
                    self.todo_input.clear();
                    self.todo_input_active = false;
                    // Advance to next pending
                    if self.todo_idx + 1 < self.todos.len() {
                        self.todo_idx += 1;
                    }
                }
            }

            // Enter activates the text input field (insert mode).
            // A second Enter (when active) applies the resolution — handled in the active block above.
            KeyCode::Enter => {
                self.todo_input_active = true;
            }

            _ => {}
        }
    }

    async fn apply_todo_resolution(&mut self) {
        let resolution = if !self.todo_input.is_empty() {
            Some(self.todo_input.clone())
        } else {
            self.todos
                .get(self.todo_idx)
                .and_then(|i| i.ai_suggestion.clone())
        };
        // Strip markdown backtick wrapping that AI models sometimes emit (e.g. `value`).
        let resolution = resolution.map(|r| {
            let t = r.trim();
            if t.starts_with('`') && t.ends_with('`') && t.len() > 1 {
                t[1..t.len() - 1].to_string()
            } else {
                t.to_string()
            }
        });

        if let (Some(res), Some(output_dir)) = (resolution, self.output_path.clone())
            && let Some(item) = self.todos.get_mut(self.todo_idx)
        {
            let _ = apply_resolution(&output_dir, item, &res);
            item.resolution = Some(res.clone());
            item.status = TodoStatus::Resolved;
            self.todo_input.clear();
            self.todo_input_active = false;
            let next = (self.todo_idx + 1).min(self.todos.len().saturating_sub(1));
            if next > self.todo_idx || self.todos.len() == 1 {
                self.todo_idx = next;
            }
        }
    }

    fn handle_message(&mut self, msg: AppMessage) {
        match msg {
            AppMessage::FileFound(path) => {
                self.scan_files.push(path.clone());
                self.scan_current = Some(path);
            }
            AppMessage::ScanComplete(results, passthroughs) => {
                self.resources = results.iter().map(|r| r.resource_type.clone()).collect();
                self.migration_results = results;
                self.passthroughs = passthroughs;
                self.scan_complete = true;
                self.table_state.select(Some(0));
            }
            AppMessage::GenerateLog(line) => {
                self.gen_log.push(line);
            }
            AppMessage::GenerateDone(count, resolved_map) => {
                self.is_generating = false;
                self.gen_complete = true;
                self.gen_complete_tick = self.tick;
                self.gen_step = GenStep::Done;
                self.gen_files_count = count;
                self.resolved_hcl_map = resolved_map;
            }
            AppMessage::AiSuggestion(idx, suggestion) => {
                if let Some(item) = self.todos.get_mut(idx) {
                    item.ai_suggestion = Some(suggestion);
                    item.status = TodoStatus::Pending;
                }
            }
            AppMessage::AiError(idx, err) => {
                if let Some(item) = self.todos.get_mut(idx) {
                    item.ai_suggestion = Some(format!("[AI error: {}]", err));
                    item.status = TodoStatus::Pending;
                }
            }
            AppMessage::ChatResponse(text) => {
                self.chat_loading = false;
                self.chat_messages.push(ChatMessage::ai(text));
                self.chat_scroll = 9999;
            }
            AppMessage::ChatError(err) => {
                self.chat_loading = false;
                self.chat_messages
                    .push(ChatMessage::ai(format!("[Error: {}]", err)));
                self.chat_scroll = 9999;
            }
            AppMessage::Error(e) => {
                self.gen_log.push(format!("[ERR] {}", e));
            }
        }
    }

    async fn start_scan(&self, path: PathBuf) {
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let tf_files = match find_tf_files(&path) {
                Ok(files) => files,
                Err(e) => {
                    let _ = tx
                        .send(AppMessage::Error(format!("Scan error: {}", e)))
                        .await;
                    return;
                }
            };

            // Parse each file and map resources
            let mut all_results: Vec<MigrationResult> = Vec::new();
            let mut all_passthroughs: Vec<PassthroughBlock> = Vec::new();
            for file in &tf_files {
                let display = file.display().to_string();
                let _ = tx.send(AppMessage::FileFound(display)).await;

                // Small yield to let UI update
                tokio::time::sleep(tokio::time::Duration::from_millis(30)).await;

                match parse_tf_file(file) {
                    Ok(tf_file) => {
                        for res in &tf_file.resources {
                            let result = map_resource(res);
                            all_results.push(result);
                        }
                        all_passthroughs.extend(tf_file.passthroughs);
                    }
                    Err(e) => {
                        let _ = tx
                            .send(AppMessage::Error(format!(
                                "Parse error in {}: {}",
                                file.display(),
                                e
                            )))
                            .await;
                    }
                }
            }

            let _ = tx
                .send(AppMessage::ScanComplete(all_results, all_passthroughs))
                .await;
        });
    }

    async fn start_generation(&mut self, output_dir: PathBuf) {
        self.is_generating = true;
        self.gen_log.clear();
        self.gen_log.push(format!(">> Zone: {}", self.target_zone));
        self.gen_log
            .push(format!(">> Generating to: {}", output_dir.display()));

        let tx = self.tx.clone();
        let results = self.migration_results.clone();
        let passthroughs = self.passthroughs.clone();
        let zone = self.target_zone.clone();
        let source_dir = self.scan_path.clone();

        tokio::spawn(async move {
            let mut log: Vec<String> = Vec::new();
            match crate::migration::generator::generate_files(
                &results,
                &passthroughs,
                &output_dir,
                source_dir.as_deref(),
                &zone,
                &mut log,
            ) {
                Ok((count, resolved_map)) => {
                    for line in log {
                        let _ = tx.send(AppMessage::GenerateLog(line)).await;
                    }
                    // Run `terraform fmt` if available.
                    match std::process::Command::new("terraform")
                        .args(["fmt", output_dir.to_str().unwrap_or(".")])
                        .output()
                    {
                        Ok(out) if out.status.success() => {
                            let _ = tx
                                .send(AppMessage::GenerateLog("[terraform fmt] OK".into()))
                                .await;
                        }
                        Ok(out) => {
                            let stderr = String::from_utf8_lossy(&out.stderr);
                            let _ = tx
                                .send(AppMessage::GenerateLog(format!(
                                    "[terraform fmt] {}",
                                    stderr.trim()
                                )))
                                .await;
                        }
                        Err(_) => {
                            let _ = tx
                                .send(AppMessage::GenerateLog(
                                    "[terraform fmt] not found — skipping".into(),
                                ))
                                .await;
                        }
                    }
                    let _ = tx.send(AppMessage::GenerateDone(count, resolved_map)).await;
                }
                Err(e) => {
                    let _ = tx
                        .send(AppMessage::Error(format!("Generation failed: {}", e)))
                        .await;
                }
            }
        });
    }

    fn handle_pricing_key(&mut self, code: KeyCode) {
        let count = self.pricing_costs.len();
        match code {
            KeyCode::Char('q') | KeyCode::Char('Q') => self.should_quit = true,
            KeyCode::Esc | KeyCode::Tab | KeyCode::BackTab => self.view = View::Generator,
            KeyCode::Up | KeyCode::Char('k') => {
                self.pricing_scroll = self.pricing_scroll.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                // Max scroll is capped in the render function; we just add here and let render clamp
                if count > 0 {
                    self.pricing_scroll = self
                        .pricing_scroll
                        .saturating_add(1)
                        .min(count.saturating_sub(1));
                }
            }
            _ => {}
        }
    }

    async fn handle_diff_key(&mut self, code: KeyCode) {
        let total = self.migration_results.len();
        match code {
            KeyCode::Char('q') | KeyCode::Char('Q') => self.should_quit = true,
            KeyCode::Esc | KeyCode::Backspace => self.view = View::Generator,
            KeyCode::Left | KeyCode::Char('h') | KeyCode::Char('H') => {
                if total > 0 {
                    self.diff_idx = if self.diff_idx == 0 {
                        total - 1
                    } else {
                        self.diff_idx - 1
                    };
                    self.diff_scroll = 0;
                }
            }
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Char('L') => {
                if total > 0 {
                    self.diff_idx = (self.diff_idx + 1) % total;
                    self.diff_scroll = 0;
                }
            }
            KeyCode::Up | KeyCode::Char('k') | KeyCode::Char('K') => {
                self.diff_scroll = self.diff_scroll.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Char('J') => {
                self.diff_scroll = self.diff_scroll.saturating_add(1);
            }
            _ => {}
        }
    }

    // AI Chat

    /// Build a truncated string of all generated .tf file contents for AI context.
    fn build_tf_context(&self) -> String {
        const MAX_CHARS: usize = 20_000;
        let Some(output_dir) = &self.output_path else {
            return String::new();
        };

        let mut ctx = String::new();
        if let Ok(entries) = std::fs::read_dir(output_dir) {
            let mut paths: Vec<_> = entries
                .flatten()
                .filter(|e| e.path().extension().is_some_and(|x| x == "tf"))
                .collect();
            paths.sort_by_key(|e| e.file_name());
            for entry in paths {
                if let Ok(content) = std::fs::read_to_string(entry.path()) {
                    let header = format!("# --- {} ---\n", entry.file_name().to_string_lossy());
                    // Check if adding this file would exceed the limit
                    if ctx.len() + header.len() + content.len() + 1 > MAX_CHARS {
                        break;
                    }
                    ctx.push_str(&header);
                    ctx.push_str(&content);
                    ctx.push('\n');
                }
            }
        }
        ctx
    }

    async fn enter_chat_view(&mut self) {
        self.chat_tf_context = self.build_tf_context();
        self.chat_messages.clear();
        self.chat_input.clear();
        self.chat_loading = false;
        self.chat_scroll = 9999;
        self.view = View::Chat;

        if let Some(_api_key) = &self.api_key {
            self.chat_messages.push(ChatMessage::user(
                "Briefly introduce yourself and summarise the generated Terraform in 2-3 sentences.",
            ));
            self.chat_loading = true;
            self.start_chat_message().await;
        }
    }

    async fn start_chat_message(&mut self) {
        let Some(api_key) = self.api_key.clone() else {
            return;
        };
        let messages = self.chat_messages.clone();
        let tf_context = self.chat_tf_context.clone();
        let tx = self.tx.clone();

        tokio::spawn(async move {
            match crate::ai::chat_with_tf(&messages, &tf_context, &api_key).await {
                Ok(resp) => {
                    let _ = tx.send(AppMessage::ChatResponse(resp)).await;
                }
                Err(e) => {
                    let _ = tx.send(AppMessage::ChatError(e.to_string())).await;
                }
            }
        });
    }

    async fn handle_chat_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('q') | KeyCode::Char('Q') if self.chat_input.is_empty() => {
                self.should_quit = true;
            }
            KeyCode::Esc | KeyCode::Tab | KeyCode::BackTab => {
                self.view = View::Generator;
            }
            KeyCode::Up | KeyCode::Char('k') if self.chat_input.is_empty() => {
                let max = self.chat_scroll_max.get() as usize;
                let effective = self.chat_scroll.min(max);
                self.chat_scroll = effective.saturating_sub(3);
            }
            KeyCode::Down | KeyCode::Char('j') if self.chat_input.is_empty() => {
                let max = self.chat_scroll_max.get() as usize;
                let effective = self.chat_scroll.min(max);
                if effective >= max {
                    self.chat_scroll = 9999;
                } else {
                    self.chat_scroll = (effective + 3).min(max);
                }
            }
            KeyCode::Char(c) => {
                self.chat_input.push(c);
            }
            KeyCode::Backspace => {
                self.chat_input.pop();
            }
            KeyCode::Enter => {
                let msg = self.chat_input.trim().to_string();
                if !msg.is_empty() && !self.chat_loading {
                    self.chat_input.clear();
                    self.chat_messages.push(ChatMessage::user(&msg));
                    self.chat_loading = true;
                    self.chat_scroll = 9999;
                    self.start_chat_message().await;
                }
            }
            _ => {}
        }
    }
}
