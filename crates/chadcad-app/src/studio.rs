//! Application state and the background engine thread.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use chadcad_ai::{Conversation, Event as AiEvent, Validator};
use chadcad_i18n::I18n;
use chadcad_kernel::Format;
use chadcad_model::{Document, FeatureId};
use chadcad_timeline::{BuildResult, Engine};
use serde_json::Value;

/// Display mesh deflection, in millimetres.
pub const DISPLAY_CHORD: f64 = 0.05;
/// Export mesh deflection, in millimetres.
pub const EXPORT_CHORD: f64 = 0.01;

const UNDO_DEPTH: usize = 200;

enum Job {
    Build(Box<Document>),
    Export { document: Box<Document>, path: PathBuf, format: Format },
}

pub enum Done {
    Built(Box<BuildResult>),
    Exported(Result<PathBuf, String>),
}

/// Runs the engine off the main thread. Edits queue faster than a rebuild can
/// finish, so a queued build is dropped in favour of a newer one.
fn worker(jobs: Receiver<Job>, done: Sender<Done>) {
    let mut engine = Engine::new();
    while let Ok(mut job) = jobs.recv() {
        while let (Job::Build(_), Ok(newer)) = (&job, jobs.try_recv()) {
            job = newer;
        }
        let message = match job {
            Job::Build(document) => Done::Built(Box::new(engine.build(&document, DISPLAY_CHORD))),
            Job::Export { document, path, format } => {
                let result = chadcad_timeline::export_document(&document, format, EXPORT_CHORD)
                    .and_then(|bytes| std::fs::write(&path, bytes).map_err(|e| e.to_string()))
                    .map(|()| path);
                Done::Exported(result)
            }
        };
        if done.send(message).is_err() {
            return;
        }
    }
}

pub struct ChatLine {
    pub from_user: bool,
    pub text: String,
}

pub struct AiPanel {
    pub model: String,
    pub input: String,
    pub lines: Vec<ChatLine>,
    conversation: Conversation,
    running: Option<(Mutex<Receiver<AiEvent>>, Mutex<Receiver<Conversation>>)>,
    validator: Validator,
}

#[derive(Default, PartialEq, Clone, Copy)]
pub enum SideTab {
    #[default]
    Ai,
    Lua,
}

#[derive(Resource)]
pub struct Studio {
    pub doc: Document,
    pub path: Option<PathBuf>,
    pub dirty: bool,
    undo: Vec<Document>,
    redo: Vec<Document>,
    pub selected: Option<FeatureId>,
    /// The selected feature's fields as JSON, being edited.
    pub draft: Option<Value>,
    pub param_drafts: Vec<(String, String)>,
    pub new_param: String,
    pub build: BuildResult,
    pub building: bool,
    pub status: String,
    pub tr: Arc<I18n>,
    pub tab: SideTab,
    pub lua_source: String,
    pub lua_output: String,
    pub ai: AiPanel,
    /// The scene needs rebuilding from `build`.
    pub scene_stale: bool,
    pub fit_requested: bool,
    /// Physical viewport rectangle, left over from the panels.
    pub viewport: Rect,
    pub pointer_on_ui: bool,
    jobs: Sender<Job>,
    // Resources must be Sync; the mutexes are only ever taken through `&mut self`.
    done: Mutex<Receiver<Done>>,
}

fn validator() -> Validator {
    let engine = Mutex::new(Engine::new());
    Arc::new(move |document: &Document| match engine.lock() {
        Ok(mut engine) => {
            let result = engine.build(document, DISPLAY_CHORD);
            chadcad_timeline::describe(document, &result)
        }
        Err(_) => "the engine is unavailable".into(),
    })
}

impl Studio {
    pub fn new(doc: Document, path: Option<PathBuf>) -> Self {
        let (jobs, job_rx) = mpsc::channel();
        let (done_tx, done) = mpsc::channel();
        std::thread::spawn(move || worker(job_rx, done_tx));
        let tr = Arc::new(I18n::detect());
        let mut studio = Self {
            doc,
            path,
            dirty: false,
            undo: Vec::new(),
            redo: Vec::new(),
            selected: None,
            draft: None,
            param_drafts: Vec::new(),
            new_param: String::new(),
            build: BuildResult::default(),
            building: false,
            status: tr.t("status-ready"),
            tr,
            tab: SideTab::default(),
            lua_source: "cad.param(\"size\", 30)\ncad.box{size={\"size\", \"size\", 10}}\n".into(),
            lua_output: String::new(),
            ai: AiPanel {
                model: std::env::var("CHADCAD_MODEL")
                    .unwrap_or_else(|_| chadcad_ai::DEFAULT_MODEL.to_owned()),
                input: String::new(),
                lines: Vec::new(),
                conversation: Conversation::new(),
                running: None,
                validator: validator(),
            },
            scene_stale: false,
            fit_requested: true,
            viewport: Rect::default(),
            pointer_on_ui: false,
            jobs,
            done: Mutex::new(done),
        };
        studio.rebuild();
        studio
    }

    pub fn ai_running(&self) -> bool {
        self.ai.running.is_some()
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    fn rebuild(&mut self) {
        self.building = self.jobs.send(Job::Build(Box::new(self.doc.clone()))).is_ok();
        self.refresh_drafts();
    }

    /// Re-reads the editor buffers from the document.
    pub fn refresh_drafts(&mut self) {
        self.param_drafts =
            self.doc.parameters.iter().map(|p| (p.name.clone(), p.expr.0.clone())).collect();
        self.draft = self
            .selected
            .and_then(|id| self.doc.feature(id))
            .and_then(|f| serde_json::to_value(&f.kind).ok());
    }

    pub fn select(&mut self, id: Option<FeatureId>) {
        self.selected = id;
        self.refresh_drafts();
    }

    /// Applies an edit as one undo step.
    pub fn edit(&mut self, change: impl FnOnce(&mut Document)) {
        let mut next = self.doc.clone();
        change(&mut next);
        self.replace(next);
    }

    /// Replaces the document as one undo step, if it differs.
    pub fn replace(&mut self, next: Document) {
        if next == self.doc {
            return;
        }
        self.undo.push(std::mem::replace(&mut self.doc, next));
        if self.undo.len() > UNDO_DEPTH {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.dirty = true;
        if self.selected.is_some_and(|id| self.doc.feature(id).is_none()) {
            self.selected = None;
        }
        self.rebuild();
    }

    pub fn undo(&mut self) {
        if let Some(previous) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.doc, previous));
            self.dirty = true;
            self.rebuild();
        }
    }

    pub fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.doc, next));
            self.dirty = true;
            self.rebuild();
        }
    }

    pub fn new_document(&mut self) {
        self.open_document(Document::default(), None);
    }

    pub fn open_document(&mut self, doc: Document, path: Option<PathBuf>) {
        self.doc = doc;
        self.path = path;
        self.dirty = false;
        self.undo.clear();
        self.redo.clear();
        self.selected = None;
        self.fit_requested = true;
        self.rebuild();
    }

    pub fn open_path(&mut self, path: PathBuf) {
        let loaded = std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|text| Document::from_json(&text).map_err(|e| e.to_string()));
        match loaded {
            Ok(doc) => self.open_document(doc, Some(path)),
            Err(error) => self.status = self.tr.t_with("status-failed", &[("error", error)]),
        }
    }

    pub fn save_to(&mut self, path: PathBuf) {
        let written = self
            .doc
            .to_json()
            .map_err(|e| e.to_string())
            .and_then(|json| std::fs::write(&path, json).map_err(|e| e.to_string()));
        self.status = match written {
            Ok(()) => {
                self.dirty = false;
                let shown = path.display().to_string();
                self.path = Some(path);
                self.tr.t_with("status-saved", &[("path", shown)])
            }
            Err(error) => self.tr.t_with("status-failed", &[("error", error)]),
        };
    }

    pub fn export(&mut self, path: PathBuf, format: Format) {
        let document = Box::new(self.doc.clone());
        let _ = self.jobs.send(Job::Export { document, path, format });
        self.status = self.tr.t("status-exporting");
    }

    pub fn run_lua(&mut self) {
        let outcome = chadcad_lua::run_document(&self.lua_source, &self.doc);
        self.lua_output = outcome.output;
        if let Some(error) = outcome.error {
            self.lua_output.push_str(&error);
        }
        if let Some(document) = outcome.document {
            self.replace(document);
        }
    }

    pub fn send_to_ai(&mut self) {
        let prompt = std::mem::take(&mut self.ai.input);
        if prompt.trim().is_empty() || self.ai_running() {
            return;
        }
        self.ai.lines.push(ChatLine { from_user: true, text: prompt.clone() });
        let (events, conversation) = chadcad_ai::spawn_turn(
            self.ai.model.clone(),
            self.ai.conversation.clone(),
            prompt,
            self.doc.clone(),
            self.ai.validator.clone(),
        );
        self.ai.running = Some((Mutex::new(events), Mutex::new(conversation)));
    }

    fn poll_ai(&mut self) {
        let Some((events, _)) = &mut self.ai.running else { return };
        let Ok(events) = events.get_mut() else { return };
        let events: Vec<AiEvent> = events.try_iter().collect();
        for event in events {
            match event {
                AiEvent::Text(text) => self.ai.lines.push(ChatLine { from_user: false, text }),
                AiEvent::Tool { script, output } => {
                    let tool = self.tr.t("ai-ran-script");
                    let text = format!("{tool}\n{script}\n> {output}");
                    self.ai.lines.push(ChatLine { from_user: false, text });
                }
                AiEvent::Document(document) => self.replace(*document),
                AiEvent::Finished(error) => {
                    if let Some(error) = error {
                        let text = self.tr.t_with("ai-failed", &[("error", error)]);
                        self.ai.lines.push(ChatLine { from_user: false, text });
                    }
                    if let Some((_, conversation)) = self.ai.running.take()
                        && let Ok(conversation) = conversation.into_inner()
                        && let Ok(conversation) = conversation.try_recv()
                    {
                        self.ai.conversation = conversation;
                    }
                }
            }
        }
    }

    fn poll_worker(&mut self) {
        let Ok(done) = self.done.get_mut() else { return };
        let finished: Vec<Done> = done.try_iter().collect();
        for done in finished {
            match done {
                Done::Built(result) => {
                    self.building = false;
                    self.build = *result;
                    self.scene_stale = true;
                }
                Done::Exported(result) => {
                    self.status = match result {
                        Ok(path) => {
                            let path = path.display().to_string();
                            self.tr.t_with("status-exported", &[("path", path)])
                        }
                        Err(error) => self.tr.t_with("status-failed", &[("error", error)]),
                    };
                }
            }
        }
    }
}

pub fn poll(mut studio: ResMut<Studio>) {
    studio.poll_worker();
    studio.poll_ai();
}
