//! The feature timeline as Salsa queries.
//!
//! Each timeline position is a query over the position before it, so an edit
//! rebuilds only from the first feature it actually changes, parameters are
//! tracked one by one, and scrubbing back and forth is a cache hit.

mod build;

use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use chadcad_kernel::{Bench, Format, Handle, MeshData, Properties};
use chadcad_model::{Document, Expr, Feature, FeatureId};
use salsa::{Database as _, Setter as _};

#[salsa::db]
pub trait TimelineDb: salsa::Database {
    /// Counts feature executions, so tests can prove what did not rebuild.
    fn note_build(&self);
}

#[salsa::db]
#[derive(Clone, Default)]
pub struct Db {
    storage: salsa::Storage<Self>,
    builds: Arc<AtomicUsize>,
}

#[salsa::db]
impl salsa::Database for Db {}

#[salsa::db]
impl TimelineDb for Db {
    fn note_build(&self) {
        self.builds.fetch_add(1, Ordering::Relaxed);
    }
}

#[salsa::input(debug)]
pub(crate) struct ParamIn {
    #[returns(ref)]
    pub expr: String,
}

#[salsa::input(debug)]
pub(crate) struct FeatureIn {
    #[returns(ref)]
    pub feature: Feature,
}

#[salsa::input(debug)]
pub(crate) struct DocIn {
    #[returns(ref)]
    pub params: BTreeMap<String, ParamIn>,
    #[returns(ref)]
    pub features: Vec<FeatureIn>,
}

#[salsa::interned(debug)]
pub(crate) struct ParamName<'db> {
    #[returns(ref)]
    pub text: String,
}

/// A parameter's value. Its own query, so a feature using `w` ignores edits to
/// `h`, and rewriting `w` to an equal value stops here.
#[salsa::tracked(cycle_result = param_cycle)]
pub(crate) fn param_value<'db>(
    db: &'db dyn TimelineDb,
    doc: DocIn,
    name: ParamName<'db>,
) -> Result<f64, String> {
    let text = name.text(db);
    let Some(param) = doc.params(db).get(text) else {
        return Err(format!("unknown parameter `{text}`"));
    };
    chadcad_model::eval(param.expr(db), &mut |n| {
        param_value(db, doc, ParamName::new(db, n.to_owned())).clone()
    })
    .map_err(|e| format!("{text}: {e}"))
}

fn param_cycle<'db>(
    _db: &'db dyn TimelineDb,
    _id: salsa::Id,
    _doc: DocIn,
    _name: ParamName<'db>,
) -> Result<f64, String> {
    Err("parameters depend on each other in a cycle".into())
}

pub(crate) fn expr_value(db: &dyn TimelineDb, doc: DocIn, expr: &Expr) -> Result<f64, String> {
    chadcad_model::eval(&expr.0, &mut |n| {
        param_value(db, doc, ParamName::new(db, n.to_owned())).clone()
    })
}

/// Early cutoff for the feature list: inserting at the end leaves earlier positions untouched.
#[salsa::tracked]
pub(crate) fn feature_at(db: &dyn TimelineDb, doc: DocIn, index: usize) -> Option<FeatureIn> {
    doc.features(db).get(index).copied()
}

/// The state after one timeline position.
#[derive(Debug, Clone)]
pub struct Step {
    /// The body so far. A failed feature passes its predecessor's body through.
    pub body: Option<Body>,
    /// Why this feature failed, if it did.
    pub error: Option<String>,
    /// A hash of everything that determined this result.
    stamp: u64,
}

#[derive(Debug, Clone)]
pub struct Body {
    pub bench: Arc<Bench>,
    pub shape: Handle,
}

/// The stamp stands in for geometry equality, which the kernel has no cheap way to answer.
impl PartialEq for Step {
    fn eq(&self, other: &Self) -> bool {
        self.stamp == other.stamp && self.error == other.error
    }
}

impl Eq for Step {}

/// The state after the first `count` features.
#[salsa::tracked(returns(ref))]
pub(crate) fn step(db: &dyn TimelineDb, doc: DocIn, count: usize) -> Step {
    let empty = Step { body: None, error: None, stamp: 0 };
    let Some(previous) = count.checked_sub(1) else { return empty };
    let before = step(db, doc, previous);
    let Some(input) = feature_at(db, doc, previous) else { return before.clone() };
    let feature = input.feature(db);
    if feature.suppressed {
        return Step { error: None, ..before.clone() };
    }
    db.note_build();
    let mut resolved = Vec::new();
    let mut script_inputs = String::new();
    let result = build::apply(
        feature,
        before.body.as_ref(),
        &mut |e| {
            let v = expr_value(db, doc, e)?;
            resolved.push(v.to_bits());
            Ok(v)
        },
        &mut || {
            let values = script_params(db, doc);
            script_inputs = format!("{values:?}");
            values
        },
    );
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    before.stamp.hash(&mut hasher);
    format!("{:?}", feature.kind).hash(&mut hasher);
    resolved.hash(&mut hasher);
    script_inputs.hash(&mut hasher);
    let stamp = hasher.finish();
    match result {
        Ok(body) => Step { body: Some(body), error: None, stamp },
        Err(error) => Step { body: before.body.clone(), error: Some(error), stamp },
    }
}

/// Every parameter, for script features: a script can read any of them.
fn script_params(db: &dyn TimelineDb, doc: DocIn) -> Vec<(String, f64)> {
    doc.params(db)
        .keys()
        .filter_map(|name| {
            let value = param_value(db, doc, ParamName::new(db, name.clone())).clone();
            value.ok().map(|v| (name.clone(), v))
        })
        .collect()
}

#[salsa::tracked(returns(ref))]
pub(crate) fn mesh_at(
    db: &dyn TimelineDb,
    doc: DocIn,
    count: usize,
    chord_micrometres: u32,
) -> Result<Arc<MeshData>, String> {
    let Some(body) = &step(db, doc, count).body else { return Ok(Arc::default()) };
    let chord = f64::from(chord_micrometres) / 1000.;
    body.bench.tessellate(&body.shape, chord).map(Arc::new).map_err(|e| e.to_string())
}

#[salsa::tracked]
pub(crate) fn properties_at(
    db: &dyn TimelineDb,
    doc: DocIn,
    count: usize,
) -> Result<Option<PropertiesEq>, String> {
    let Some(body) = &step(db, doc, count).body else { return Ok(None) };
    body.bench.properties(&body.shape).map(|p| Some(PropertiesEq(p))).map_err(|e| e.to_string())
}

/// [`Properties`] with bitwise equality, for Salsa's cutoff.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PropertiesEq(Properties);

impl PartialEq for PropertiesEq {
    fn eq(&self, other: &Self) -> bool {
        format!("{:?}", self.0) == format!("{:?}", other.0)
    }
}

impl Eq for PropertiesEq {}

/// Everything the UI needs after a build.
#[derive(Debug, Clone, Default)]
pub struct BuildResult {
    pub mesh: Arc<MeshData>,
    pub properties: Option<Properties>,
    /// Per-feature errors by feature id.
    pub errors: HashMap<FeatureId, String>,
    /// Values of all parameters, or the reason each has none.
    pub parameters: Vec<(String, Result<f64, String>)>,
    /// How many features actually ran for this build.
    pub rebuilt: usize,
}

/// A Salsa database kept in step with a [`Document`].
pub struct Engine {
    db: Db,
    doc: DocIn,
    params: BTreeMap<String, ParamIn>,
    features: HashMap<FeatureId, FeatureIn>,
    order: Vec<FeatureId>,
    built_count: usize,
}

impl Default for Engine {
    fn default() -> Self {
        let db = Db::default();
        let doc = DocIn::new(&db, BTreeMap::new(), Vec::new());
        Self {
            db,
            doc,
            params: BTreeMap::new(),
            features: HashMap::new(),
            order: Vec::new(),
            built_count: 0,
        }
    }
}

impl Engine {
    pub fn new() -> Self {
        Self::default()
    }

    /// Copies `document` into Salsa inputs, setting only what changed.
    fn sync(&mut self, document: &Document) {
        let db = &mut self.db;
        let mut params_changed = self.params.len() != document.parameters.len();
        for p in &document.parameters {
            match self.params.get(&p.name) {
                Some(input) => {
                    if input.expr(db) != &p.expr.0 {
                        input.set_expr(db).to(p.expr.0.clone());
                    }
                }
                None => {
                    self.params.insert(p.name.clone(), ParamIn::new(db, p.expr.0.clone()));
                    params_changed = true;
                }
            }
        }
        if self.params.len() != document.parameters.len() {
            self.params.retain(|name, _| document.parameters.iter().any(|p| &p.name == name));
            params_changed = true;
        }
        if params_changed {
            self.doc.set_params(db).to(self.params.clone());
        }
        for f in &document.features {
            match self.features.get(&f.id) {
                Some(input) => {
                    if input.feature(db) != f {
                        input.set_feature(db).to(f.clone());
                    }
                }
                None => {
                    self.features.insert(f.id, FeatureIn::new(db, f.clone()));
                }
            }
        }
        let order: Vec<FeatureId> = document.features.iter().map(|f| f.id).collect();
        if order != self.order {
            self.features.retain(|id, _| order.contains(id));
            let inputs = order.iter().map(|id| self.features[id]).collect();
            self.doc.set_features(db).to(inputs);
            self.order = order;
        }
    }

    fn count(document: &Document) -> usize {
        document.rollback.map_or(document.features.len(), |r| r.min(document.features.len()))
    }

    /// Syncs and evaluates `document`. `chord` is the display mesh deflection in mm.
    pub fn build(&mut self, document: &Document, chord: f64) -> BuildResult {
        self.sync(document);
        let db = &self.db;
        let before = db.builds.load(Ordering::Relaxed);
        let doc = self.doc;
        let count = Self::count(document);
        self.built_count = count;
        let chord = (chord * 1000.).round().clamp(1., 1e9) as u32;
        let mut errors = HashMap::new();
        for at in 1..=count {
            if let Some(error) = &step(db, doc, at).error {
                errors.insert(document.features[at - 1].id, error.clone());
            }
        }
        let mesh = match mesh_at(db, doc, count, chord) {
            Ok(mesh) => mesh.clone(),
            Err(error) => {
                if let Some(last) = document.features.get(count.wrapping_sub(1)) {
                    errors.insert(last.id, error.clone());
                }
                Arc::default()
            }
        };
        let properties = properties_at(db, doc, count).clone().ok().flatten().map(|p| p.0);
        let parameters = document
            .parameters
            .iter()
            .map(|p| {
                let name = ParamName::new(db, p.name.clone());
                (p.name.clone(), param_value(db, doc, name).clone())
            })
            .collect();
        let rebuilt = db.builds.load(Ordering::Relaxed) - before;
        BuildResult { mesh, properties, errors, parameters, rebuilt }
    }

    /// The last built body, for export.
    pub fn body(&self) -> Option<Body> {
        self.db.attach(|_| ());
        step(&self.db, self.doc, self.built_count).body.clone()
    }

    pub fn export(&self, name: &str, format: Format, chord: f64) -> Result<Vec<u8>, String> {
        let body = self.body().ok_or("nothing to export: the timeline has no body")?;
        body.bench.export(&body.shape, name, format, chord).map_err(|e| e.to_string())
    }
}

/// Builds `document` once and writes `format`, for tools that need no live engine.
pub fn export_document(document: &Document, format: Format, chord: f64) -> Result<Vec<u8>, String> {
    let mut engine = Engine::new();
    let result = engine.build(document, chord);
    if let Some((_, error)) = result.errors.iter().next() {
        return Err(error.clone());
    }
    engine.export(&document.name, format, chord)
}
