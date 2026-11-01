//! The ChadCAD document: parameters and an ordered feature timeline.
//!
//! This is plain data. The Salsa engine in `chadcad-timeline` turns it into
//! geometry, and Lua and the AI assistant edit it through serde, so a new
//! feature kind is reachable from all of them as soon as it exists here.

pub mod expr;

use chadcad_kernel::{EdgeSelector, FaceSelector, PlaneKind};
use serde::{Deserialize, Serialize};

pub use expr::{Expr, eval};

pub const FILE_EXTENSION: &str = "chad";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Parameter {
    pub name: String,
    pub expr: Expr,
}

pub type Xyz = [Expr; 3];
pub type Uv = [Expr; 2];

/// How a feature's solid meets the body so far.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Combine {
    #[default]
    Join,
    Cut,
    Intersect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum AxisKind {
    X,
    Y,
    #[default]
    Z,
}

impl AxisKind {
    pub fn vector(self) -> [f64; 3] {
        match self {
            Self::X => [1., 0., 0.],
            Self::Y => [0., 1., 0.],
            Self::Z => [0., 0., 1.],
        }
    }
}

/// One closed loop of a sketch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Profile {
    /// `center` defaults to the sketch origin.
    Rect {
        #[serde(default)]
        center: Uv,
        size: Uv,
    },
    Circle {
        #[serde(default)]
        center: Uv,
        radius: Expr,
    },
    Polygon {
        points: Vec<Uv>,
    },
    RegularPolygon {
        #[serde(default)]
        center: Uv,
        radius: Expr,
        sides: Expr,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sketch {
    #[serde(default)]
    pub plane: PlaneKind,
    /// Distance from the origin along the plane's normal.
    #[serde(default)]
    pub offset: Expr,
    pub outer: Profile,
    #[serde(default)]
    pub holes: Vec<Profile>,
}

fn one() -> Expr {
    Expr("1".into())
}

fn full_turn() -> Expr {
    Expr("360".into())
}

fn x_axis() -> Xyz {
    [one(), Expr::default(), Expr::default()]
}

fn all_edges() -> EdgeSelector {
    EdgeSelector::All
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FeatureKind {
    Box {
        #[serde(default)]
        origin: Xyz,
        size: Xyz,
        #[serde(default)]
        op: Combine,
    },
    Cylinder {
        #[serde(default)]
        origin: Xyz,
        #[serde(default)]
        axis: AxisKind,
        radius: Expr,
        height: Expr,
        #[serde(default)]
        op: Combine,
    },
    Cone {
        #[serde(default)]
        origin: Xyz,
        #[serde(default)]
        axis: AxisKind,
        base_radius: Expr,
        #[serde(default)]
        top_radius: Expr,
        height: Expr,
        #[serde(default)]
        op: Combine,
    },
    Sphere {
        #[serde(default)]
        center: Xyz,
        radius: Expr,
        #[serde(default)]
        op: Combine,
    },
    Torus {
        #[serde(default)]
        center: Xyz,
        #[serde(default)]
        axis: AxisKind,
        major: Expr,
        minor: Expr,
        #[serde(default)]
        op: Combine,
    },
    Extrude {
        sketch: Sketch,
        distance: Expr,
        /// Degrees; positive narrows toward the far end.
        #[serde(default)]
        taper: Expr,
        #[serde(default)]
        op: Combine,
    },
    Revolve {
        sketch: Sketch,
        #[serde(default)]
        axis: AxisKind,
        #[serde(default)]
        axis_origin: Xyz,
        #[serde(default = "full_turn")]
        angle: Expr,
        #[serde(default)]
        op: Combine,
    },
    Fillet {
        #[serde(default = "all_edges")]
        edges: EdgeSelector,
        radius: Expr,
    },
    Chamfer {
        #[serde(default = "all_edges")]
        edges: EdgeSelector,
        distance: Expr,
    },
    /// Hollow the body, opening the selected faces.
    Shell {
        open: FaceSelector,
        thickness: Expr,
    },
    Translate {
        by: Xyz,
    },
    Rotate {
        #[serde(default)]
        origin: Xyz,
        #[serde(default)]
        axis: AxisKind,
        angle: Expr,
    },
    Mirror {
        #[serde(default)]
        origin: Xyz,
        /// Mirror across the plane through `origin` normal to this axis.
        #[serde(default)]
        normal: AxisKind,
        /// Keep the original and fuse the reflection onto it.
        #[serde(default)]
        merge: bool,
    },
    LinearPattern {
        #[serde(default = "x_axis")]
        direction: Xyz,
        count: Expr,
        spacing: Expr,
    },
    PolarPattern {
        #[serde(default)]
        origin: Xyz,
        #[serde(default)]
        axis: AxisKind,
        count: Expr,
        #[serde(default = "full_turn")]
        angle: Expr,
    },
    /// A solid computed by a sandboxed Lua script; see `chadcad-lua`.
    Script {
        source: String,
        #[serde(default)]
        op: Combine,
        /// The script's result is the new whole body, built from `body` itself.
        #[serde(default)]
        replace: bool,
    },
}

impl FeatureKind {
    /// The variant name, also the serde tag and the i18n key suffix.
    pub fn tag(&self) -> &'static str {
        match self {
            Self::Box { .. } => "Box",
            Self::Cylinder { .. } => "Cylinder",
            Self::Cone { .. } => "Cone",
            Self::Sphere { .. } => "Sphere",
            Self::Torus { .. } => "Torus",
            Self::Extrude { .. } => "Extrude",
            Self::Revolve { .. } => "Revolve",
            Self::Fillet { .. } => "Fillet",
            Self::Chamfer { .. } => "Chamfer",
            Self::Shell { .. } => "Shell",
            Self::Translate { .. } => "Translate",
            Self::Rotate { .. } => "Rotate",
            Self::Mirror { .. } => "Mirror",
            Self::LinearPattern { .. } => "LinearPattern",
            Self::PolarPattern { .. } => "PolarPattern",
            Self::Script { .. } => "Script",
        }
    }
}

pub type FeatureId = u64;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Feature {
    pub id: FeatureId,
    pub name: String,
    #[serde(default)]
    pub suppressed: bool,
    pub kind: FeatureKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub name: String,
    #[serde(default)]
    pub parameters: Vec<Parameter>,
    #[serde(default)]
    pub features: Vec<Feature>,
    /// Build only the first `n` features; `None` builds them all.
    #[serde(default)]
    pub rollback: Option<usize>,
    #[serde(default)]
    pub next_id: FeatureId,
}

impl Default for Document {
    fn default() -> Self {
        Self {
            name: "Untitled".into(),
            parameters: Vec::new(),
            features: Vec::new(),
            rollback: None,
            next_id: 1,
        }
    }
}

impl Document {
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    pub fn from_json(text: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(text)
    }

    /// Creates or updates a parameter.
    pub fn set_parameter(&mut self, name: &str, expr: Expr) {
        match self.parameters.iter_mut().find(|p| p.name == name) {
            Some(p) => p.expr = expr,
            None => self.parameters.push(Parameter { name: name.to_owned(), expr }),
        }
    }

    /// Appends at the rollback position and returns the new feature's id.
    pub fn add_feature(&mut self, kind: FeatureKind) -> FeatureId {
        let id = self.next_id.max(1);
        self.next_id = id + 1;
        let tag = kind.tag();
        let serial = self.features.iter().filter(|f| f.kind.tag() == tag).count() + 1;
        let feature = Feature { id, name: format!("{tag}{serial}"), suppressed: false, kind };
        match self.rollback {
            Some(at) if at < self.features.len() => {
                self.features.insert(at, feature);
                self.rollback = Some(at + 1);
            }
            _ => self.features.push(feature),
        }
        id
    }

    pub fn feature(&self, id: FeatureId) -> Option<&Feature> {
        self.features.iter().find(|f| f.id == id)
    }

    pub fn feature_mut(&mut self, id: FeatureId) -> Option<&mut Feature> {
        self.features.iter_mut().find(|f| f.id == id)
    }

    pub fn remove_feature(&mut self, id: FeatureId) -> bool {
        let Some(at) = self.features.iter().position(|f| f.id == id) else { return false };
        self.features.remove(at);
        if let Some(r) = &mut self.rollback
            && at < *r
        {
            *r -= 1;
        }
        true
    }

    /// Moves a feature `delta` places along the timeline, clamped to its ends.
    pub fn move_feature(&mut self, id: FeatureId, delta: isize) {
        let Some(at) = self.features.iter().position(|f| f.id == id) else { return };
        let to = at.saturating_add_signed(delta).min(self.features.len() - 1);
        let feature = self.features.remove(at);
        self.features.insert(to, feature);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube() -> FeatureKind {
        FeatureKind::Box {
            origin: Default::default(),
            size: ["w".into(), 2.0.into(), "w/2".into()],
            op: Combine::Join,
        }
    }

    #[test]
    fn json_round_trip() {
        let mut d = Document::default();
        d.set_parameter("w", "10".into());
        d.add_feature(cube());
        assert_eq!(Document::from_json(&d.to_json().unwrap()).unwrap(), d);
    }

    #[test]
    fn numbers_and_defaults_deserialize() {
        let k: FeatureKind = serde_json::from_str(r#"{"Box":{"size":[1,2.5,"w"]}}"#).unwrap();
        let FeatureKind::Box { size, origin, op } = k else { panic!() };
        assert_eq!(size[0].0, "1");
        assert_eq!(size[1].0, "2.5");
        assert_eq!(origin[0].0, "0");
        assert_eq!(op, Combine::Join);
    }

    #[test]
    fn inserts_at_rollback_and_names_uniquely() {
        let mut d = Document::default();
        let a = d.add_feature(cube());
        let c = d.add_feature(cube());
        d.rollback = Some(1);
        let b = d.add_feature(cube());
        assert_eq!(d.features.iter().map(|f| f.id).collect::<Vec<_>>(), [a, b, c]);
        assert_eq!(d.rollback, Some(2));
        assert_eq!(d.features[1].name, "Box3");
        d.remove_feature(a);
        assert_eq!(d.rollback, Some(1));
        d.move_feature(c, -5);
        assert_eq!(d.features[0].id, c);
    }
}
