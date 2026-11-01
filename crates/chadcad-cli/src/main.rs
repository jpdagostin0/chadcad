//! Headless ChadCAD: build documents, run scripts, export, and ask the assistant.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, Mutex};

use chadcad_ai::{Conversation, Event};
use chadcad_kernel::Format;
use chadcad_model::Document;
use chadcad_timeline::Engine;

const CHORD: f64 = 0.01;
const USAGE: &str = "usage:
  chadcad-cli info DOC.chad
  chadcad-cli export DOC.chad OUT.{step,iges,brep,stl,obj,ply,glb,3mf,ogeom}
  chadcad-cli lua SCRIPT.lua DOC.chad    apply a script, creating DOC if needed
  chadcad-cli ask DOC.chad \"prompt\"       let the assistant edit DOC (CHADCAD_MODEL picks the model)";

fn load(path: &Path) -> Result<Document, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Document::from_json(&text).map_err(|e| format!("{}: {e}", path.display()))
}

fn store(path: &Path, doc: &Document) -> Result<(), String> {
    let json = doc.to_json().map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| format!("{}: {e}", path.display()))
}

fn info(path: &Path) -> Result<(), String> {
    let doc = load(path)?;
    let result = Engine::new().build(&doc, CHORD);
    print!("{}", chadcad_timeline::describe(&doc, &result));
    if result.errors.is_empty() { Ok(()) } else { Err("the timeline has errors".into()) }
}

fn export(path: &Path, out: &Path) -> Result<(), String> {
    let doc = load(path)?;
    let extension = out.extension().and_then(|e| e.to_str()).unwrap_or("");
    let format = Format::from_extension(extension).map_err(|e| e.to_string())?;
    let bytes = chadcad_timeline::export_document(&doc, format, CHORD)?;
    std::fs::write(out, &bytes).map_err(|e| format!("{}: {e}", out.display()))?;
    println!("wrote {} ({} bytes)", out.display(), bytes.len());
    Ok(())
}

fn lua(script: &Path, path: &Path) -> Result<(), String> {
    let source =
        std::fs::read_to_string(script).map_err(|e| format!("{}: {e}", script.display()))?;
    let doc = if path.exists() { load(path)? } else { Document::default() };
    let outcome = chadcad_lua::run_document(&source, &doc);
    print!("{}", outcome.output);
    if let Some(error) = outcome.error {
        return Err(error);
    }
    let mut doc = outcome.document.ok_or("the script produced no document")?;
    if doc.name == Document::default().name
        && let Some(stem) = path.file_stem()
    {
        doc.name = stem.to_string_lossy().into_owned();
    }
    store(path, &doc)
}

fn ask(path: &Path, prompt: String) -> Result<(), String> {
    let doc = if path.exists() { load(path)? } else { Document::default() };
    let model =
        std::env::var("CHADCAD_MODEL").unwrap_or_else(|_| chadcad_ai::DEFAULT_MODEL.to_owned());
    let engine = Mutex::new(Engine::new());
    let validate = Arc::new(move |d: &Document| match engine.lock() {
        Ok(mut engine) => chadcad_timeline::describe(d, &engine.build(d, CHORD)),
        Err(_) => "the engine is unavailable".to_owned(),
    });
    let (events, _) = chadcad_ai::spawn_turn(model, Conversation::new(), prompt, doc, validate);
    for event in events {
        match event {
            Event::Text(text) => println!("{text}"),
            Event::Tool { script, output } => {
                println!("--- script\n{script}\n--- result\n{output}")
            }
            Event::Document(doc) => store(path, &doc)?,
            Event::Finished(Some(error)) => return Err(error),
            Event::Finished(None) => {}
        }
    }
    Ok(())
}

fn run(args: &[String]) -> Result<(), String> {
    let path = |i: usize| -> PathBuf { PathBuf::from(&args[i]) };
    match args.first().map(String::as_str) {
        Some("info") if args.len() == 2 => info(&path(1)),
        Some("export") if args.len() == 3 => export(&path(1), &path(2)),
        Some("lua") if args.len() == 3 => lua(&path(1), &path(2)),
        Some("ask") if args.len() == 3 => ask(&path(1), args[2].clone()),
        _ => Err(USAGE.into()),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
