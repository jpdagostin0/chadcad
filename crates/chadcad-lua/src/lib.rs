//! Sandboxed Lua for ChadCAD. Two entry points share one locked-down runtime:
//!
//! - [`run_feature`] evaluates a `Script` timeline feature to a solid.
//! - [`run_document`] edits a [`Document`], for the console and the AI assistant.
//!
//! Neither can reach the file system, the OS or other Lua code on disk.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chadcad_kernel::{Bench, EdgeSelector, Handle, KernelError, Side, Vec3};
use chadcad_model::{Document, Expr, FeatureKind};
use mlua::{
    Error as LuaError, FromLua, HookTriggers, Lua, LuaOptions, LuaSerdeExt, MetaMethod, StdLib,
    Table, UserData, UserDataMethods, Value, VmState,
};

const MEMORY_LIMIT: usize = 256 * 1024 * 1024;
const TIME_LIMIT: Duration = Duration::from_secs(30);

/// Reference for the scripting API, shown to the AI assistant and in the console help.
pub const API_REFERENCE: &str = include_str!("api.md");

fn sandbox() -> Result<Lua, String> {
    let libs = StdLib::MATH | StdLib::STRING | StdLib::TABLE | StdLib::UTF8;
    let lua = Lua::new_with(libs, LuaOptions::default()).map_err(|e| e.to_string())?;
    lua.set_memory_limit(MEMORY_LIMIT).map_err(|e| e.to_string())?;
    let globals = lua.globals();
    for name in ["dofile", "loadfile", "load", "require", "collectgarbage"] {
        globals.set(name, Value::Nil).map_err(|e| e.to_string())?;
    }
    let started = Instant::now();
    lua.set_hook(HookTriggers::new().every_nth_instruction(100_000), move |_, _| {
        if started.elapsed() > TIME_LIMIT {
            return Err(LuaError::runtime("script exceeded its time limit"));
        }
        Ok(VmState::Continue)
    })
    .map_err(|e| e.to_string())?;
    Ok(lua)
}

/// Lua's error text without the Rust callback wrapper and traceback.
fn message(e: &LuaError) -> String {
    match e {
        LuaError::CallbackError { cause, .. } => message(cause),
        LuaError::RuntimeError(m) | LuaError::SyntaxError { message: m, .. } => {
            m.lines().next().unwrap_or(m).to_owned()
        }
        other => other.to_string(),
    }
}

fn kernel<T>(r: chadcad_kernel::Result<T>) -> mlua::Result<T> {
    r.map_err(|e: KernelError| LuaError::runtime(e.to_string()))
}

type SharedBench = Arc<Mutex<Bench>>;

#[derive(Clone)]
struct Solid {
    bench: SharedBench,
    handle: Handle,
}

impl Solid {
    fn with(
        &self,
        op: impl FnOnce(&mut Bench) -> chadcad_kernel::Result<Handle>,
    ) -> mlua::Result<Self> {
        let mut bench = self.bench.lock().map_err(|_| LuaError::runtime("kernel poisoned"))?;
        Ok(Self { bench: self.bench.clone(), handle: kernel(op(&mut bench))? })
    }
}

impl FromLua for Solid {
    fn from_lua(value: Value, _: &Lua) -> mlua::Result<Self> {
        match value {
            Value::UserData(ud) => Ok(ud.borrow::<Self>()?.clone()),
            other => Err(LuaError::runtime(format!("expected a solid, got {}", other.type_name()))),
        }
    }
}

fn edge_selector(sel: Option<String>) -> mlua::Result<EdgeSelector> {
    let Some(sel) = sel else { return Ok(EdgeSelector::All) };
    Ok(match sel.as_str() {
        "all" => EdgeSelector::All,
        "curved" => EdgeSelector::Curved,
        "x" => EdgeSelector::Parallel(0),
        "y" => EdgeSelector::Parallel(1),
        "z" => EdgeSelector::Parallel(2),
        "top" => EdgeSelector::OnSide(Side::Top),
        "bottom" => EdgeSelector::OnSide(Side::Bottom),
        "left" => EdgeSelector::OnSide(Side::Left),
        "right" => EdgeSelector::OnSide(Side::Right),
        "front" => EdgeSelector::OnSide(Side::Front),
        "back" => EdgeSelector::OnSide(Side::Back),
        other => return Err(LuaError::runtime(format!("unknown edge selector `{other}`"))),
    })
}

fn axis(name: &str) -> mlua::Result<Vec3> {
    match name {
        "x" => Ok([1., 0., 0.]),
        "y" => Ok([0., 1., 0.]),
        "z" => Ok([0., 0., 1.]),
        other => Err(LuaError::runtime(format!("unknown axis `{other}`"))),
    }
}

impl UserData for Solid {
    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method("fuse", |_, s, o: Self| s.with(|b| b.fuse(&s.handle, &o.handle)));
        m.add_method("cut", |_, s, o: Self| s.with(|b| b.cut(&s.handle, &o.handle)));
        m.add_method("common", |_, s, o: Self| s.with(|b| b.common(&s.handle, &o.handle)));
        m.add_meta_method(MetaMethod::Add, |_, s, o: Self| {
            s.with(|b| b.fuse(&s.handle, &o.handle))
        });
        m.add_meta_method(MetaMethod::Sub, |_, s, o: Self| s.with(|b| b.cut(&s.handle, &o.handle)));
        m.add_meta_method(MetaMethod::Mul, |_, s, o: Self| {
            s.with(|b| b.common(&s.handle, &o.handle))
        });
        m.add_method("translate", |_, s, (x, y, z): (f64, f64, f64)| {
            s.with(|b| b.translate(&s.handle, [x, y, z]))
        });
        m.add_method("rotate", |_, s, (name, degrees): (String, f64)| {
            let a = axis(&name)?;
            s.with(|b| b.rotate(&s.handle, [0.; 3], a, degrees))
        });
        m.add_method("mirror", |_, s, name: String| {
            let a = axis(&name)?;
            s.with(|b| b.mirror(&s.handle, [0.; 3], a))
        });
        m.add_method("fillet", |_, s, (radius, sel): (f64, Option<String>)| {
            let sel = edge_selector(sel)?;
            s.with(|b| b.fillet(&s.handle, &sel, radius))
        });
        m.add_method("chamfer", |_, s, (distance, sel): (f64, Option<String>)| {
            let sel = edge_selector(sel)?;
            s.with(|b| b.chamfer(&s.handle, &sel, distance))
        });
        m.add_method("volume", |_, s, ()| {
            let bench = s.bench.lock().map_err(|_| LuaError::runtime("kernel poisoned"))?;
            Ok(kernel(bench.properties(&s.handle))?.volume)
        });
    }
}

fn install_constructors(lua: &Lua, bench: &SharedBench) -> mlua::Result<()> {
    let g = lua.globals();
    macro_rules! ctor {
        ($name:literal, $args:ty, $make:expr) => {{
            let bench = bench.clone();
            g.set(
                $name,
                lua.create_function(move |_, args: $args| {
                    let handle = {
                        let mut b =
                            bench.lock().map_err(|_| LuaError::runtime("kernel poisoned"))?;
                        kernel($make(&mut *b, args))?
                    };
                    Ok(Solid { bench: bench.clone(), handle })
                })?,
            )?;
        }};
    }
    ctor!("box", (f64, f64, f64), |b: &mut Bench, (x, y, z)| b.cuboid([0.; 3], [x, y, z]));
    ctor!("cylinder", (f64, f64), |b: &mut Bench, (r, h)| b.cylinder([0.; 3], [0., 0., 1.], r, h));
    ctor!("cone", (f64, f64, f64), |b: &mut Bench, (r1, r2, h)| b.cone(
        [0.; 3],
        [0., 0., 1.],
        r1,
        r2,
        h
    ));
    ctor!("sphere", f64, |b: &mut Bench, r| b.sphere([0.; 3], r));
    ctor!("torus", (f64, f64), |b: &mut Bench, (major, minor)| b.torus(
        [0.; 3],
        [0., 0., 1.],
        major,
        minor
    ));
    ctor!("prism", (Vec<[f64; 2]>, f64), |b: &mut Bench, (points, h): (Vec<[f64; 2]>, f64)| {
        let outer = chadcad_kernel::Loop::Polygon(points);
        let face = b.profile(chadcad_kernel::PlaneKind::XY, 0., &outer, &[])?;
        b.extrude(&face, [0., 0., h], 0.)
    });
    Ok(())
}

/// Runs a `Script` feature. `params` are exposed as the `params` table and
/// `body`, when there is one, is the solid built so far.
pub fn run_feature(
    source: &str,
    bench: Bench,
    body: Option<Handle>,
    params: &[(String, f64)],
) -> Result<(Bench, Handle), String> {
    let lua = sandbox()?;
    let bench: SharedBench = Arc::new(Mutex::new(bench));
    let run = || -> mlua::Result<Solid> {
        install_constructors(&lua, &bench)?;
        let table = lua.create_table()?;
        for (name, value) in params {
            table.set(name.as_str(), *value)?;
        }
        lua.globals().set("params", table)?;
        if let Some(handle) = body {
            lua.globals().set("body", Solid { bench: bench.clone(), handle })?;
        }
        lua.load(source).set_name("feature").eval()
    };
    let solid = run().map_err(|e| message(&e))?;
    let handle = solid.handle.clone();
    drop(solid);
    lua.globals().clear().map_err(|e| e.to_string())?;
    drop(lua);
    let bench = Arc::try_unwrap(bench)
        .map_err(|_| "script kept the kernel alive".to_owned())?
        .into_inner()
        .map_err(|_| "kernel poisoned".to_owned())?;
    Ok((bench, handle))
}

const KINDS: [(&str, &str); 16] = [
    ("box", "Box"),
    ("cylinder", "Cylinder"),
    ("cone", "Cone"),
    ("sphere", "Sphere"),
    ("torus", "Torus"),
    ("extrude", "Extrude"),
    ("revolve", "Revolve"),
    ("fillet", "Fillet"),
    ("chamfer", "Chamfer"),
    ("shell", "Shell"),
    ("translate", "Translate"),
    ("rotate", "Rotate"),
    ("mirror", "Mirror"),
    ("linear_pattern", "LinearPattern"),
    ("polar_pattern", "PolarPattern"),
    ("script", "Script"),
];

fn add_feature(lua: &Lua, doc: &Mutex<Document>, tag: &str, args: Table) -> mlua::Result<String> {
    let name: Option<String> = args.get("name")?;
    args.set("name", Value::Nil)?;
    let wrapper = lua.create_table()?;
    wrapper.set(tag, args)?;
    let kind: FeatureKind = lua
        .from_value(Value::Table(wrapper))
        .map_err(|e| LuaError::runtime(format!("{tag}: {}", message(&e))))?;
    let mut doc = doc.lock().map_err(|_| LuaError::runtime("document poisoned"))?;
    let id = doc.add_feature(kind);
    let feature = doc.feature_mut(id).ok_or_else(|| LuaError::runtime("feature vanished"))?;
    if let Some(name) = name {
        feature.name = name;
    }
    Ok(feature.name.clone())
}

fn feature_id(doc: &Document, name: &str) -> mlua::Result<u64> {
    doc.features
        .iter()
        .find(|f| f.name == name)
        .map(|f| f.id)
        .ok_or_else(|| LuaError::runtime(format!("no feature named `{name}`")))
}

fn install_document_api(
    lua: &Lua,
    doc: &Arc<Mutex<Document>>,
    log: &Arc<Mutex<String>>,
) -> mlua::Result<()> {
    let cad = lua.create_table()?;
    let lock = |d: &'_ Arc<Mutex<Document>>| {
        d.lock().map(|g| g.clone()).map_err(|_| LuaError::runtime("document poisoned"))
    };
    for (lua_name, tag) in KINDS {
        let doc = doc.clone();
        let f = lua.create_function(move |lua, args: Table| add_feature(lua, &doc, tag, args))?;
        cad.set(lua_name, f)?;
    }
    {
        let doc = doc.clone();
        let f = lua.create_function(move |_, (name, expr): (String, Value)| {
            let expr = match expr {
                Value::String(s) => Expr(s.to_str()?.to_owned()),
                Value::Integer(i) => Expr(i.to_string()),
                Value::Number(n) => Expr::from(n),
                other => {
                    let got = other.type_name();
                    return Err(LuaError::runtime(format!("param wants a value, got {got}")));
                }
            };
            let mut doc = doc.lock().map_err(|_| LuaError::runtime("document poisoned"))?;
            doc.set_parameter(&name, expr);
            Ok(())
        })?;
        cad.set("param", f)?;
    }
    {
        let doc = doc.clone();
        let f = lua.create_function(move |_, name: String| {
            let mut doc = doc.lock().map_err(|_| LuaError::runtime("document poisoned"))?;
            let id = feature_id(&doc, &name)?;
            doc.remove_feature(id);
            Ok(())
        })?;
        cad.set("remove", f)?;
    }
    {
        let doc = doc.clone();
        let f = lua.create_function(move |_, (name, on): (String, Option<bool>)| {
            let mut doc = doc.lock().map_err(|_| LuaError::runtime("document poisoned"))?;
            let id = feature_id(&doc, &name)?;
            if let Some(f) = doc.feature_mut(id) {
                f.suppressed = on.unwrap_or(true);
            }
            Ok(())
        })?;
        cad.set("suppress", f)?;
    }
    {
        let doc = doc.clone();
        let f = lua.create_function(move |_, count: Option<usize>| {
            doc.lock().map_err(|_| LuaError::runtime("document poisoned"))?.rollback = count;
            Ok(())
        })?;
        cad.set("rollback", f)?;
    }
    {
        let doc = doc.clone();
        let f = lua.create_function(move |lua, ()| {
            let snapshot = lock(&doc)?;
            let names: Vec<String> = snapshot.features.iter().map(|f| f.name.clone()).collect();
            lua.to_value(&names)
        })?;
        cad.set("features", f)?;
    }
    lua.globals().set("cad", cad)?;
    let log = log.clone();
    let print = lua.create_function(move |_, values: mlua::Variadic<Value>| {
        let line: Vec<String> = values
            .iter()
            .map(|v| v.to_string().unwrap_or_else(|_| v.type_name().to_owned()))
            .collect();
        let mut log = log.lock().map_err(|_| LuaError::runtime("log poisoned"))?;
        log.push_str(&line.join("\t"));
        log.push('\n');
        Ok(())
    })?;
    lua.globals().set("print", print)
}

/// The result of a console script. `document` is `None` when the script failed,
/// so a failed script never leaves a half-applied edit behind.
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptOutcome {
    pub document: Option<Document>,
    pub output: String,
    pub error: Option<String>,
}

/// Runs a console script against a copy of `doc`.
pub fn run_document(source: &str, doc: &Document) -> ScriptOutcome {
    let shared = Arc::new(Mutex::new(doc.clone()));
    let log = Arc::new(Mutex::new(String::new()));
    let result = sandbox().and_then(|lua| {
        install_document_api(&lua, &shared, &log).map_err(|e| message(&e))?;
        lua.load(source).set_name("console").exec().map_err(|e| message(&e))
    });
    let output = log.lock().map(|l| l.clone()).unwrap_or_default();
    match result {
        Ok(()) => {
            ScriptOutcome { document: shared.lock().ok().map(|d| d.clone()), output, error: None }
        }
        Err(error) => ScriptOutcome { document: None, output, error: Some(error) },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feature_script_builds_a_solid_with_params_and_operators() {
        let src = "local w = params.w\n\
                   local s = box(w, w, w) - cylinder(2, w):translate(w / 2, w / 2, 0)\n\
                   return s:fillet(1, 'top')";
        let (b, h) = run_feature(src, Bench::new(), None, &[("w".into(), 10.)]).unwrap();
        let v = b.properties(&h).unwrap().volume;
        assert!(v > 800. && v < 1000. - 12. * 3.14 * 0.9, "{v}");
    }

    #[test]
    fn feature_script_sees_the_body() {
        let mut b = Bench::new();
        let body = b.cuboid([0.; 3], [10., 10., 10.]).unwrap();
        let src = "return body:cut(box(5, 5, 20))";
        let (b, h) = run_feature(src, b, Some(body), &[]).unwrap();
        assert!((b.properties(&h).unwrap().volume - 750.).abs() < 1e-6);
    }

    #[test]
    fn sandbox_blocks_the_outside_world() {
        for src in [
            "return io.open('x')",
            "return os.getenv('PATH')",
            "return require('x')",
            "return dofile('x')",
            "return load('return 1')()",
        ] {
            assert!(run_feature(src, Bench::new(), None, &[]).is_err(), "{src}");
        }
    }

    #[test]
    fn document_script_edits_through_serde() {
        let src = r#"
            cad.param("w", 30)
            cad.box{size={"w", 20, 10}, name="Base"}
            cad.cylinder{origin={15, 10, -1}, radius=3, height=12, op="Cut"}
            cad.fillet{edges={Parallel=2}, radius="w/10"}
            print(#cad.features())
        "#;
        let out = run_document(src, &Document::default());
        assert_eq!(out.error, None);
        assert_eq!(out.output, "3\n");
        let doc = out.document.unwrap();
        assert_eq!(doc.features[0].name, "Base");
        assert_eq!(doc.features[1].kind.tag(), "Cylinder");
        assert_eq!(doc.parameters[0].expr.0, "30");
    }

    #[test]
    fn failed_script_changes_nothing() {
        let out = run_document("cad.box{size={1,2,3}}\nerror('boom')", &Document::default());
        assert!(out.document.is_none());
        assert!(out.error.unwrap().contains("boom"));
        let bad = run_document("cad.box{}", &Document::default());
        assert!(bad.error.unwrap().contains("size"));
    }
}
