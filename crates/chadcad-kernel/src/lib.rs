//! The single adapter between ChadCAD and the [`ogeom`] B-rep kernel.
//!
//! ogeom is LGPL-2.1-only, so nothing else in the workspace names it: every
//! other crate speaks this crate's types. Swapping the kernel means rewriting
//! this crate and nothing else.

mod export;
mod select;

use std::sync::Arc;

use ogeom::core::Tolerances;
use ogeom::geom::{CircleCurve, Curve, PlaneSurface, SurfaceGeometry};
use ogeom::math::{Axis, Circle, Direction, Frame, Plane, Point, Transform, Vector};
use ogeom::mesh::Deflection;
use ogeom::topo::{Model, Shape, ShapeType, explore_unique};
use serde::{Deserialize, Serialize};

pub use export::{Format, read_back_volume};
pub use select::{EdgeInfo, EdgeSelector, FaceInfo, FaceSelector, Side};

pub const TOL: Tolerances = Tolerances::millimetres();

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum KernelError {
    #[error("{0}")]
    Kernel(String),
    #[error("selector matched nothing: {0}")]
    EmptySelection(String),
    #[error("invalid input: {0}")]
    Invalid(String),
}

pub type Result<T> = std::result::Result<T, KernelError>;

fn kerr<E: std::fmt::Display>(e: E) -> KernelError {
    KernelError::Kernel(e.to_string())
}

pub type Vec3 = [f64; 3];

/// A sketch or primitive plane: the three principal planes, offset along their normal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum PlaneKind {
    #[default]
    XY,
    XZ,
    YZ,
}

impl PlaneKind {
    /// Right-handed `(u, v, normal)` axes of the plane.
    pub fn axes(self) -> (Vec3, Vec3, Vec3) {
        match self {
            Self::XY => ([1., 0., 0.], [0., 1., 0.], [0., 0., 1.]),
            Self::XZ => ([1., 0., 0.], [0., 0., 1.], [0., -1., 0.]),
            Self::YZ => ([0., 1., 0.], [0., 0., 1.], [1., 0., 0.]),
        }
    }

    pub fn to_world(self, offset: f64, p: [f64; 2]) -> Vec3 {
        let (u, v, n) = self.axes();
        std::array::from_fn(|i| u[i] * p[0] + v[i] * p[1] + n[i] * offset)
    }
}

/// One closed loop on a plane.
#[derive(Debug, Clone, PartialEq)]
pub enum Loop {
    Polygon(Vec<[f64; 2]>),
    Circle { center: [f64; 2], radius: f64 },
}

/// A triangle mesh in world coordinates, one vertex run per face so sharp edges stay sharp.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MeshData {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
    /// One polyline per topological edge.
    pub edges: Vec<Vec<[f32; 3]>>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Properties {
    pub volume: f64,
    pub area: f64,
    pub centroid: Vec3,
    pub bbox_min: Vec3,
    pub bbox_max: Vec3,
}

/// A handle to a shape held by a [`Bench`].
#[derive(Debug, Clone)]
pub struct Handle(Shape);

/// A kernel workspace. All handles used together must come from the same bench.
#[derive(Debug, Clone, Default)]
pub struct Bench {
    model: Model,
}

fn point(p: Vec3) -> Point {
    Point::new(p[0], p[1], p[2])
}

fn vector(v: Vec3) -> Vector {
    Vector::new(v[0], v[1], v[2])
}

fn direction(v: Vec3) -> Result<Direction> {
    Direction::from_coords(v[0], v[1], v[2], TOL).map_err(kerr)
}

/// A frame at `origin` with `axis` as its z, using whichever world axis is least parallel as x.
fn frame(origin: Vec3, axis: Vec3) -> Result<Frame> {
    let z = direction(axis)?;
    let reference = if axis[0].abs() < 0.9 * (axis[1].abs() + axis[2].abs()).max(1e-12) {
        Direction::X
    } else {
        Direction::Y
    };
    Frame::new(point(origin), z, reference, TOL).map_err(kerr)
}

impl Bench {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cuboid(&mut self, origin: Vec3, size: Vec3) -> Result<Handle> {
        let f = Frame::new(point(origin), Direction::Z, Direction::X, TOL).map_err(kerr)?;
        let built = ogeom::algo::make_box(&mut self.model, f, (size[0], size[1], size[2]), TOL);
        Ok(Handle(built.map_err(kerr)?.shape))
    }

    pub fn cylinder(
        &mut self,
        origin: Vec3,
        axis: Vec3,
        radius: f64,
        height: f64,
    ) -> Result<Handle> {
        let f = frame(origin, axis)?;
        let built = ogeom::algo::make_cylinder(&mut self.model, f, radius, height, TOL);
        Ok(Handle(built.map_err(kerr)?.shape))
    }

    pub fn cone(
        &mut self,
        origin: Vec3,
        axis: Vec3,
        base_radius: f64,
        top_radius: f64,
        height: f64,
    ) -> Result<Handle> {
        let f = frame(origin, axis)?;
        let built =
            ogeom::algo::make_cone(&mut self.model, f, base_radius, top_radius, height, TOL);
        Ok(Handle(built.map_err(kerr)?.shape))
    }

    pub fn sphere(&mut self, center: Vec3, radius: f64) -> Result<Handle> {
        let f = frame(center, [0., 0., 1.])?;
        let built = ogeom::algo::make_sphere(&mut self.model, f, radius, TOL);
        Ok(Handle(built.map_err(kerr)?.shape))
    }

    pub fn torus(&mut self, center: Vec3, axis: Vec3, major: f64, minor: f64) -> Result<Handle> {
        let f = frame(center, axis)?;
        let built = ogeom::algo::make_torus(&mut self.model, f, major, minor, TOL);
        Ok(Handle(built.map_err(kerr)?.shape))
    }

    fn wire(&mut self, plane: PlaneKind, offset: f64, l: &Loop) -> Result<Shape> {
        let (u, _, n) = plane.axes();
        match l {
            Loop::Polygon(points) => {
                let points: Vec<Point> =
                    points.iter().map(|p| point(plane.to_world(offset, *p))).collect();
                let built = ogeom::algo::make_polygon(&mut self.model, &points, true, TOL);
                Ok(built.map_err(kerr)?.shape)
            }
            Loop::Circle { center, radius } => {
                let origin = point(plane.to_world(offset, *center));
                let f = Frame::new(origin, direction(n)?, direction(u)?, TOL).map_err(kerr)?;
                let circle = Circle::new(f, *radius, TOL).map_err(kerr)?;
                let curve = Curve::Circle(CircleCurve::new(circle));
                let range = (0.0, std::f64::consts::TAU);
                let edge =
                    ogeom::algo::make_edge(&mut self.model, curve, range, TOL).map_err(kerr)?.shape;
                let built = ogeom::algo::make_wire(&mut self.model, &[edge], TOL);
                Ok(built.map_err(kerr)?.shape)
            }
        }
    }

    /// A planar face bounded by `outer`, with `holes` cut out of it.
    pub fn profile(
        &mut self,
        plane: PlaneKind,
        offset: f64,
        outer: &Loop,
        holes: &[Loop],
    ) -> Result<Handle> {
        let (u, _, n) = plane.axes();
        let origin = point(plane.to_world(offset, [0., 0.]));
        let f = Frame::new(origin, direction(n)?, direction(u)?, TOL).map_err(kerr)?;
        let surface = SurfaceGeometry::Plane(PlaneSurface::new(Plane::new(f)));
        let mut wires = vec![self.wire(plane, offset, outer)?];
        for hole in holes {
            wires.push(self.wire(plane, offset, hole)?);
        }
        let built = ogeom::algo::make_face(&mut self.model, surface, &wires, TOL);
        Ok(Handle(built.map_err(kerr)?.shape))
    }

    pub fn extrude(&mut self, profile: &Handle, by: Vec3, taper_deg: f64) -> Result<Handle> {
        let built = if taper_deg == 0.0 {
            ogeom::algo::make_prism(&mut self.model, &profile.0, vector(by), TOL)
        } else {
            let taper = taper_deg.to_radians();
            ogeom::algo::make_prism_tapered(&mut self.model, &profile.0, vector(by), taper, TOL)
        };
        Ok(Handle(built.map_err(kerr)?.shape))
    }

    pub fn revolve(
        &mut self,
        profile: &Handle,
        origin: Vec3,
        axis: Vec3,
        angle_deg: f64,
    ) -> Result<Handle> {
        let axis = Axis { location: point(origin), direction: direction(axis)? };
        let angle = angle_deg.to_radians();
        let built = ogeom::algo::make_revolution(&mut self.model, &profile.0, axis, angle, TOL);
        Ok(Handle(built.map_err(kerr)?.shape))
    }

    pub fn fuse(&mut self, a: &Handle, b: &Handle) -> Result<Handle> {
        let built = ogeom::boolean::fuse(&mut self.model, &a.0, &b.0, TOL);
        Ok(Handle(built.map_err(kerr)?.shape))
    }

    pub fn cut(&mut self, a: &Handle, b: &Handle) -> Result<Handle> {
        let built = ogeom::boolean::cut(&mut self.model, &a.0, &b.0, TOL);
        Ok(Handle(built.map_err(kerr)?.shape))
    }

    pub fn common(&mut self, a: &Handle, b: &Handle) -> Result<Handle> {
        let built = ogeom::boolean::common(&mut self.model, &a.0, &b.0, TOL);
        Ok(Handle(built.map_err(kerr)?.shape))
    }

    pub fn fillet(&mut self, solid: &Handle, sel: &EdgeSelector, radius: f64) -> Result<Handle> {
        let edges = self.select_edges(solid, sel)?;
        let built = ogeom::fillet::fillet_edges(&mut self.model, &solid.0, &edges, radius, TOL);
        Ok(Handle(built.map_err(kerr)?.shape))
    }

    pub fn chamfer(&mut self, solid: &Handle, sel: &EdgeSelector, distance: f64) -> Result<Handle> {
        let edges = self.select_edges(solid, sel)?;
        let built = ogeom::fillet::chamfer_edges(&mut self.model, &solid.0, &edges, distance, TOL);
        Ok(Handle(built.map_err(kerr)?.shape))
    }

    /// Hollow `solid`, opening the selected faces. Positive thickness hollows inward, negative grows the wall outward.
    pub fn shell(&mut self, solid: &Handle, open: &FaceSelector, thickness: f64) -> Result<Handle> {
        let faces = self.select_faces(solid, open)?;
        let built =
            ogeom::offset::make_thick_solid(&mut self.model, &solid.0, &faces, thickness, TOL);
        Ok(Handle(built.map_err(kerr)?.shape))
    }

    pub fn translate(&mut self, shape: &Handle, by: Vec3) -> Result<Handle> {
        self.place(shape, Transform::translation(vector(by)))
    }

    pub fn rotate(
        &mut self,
        shape: &Handle,
        origin: Vec3,
        axis: Vec3,
        angle_deg: f64,
    ) -> Result<Handle> {
        let axis = Axis { location: point(origin), direction: direction(axis)? };
        self.place(shape, Transform::rotation(axis, angle_deg.to_radians()))
    }

    pub fn mirror(&mut self, shape: &Handle, origin: Vec3, normal: Vec3) -> Result<Handle> {
        self.place(shape, Transform::plane_mirror(point(origin), direction(normal)?))
    }

    fn place(&mut self, shape: &Handle, transform: Transform) -> Result<Handle> {
        let built = ogeom::algo::transformed(&mut self.model, &shape.0, transform);
        Ok(Handle(built.map_err(kerr)?.shape))
    }

    /// Drop everything `keep` does not reach, so per-step snapshots stay small.
    pub fn compact(&mut self, keep: &Handle) -> Result<()> {
        self.model.retain_reachable(std::slice::from_ref(&keep.0)).map_err(kerr)
    }

    pub fn tessellate(&self, shape: &Handle, chord: f64) -> Result<MeshData> {
        let deflection = deflection(chord)?;
        let (faces, _) = ogeom::mesh::face_meshes_for(&self.model, &shape.0, deflection, None, TOL)
            .map_err(kerr)?;
        let mut out = MeshData::default();
        for (_, face) in faces {
            let face = face.map_err(kerr)?;
            let base = out.positions.len() as u32;
            for (i, p) in face.positions.iter().enumerate() {
                out.positions.push([p.x as f32, p.y as f32, p.z as f32]);
                out.normals.push(
                    face.normals
                        .get(i)
                        .map_or([0., 0., 1.], |n| [n.x as f32, n.y as f32, n.z as f32]),
                );
            }
            out.indices.extend(face.triangles.iter().flatten().map(|i| i + base));
        }
        for edge in explore_unique(&self.model, &shape.0, ShapeType::Edge).map_err(kerr)? {
            // Degenerate seam edges have no polyline, and nothing to draw anyway.
            let Ok(points) = ogeom::mesh::polyline_of_edge(&self.model, &edge, deflection, TOL)
            else {
                continue;
            };
            out.edges.push(points.iter().map(|p| [p.x as f32, p.y as f32, p.z as f32]).collect());
        }
        Ok(out)
    }

    pub fn properties(&self, shape: &Handle) -> Result<Properties> {
        let d = Deflection::default();
        let solid = ogeom::algo::volume_properties(&self.model, &shape.0, d, TOL).map_err(kerr)?;
        let area = ogeom::algo::surface_properties(&self.model, &shape.0, d, TOL).map_err(kerr)?;
        let bounds = ogeom::algo::shape_bounds(&self.model, &shape.0, TOL).map_err(kerr)?;
        let corner = |p: Option<Point>| p.map_or([0.; 3], |p| [p.x, p.y, p.z]);
        Ok(Properties {
            volume: solid.mass,
            area: area.mass,
            centroid: [solid.centre.x, solid.centre.y, solid.centre.z],
            bbox_min: corner(bounds.low()),
            bbox_max: corner(bounds.high()),
        })
    }

    /// The kernel's problems with `shape`; empty when it is valid.
    pub fn check(&self, shape: &Handle) -> Result<Vec<String>> {
        let diagnosis = ogeom::algo::check(&self.model, &shape.0, TOL).map_err(kerr)?;
        Ok(diagnosis.problems.iter().map(|p| format!("{p:?}")).collect())
    }
}

fn deflection(chord: f64) -> Result<Deflection> {
    Deflection::with_chord(chord).map_err(kerr)
}

/// A shared, immutable snapshot of one modelling result.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub bench: Arc<Bench>,
    pub shape: Handle,
}
