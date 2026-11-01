//! Geometric selectors. Features name edges and faces by where they are rather
//! than by id, so a reference survives the renumbering a parameter edit causes.

use std::collections::{HashMap, HashSet};

use ogeom::geom::Curve3d as _;
use ogeom::topo::{Shape, ShapeType, explore_unique};
use serde::{Deserialize, Serialize};

use crate::{Bench, Handle, KernelError, Result, TOL, Vec3, kerr};

/// A side of the body's bounding box.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Side {
    Top,
    Bottom,
    Left,
    Right,
    Front,
    Back,
}

impl Side {
    fn axis_sign(self) -> (usize, f64) {
        match self {
            Self::Right => (0, 1.),
            Self::Left => (0, -1.),
            Self::Back => (1, 1.),
            Self::Front => (1, -1.),
            Self::Top => (2, 1.),
            Self::Bottom => (2, -1.),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EdgeSelector {
    All,
    /// Straight edges parallel to an axis (0 = x, 1 = y, 2 = z).
    Parallel(u8),
    /// Edges lying wholly on one side of the bounding box.
    OnSide(Side),
    /// Curved edges only.
    Curved,
    /// The single edge whose midpoint is closest to a point.
    Nearest(Vec3),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FaceSelector {
    OnSide(Side),
    Nearest(Vec3),
}

/// What selection needs to know about an edge.
#[derive(Debug, Clone, PartialEq)]
pub struct EdgeInfo {
    pub start: Vec3,
    pub middle: Vec3,
    pub end: Vec3,
    pub straight: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FaceInfo {
    pub min: Vec3,
    pub max: Vec3,
}

fn dist2(a: Vec3, b: Vec3) -> f64 {
    (0..3).map(|i| (a[i] - b[i]).powi(2)).sum()
}

impl Bench {
    fn edge_table(&self, solid: &Handle) -> Result<Vec<(Shape, EdgeInfo)>> {
        // A surface's seam belongs to one face only: it is a parametric artefact,
        // not a corner, and blending it is meaningless.
        let mut owners: HashMap<_, HashSet<usize>> = HashMap::new();
        let faces = explore_unique(&self.model, &solid.0, ShapeType::Face).map_err(kerr)?;
        for (i, face) in faces.iter().enumerate() {
            for edge in explore_unique(&self.model, face, ShapeType::Edge).map_err(kerr)? {
                owners.entry(edge.node()).or_default().insert(i);
            }
        }
        let mut out = Vec::new();
        for edge in explore_unique(&self.model, &solid.0, ShapeType::Edge).map_err(kerr)? {
            if owners.get(&edge.node()).is_some_and(|faces| faces.len() < 2) {
                continue;
            }
            let Some((curve, (lo, hi))) =
                ogeom::algo::placed_edge_curve(&self.model, &edge, TOL).map_err(kerr)?
            else {
                continue; // degenerate
            };
            let at = |u: f64| -> Result<Vec3> {
                let p = curve.point_at(u, TOL).map_err(kerr)?;
                Ok([p.x, p.y, p.z])
            };
            let info = EdgeInfo {
                start: at(lo)?,
                middle: at(0.5 * (lo + hi))?,
                end: at(hi)?,
                straight: matches!(curve.kind(), ogeom::geom::CurveKind::Line),
            };
            out.push((edge, info));
        }
        Ok(out)
    }

    pub fn edges(&self, solid: &Handle) -> Result<Vec<EdgeInfo>> {
        Ok(self.edge_table(solid)?.into_iter().map(|(_, i)| i).collect())
    }

    pub fn faces(&self, solid: &Handle) -> Result<Vec<FaceInfo>> {
        Ok(self.face_table(solid)?.into_iter().map(|(_, i)| i).collect())
    }

    fn face_table(&self, solid: &Handle) -> Result<Vec<(Shape, FaceInfo)>> {
        let mut out = Vec::new();
        for face in explore_unique(&self.model, &solid.0, ShapeType::Face).map_err(kerr)? {
            let bounds = ogeom::algo::face_bounds(&self.model, &face).map_err(kerr)?;
            let (Some(lo), Some(hi)) = (bounds.low(), bounds.high()) else { continue };
            let info = FaceInfo { min: [lo.x, lo.y, lo.z], max: [hi.x, hi.y, hi.z] };
            out.push((face, info));
        }
        Ok(out)
    }

    fn body_extreme(&self, solid: &Handle, side: Side) -> Result<f64> {
        let (axis, sign) = side.axis_sign();
        let bounds = ogeom::algo::shape_bounds(&self.model, &solid.0, TOL).map_err(kerr)?;
        let p = if sign > 0. { bounds.high() } else { bounds.low() };
        let p = p.ok_or_else(|| KernelError::Invalid("empty shape".into()))?;
        Ok([p.x, p.y, p.z][axis])
    }

    pub(crate) fn select_edges(&self, solid: &Handle, sel: &EdgeSelector) -> Result<Vec<Shape>> {
        let table = self.edge_table(solid)?;
        let chosen: Vec<_> = match sel {
            EdgeSelector::All => table.into_iter().map(|(e, _)| e).collect(),
            EdgeSelector::Curved => {
                table.into_iter().filter(|(_, i)| !i.straight).map(|(e, _)| e).collect()
            }
            EdgeSelector::Parallel(axis) => {
                let axis = usize::from(*axis).min(2);
                table
                    .into_iter()
                    .filter(|(_, i)| {
                        let d: Vec3 = std::array::from_fn(|k| i.end[k] - i.start[k]);
                        let len = dist2(i.start, i.end).sqrt();
                        i.straight && len > 0. && (d[axis].abs() / len) > 1. - 1e-9
                    })
                    .map(|(e, _)| e)
                    .collect()
            }
            EdgeSelector::OnSide(side) => {
                let (axis, _) = side.axis_sign();
                let extreme = self.body_extreme(solid, *side)?;
                table
                    .into_iter()
                    .filter(|(_, i)| {
                        [i.start, i.middle, i.end].iter().all(|p| (p[axis] - extreme).abs() < 1e-6)
                    })
                    .map(|(e, _)| e)
                    .collect()
            }
            EdgeSelector::Nearest(p) => table
                .into_iter()
                .min_by(|(_, a), (_, b)| dist2(a.middle, *p).total_cmp(&dist2(b.middle, *p)))
                .map(|(e, _)| e)
                .into_iter()
                .collect(),
        };
        if chosen.is_empty() {
            return Err(KernelError::EmptySelection(format!("{sel:?}")));
        }
        Ok(chosen)
    }

    pub(crate) fn select_faces(&self, solid: &Handle, sel: &FaceSelector) -> Result<Vec<Shape>> {
        let table = self.face_table(solid)?;
        let chosen: Vec<_> = match sel {
            FaceSelector::OnSide(side) => {
                let (axis, _) = side.axis_sign();
                let extreme = self.body_extreme(solid, *side)?;
                table
                    .into_iter()
                    .filter(|(_, i)| {
                        (i.min[axis] - extreme).abs() < 1e-6 && (i.max[axis] - extreme).abs() < 1e-6
                    })
                    .map(|(f, _)| f)
                    .collect()
            }
            FaceSelector::Nearest(p) => {
                let centre =
                    |i: &FaceInfo| -> Vec3 { std::array::from_fn(|k| 0.5 * (i.min[k] + i.max[k])) };
                table
                    .into_iter()
                    .min_by(|(_, a), (_, b)| dist2(centre(a), *p).total_cmp(&dist2(centre(b), *p)))
                    .map(|(f, _)| f)
                    .into_iter()
                    .collect()
            }
        };
        if chosen.is_empty() {
            return Err(KernelError::EmptySelection(format!("{sel:?}")));
        }
        Ok(chosen)
    }
}
