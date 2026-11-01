//! What each feature does to the body. Pure: no Salsa here.

use std::sync::Arc;

use chadcad_kernel::{Bench, Handle, Loop};
use chadcad_model::{Combine, Expr, Feature, FeatureKind, Profile, Sketch, Uv, Xyz};

use crate::Body;

type Eval<'a> = dyn FnMut(&Expr) -> Result<f64, String> + 'a;

const MAX_PATTERN: f64 = 1000.;

fn xyz(eval: &mut Eval, v: &Xyz) -> Result<[f64; 3], String> {
    Ok([eval(&v[0])?, eval(&v[1])?, eval(&v[2])?])
}

fn uv(eval: &mut Eval, v: &Uv) -> Result<[f64; 2], String> {
    Ok([eval(&v[0])?, eval(&v[1])?])
}

fn profile(eval: &mut Eval, p: &Profile) -> Result<Loop, String> {
    Ok(match p {
        Profile::Rect { center, size } => {
            let [cx, cy] = uv(eval, center)?;
            let [w, h] = uv(eval, size)?;
            let (a, b) = (w / 2., h / 2.);
            Loop::Polygon(vec![
                [cx - a, cy - b],
                [cx + a, cy - b],
                [cx + a, cy + b],
                [cx - a, cy + b],
            ])
        }
        Profile::Circle { center, radius } => {
            Loop::Circle { center: uv(eval, center)?, radius: eval(radius)? }
        }
        Profile::Polygon { points } => {
            Loop::Polygon(points.iter().map(|p| uv(eval, p)).collect::<Result<_, _>>()?)
        }
        Profile::RegularPolygon { center, radius, sides } => {
            let [cx, cy] = uv(eval, center)?;
            let radius = eval(radius)?;
            let sides = eval(sides)?;
            if !(3. ..=MAX_PATTERN).contains(&sides) {
                return Err(format!("a polygon cannot have {sides} sides"));
            }
            let sides = sides.round() as usize;
            let corner = |i: usize| {
                let a = std::f64::consts::TAU * i as f64 / sides as f64;
                [cx + radius * a.cos(), cy + radius * a.sin()]
            };
            Loop::Polygon((0..sides).map(corner).collect())
        }
    })
}

fn sketch_face(bench: &mut Bench, eval: &mut Eval, s: &Sketch) -> Result<Handle, String> {
    let offset = eval(&s.offset)?;
    let outer = profile(eval, &s.outer)?;
    let holes = s.holes.iter().map(|h| profile(eval, h)).collect::<Result<Vec<_>, _>>()?;
    bench.profile(s.plane, offset, &outer, &holes).map_err(|e| e.to_string())
}

fn pattern_count(eval: &mut Eval, count: &Expr) -> Result<usize, String> {
    let count = eval(count)?;
    if !(1. ..=MAX_PATTERN).contains(&count) {
        return Err(format!("pattern count {count} is out of range"));
    }
    Ok(count.round() as usize)
}

fn combine(
    bench: &mut Bench,
    body: Option<&Handle>,
    tool: Handle,
    op: Combine,
) -> Result<Handle, String> {
    let e = |e: chadcad_kernel::KernelError| e.to_string();
    match (body, op) {
        (None, Combine::Join) => Ok(tool),
        (None, _) => Err("nothing to cut or intersect yet".into()),
        (Some(b), Combine::Join) => bench.fuse(b, &tool).map_err(e),
        (Some(b), Combine::Cut) => bench.cut(b, &tool).map_err(e),
        (Some(b), Combine::Intersect) => bench.common(b, &tool).map_err(e),
    }
}

/// Runs one feature on a private copy of the previous bench.
pub(crate) fn apply(
    feature: &Feature,
    before: Option<&Body>,
    eval: &mut Eval,
    params: &mut dyn FnMut() -> Vec<(String, f64)>,
) -> Result<Body, String> {
    let mut bench = before.map_or_else(Bench::new, |b| (*b.bench).clone());
    let body = before.map(|b| &b.shape);
    let need_body =
        || body.ok_or_else(|| format!("{} needs a body to work on", feature.kind.tag()));
    let k = |e: chadcad_kernel::KernelError| e.to_string();
    let shape = match &feature.kind {
        FeatureKind::Box { origin, size, op } => {
            let origin = xyz(eval, origin)?;
            let size = xyz(eval, size)?;
            let tool = bench.cuboid(origin, size).map_err(k)?;
            combine(&mut bench, body, tool, *op)?
        }
        FeatureKind::Cylinder { origin, axis, radius, height, op } => {
            let origin = xyz(eval, origin)?;
            let (radius, height) = (eval(radius)?, eval(height)?);
            let tool = bench.cylinder(origin, axis.vector(), radius, height).map_err(k)?;
            combine(&mut bench, body, tool, *op)?
        }
        FeatureKind::Cone { origin, axis, base_radius, top_radius, height, op } => {
            let origin = xyz(eval, origin)?;
            let (base, top, height) = (eval(base_radius)?, eval(top_radius)?, eval(height)?);
            let tool = bench.cone(origin, axis.vector(), base, top, height).map_err(k)?;
            combine(&mut bench, body, tool, *op)?
        }
        FeatureKind::Sphere { center, radius, op } => {
            let center = xyz(eval, center)?;
            let tool = bench.sphere(center, eval(radius)?).map_err(k)?;
            combine(&mut bench, body, tool, *op)?
        }
        FeatureKind::Torus { center, axis, major, minor, op } => {
            let center = xyz(eval, center)?;
            let (major, minor) = (eval(major)?, eval(minor)?);
            let tool = bench.torus(center, axis.vector(), major, minor).map_err(k)?;
            combine(&mut bench, body, tool, *op)?
        }
        FeatureKind::Extrude { sketch, distance, taper, op } => {
            let face = sketch_face(&mut bench, eval, sketch)?;
            let (_, _, n) = sketch.plane.axes();
            let d = eval(distance)?;
            let tool = bench.extrude(&face, n.map(|c| c * d), eval(taper)?).map_err(k)?;
            combine(&mut bench, body, tool, *op)?
        }
        FeatureKind::Revolve { sketch, axis, axis_origin, angle, op } => {
            let face = sketch_face(&mut bench, eval, sketch)?;
            let origin = xyz(eval, axis_origin)?;
            let tool = bench.revolve(&face, origin, axis.vector(), eval(angle)?).map_err(k)?;
            combine(&mut bench, body, tool, *op)?
        }
        FeatureKind::Fillet { edges, radius } => {
            bench.fillet(need_body()?, edges, eval(radius)?).map_err(k)?
        }
        FeatureKind::Chamfer { edges, distance } => {
            bench.chamfer(need_body()?, edges, eval(distance)?).map_err(k)?
        }
        FeatureKind::Shell { open, thickness } => {
            bench.shell(need_body()?, open, eval(thickness)?).map_err(k)?
        }
        FeatureKind::Translate { by } => {
            let by = xyz(eval, by)?;
            bench.translate(need_body()?, by).map_err(k)?
        }
        FeatureKind::Rotate { origin, axis, angle } => {
            let origin = xyz(eval, origin)?;
            bench.rotate(need_body()?, origin, axis.vector(), eval(angle)?).map_err(k)?
        }
        FeatureKind::Mirror { origin, normal, merge } => {
            let origin = xyz(eval, origin)?;
            let source = need_body()?;
            let image = bench.mirror(source, origin, normal.vector()).map_err(k)?;
            if *merge { bench.fuse(source, &image).map_err(k)? } else { image }
        }
        FeatureKind::LinearPattern { direction, count, spacing } => {
            let direction = xyz(eval, direction)?;
            let length = direction.iter().map(|c| c * c).sum::<f64>().sqrt();
            if length == 0. {
                return Err("pattern direction is zero".into());
            }
            let step = eval(spacing)? / length;
            let count = pattern_count(eval, count)?;
            let source = need_body()?;
            let mut out = source.clone();
            for i in 1..count {
                let by = direction.map(|c| c * step * i as f64);
                let copy = bench.translate(source, by).map_err(k)?;
                out = bench.fuse(&out, &copy).map_err(k)?;
            }
            out
        }
        FeatureKind::PolarPattern { origin, axis, count, angle } => {
            let origin = xyz(eval, origin)?;
            let angle = eval(angle)?;
            let count = pattern_count(eval, count)?;
            // A full turn spaces copies evenly; anything less ends on the last copy.
            let full = (angle.abs() - 360.).abs() < 1e-9;
            let divisions = if full { count } else { count.saturating_sub(1).max(1) };
            let source = need_body()?;
            let mut out = source.clone();
            for i in 1..count {
                let a = angle * i as f64 / divisions as f64;
                let copy = bench.rotate(source, origin, axis.vector(), a).map_err(k)?;
                out = bench.fuse(&out, &copy).map_err(k)?;
            }
            out
        }
        FeatureKind::Script { source, op, replace } => {
            let (scripted, tool) =
                chadcad_lua::run_feature(source, bench, body.cloned(), &params())?;
            bench = scripted;
            if *replace { tool } else { combine(&mut bench, body, tool, *op)? }
        }
    };
    bench.compact(&shape).map_err(k)?;
    Ok(Body { bench: Arc::new(bench), shape })
}
