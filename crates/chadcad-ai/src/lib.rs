//! The AI assistant, built on the multi-provider `genai` crate.
//!
//! The model edits the document only through the sandboxed Lua API: every
//! change is an ordinary script, and each one reports back how the timeline
//! built, so the model can see and fix its own mistakes.
//!
//! Keys come from the provider's usual environment variable (`ANTHROPIC_API_KEY`,
//! `OPENAI_API_KEY`, `GEMINI_API_KEY`, ...) and are never stored. Prefix a local
//! model with `ollama::` to use Ollama, which needs no key.

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};

use chadcad_model::Document;
use genai::Client;
use genai::chat::{ChatMessage, ChatRequest, Tool, ToolCall, ToolResponse};
use serde_json::{Value, json};

pub const DEFAULT_MODEL: &str = "claude-opus-5-5";
const MAX_ROUNDS: usize = 12;

/// Builds a document and describes the outcome, for the model to read.
pub type Validator = Arc<dyn Fn(&Document) -> String + Send + Sync>;

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// A script the model ran, and what it printed or why it failed.
    Tool { script: String, output: String },
    /// The document after a successful edit.
    Document(Box<Document>),
    /// The model's reply.
    Text(String),
    /// The turn ended; the error, if it failed.
    Finished(Option<String>),
}

fn system_prompt() -> String {
    format!(
        "You are the modelling assistant inside ChadCAD, a parametric CAD application. \
         Change the user's model by calling run_lua; call get_document first when you need \
         to see the current state. Prefer named parameters and expressions over literal \
         numbers, so the user can tune the part afterwards. After each run_lua, read the \
         build report and fix any feature errors before replying. Reply briefly, in the \
         user's language, saying what you changed.\n\n{}",
        chadcad_lua::API_REFERENCE
    )
}

fn tools() -> Vec<Tool> {
    vec![
        Tool::new("get_document")
            .with_description("Returns the current document (parameters and features) as JSON.")
            .with_schema(json!({"type": "object", "properties": {}})),
        Tool::new("run_lua")
            .with_description(
                "Runs a Lua script against the document using the `cad` API. A script that \
                 raises an error changes nothing. Returns printed output and a build report.",
            )
            .with_schema(json!({
                "type": "object",
                "properties": {"script": {"type": "string", "description": "Lua source"}},
                "required": ["script"]
            })),
    ]
}

/// Runs one tool call, updating `doc` on success. Returns the text for the model.
pub fn execute_tool(
    name: &str,
    arguments: &Value,
    doc: &mut Document,
    validate: &Validator,
    events: &dyn Fn(Event),
) -> String {
    match name {
        "get_document" => doc.to_json().unwrap_or_else(|e| format!("error: {e}")),
        "run_lua" => {
            let Some(script) = arguments.get("script").and_then(Value::as_str) else {
                return "error: run_lua needs a `script` string".into();
            };
            let outcome = chadcad_lua::run_document(script, doc);
            let mut report = outcome.output;
            match (outcome.document, outcome.error) {
                (Some(edited), _) => {
                    *doc = edited;
                    report.push_str(&validate(doc));
                    events(Event::Document(Box::new(doc.clone())));
                }
                (None, error) => {
                    let error = error.unwrap_or_default();
                    report.push_str(&format!("script failed, nothing changed: {error}"));
                }
            }
            events(Event::Tool { script: script.to_owned(), output: report.clone() });
            report
        }
        other => format!("error: unknown tool `{other}`"),
    }
}

/// A conversation, kept across turns.
#[derive(Clone)]
pub struct Conversation {
    request: ChatRequest,
}

impl Default for Conversation {
    fn default() -> Self {
        Self { request: ChatRequest::from_system(system_prompt()).with_tools(tools()) }
    }
}

impl Conversation {
    pub fn new() -> Self {
        Self::default()
    }
}

async fn turn(
    client: &Client,
    model: &str,
    conversation: &mut Conversation,
    doc: &mut Document,
    validate: &Validator,
    events: &dyn Fn(Event),
) -> Result<(), String> {
    for _ in 0..MAX_ROUNDS {
        let response = client
            .exec_chat(model, conversation.request.clone(), None)
            .await
            .map_err(|e| e.to_string())?;
        let text = response.first_text().map(str::to_owned);
        let calls: Vec<ToolCall> = response.into_tool_calls();
        if let Some(text) = text.filter(|t| !t.trim().is_empty()) {
            events(Event::Text(text.clone()));
            if calls.is_empty() {
                conversation.request =
                    conversation.request.clone().append_message(ChatMessage::assistant(text));
                return Ok(());
            }
        }
        if calls.is_empty() {
            return Ok(());
        }
        let mut request = conversation.request.clone().append_message(calls.clone());
        for call in &calls {
            let reply = execute_tool(&call.fn_name, &call.fn_arguments, doc, validate, events);
            request = request.append_message(ToolResponse::new(call.call_id.clone(), reply));
        }
        conversation.request = request;
    }
    Err(format!("stopped after {MAX_ROUNDS} rounds without finishing"))
}

/// Starts a turn on its own thread. Events arrive on the returned channel, and
/// the updated conversation comes back with the final event's sender dropped.
pub fn spawn_turn(
    model: String,
    mut conversation: Conversation,
    prompt: String,
    mut doc: Document,
    validate: Validator,
) -> (Receiver<Event>, Receiver<Conversation>) {
    let (tx, rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let send = |tx: &Sender<Event>, e| {
            let _ = tx.send(e);
        };
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build();
        let runtime = match runtime {
            Ok(r) => r,
            Err(e) => return send(&tx, Event::Finished(Some(e.to_string()))),
        };
        conversation.request =
            conversation.request.clone().append_message(ChatMessage::user(prompt));
        let client = match Client::new() {
            Ok(c) => c,
            Err(e) => return send(&tx, Event::Finished(Some(e.to_string()))),
        };
        let events = |e| send(&tx, e);
        let result = runtime.block_on(turn(
            &client,
            &model,
            &mut conversation,
            &mut doc,
            &validate,
            &events,
        ));
        let _ = done_tx.send(conversation);
        send(&tx, Event::Finished(result.err()));
    });
    (rx, done_rx)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    fn validator() -> Validator {
        Arc::new(|d: &Document| format!("built {} features", d.features.len()))
    }

    #[test]
    fn run_lua_edits_and_reports() {
        let mut doc = Document::default();
        let seen = Mutex::new(Vec::new());
        let reply = execute_tool(
            "run_lua",
            &json!({"script": "cad.box{size={1,2,3}} print('ok')"}),
            &mut doc,
            &validator(),
            &|e| seen.lock().unwrap().push(e),
        );
        assert_eq!(reply, "ok\nbuilt 1 features");
        assert_eq!(doc.features.len(), 1);
        assert_eq!(seen.lock().unwrap().len(), 2);
    }

    #[test]
    fn failing_script_leaves_the_document_alone() {
        let mut doc = Document::default();
        let reply = execute_tool(
            "run_lua",
            &json!({"script": "cad.box{size={1,2,3}} error('no')"}),
            &mut doc,
            &validator(),
            &|_| {},
        );
        assert!(reply.contains("nothing changed"), "{reply}");
        assert!(doc.features.is_empty());
    }

    #[test]
    fn get_document_returns_json_and_bad_calls_are_errors() {
        let mut doc = Document::default();
        let text = execute_tool("get_document", &json!({}), &mut doc, &validator(), &|_| {});
        assert_eq!(Document::from_json(&text).unwrap(), doc);
        let missing = execute_tool("run_lua", &json!({}), &mut doc, &validator(), &|_| {});
        assert!(missing.starts_with("error"));
        assert!(
            execute_tool("nope", &json!({}), &mut doc, &validator(), &|_| {}).contains("unknown")
        );
    }

    #[test]
    fn system_prompt_carries_the_api_reference() {
        assert!(system_prompt().contains("cad.extrude"));
    }
}
