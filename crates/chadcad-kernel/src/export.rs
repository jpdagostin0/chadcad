//! Writing a shape out: exact B-rep formats and tessellated mesh formats.

use ogeom::doc::Document;
use ogeom::io::{self, ExportMesh, threemf};
use ogeom::mesh::triangulate;
use serde::{Deserialize, Serialize};

use crate::{Bench, Handle, KernelError, Result, TOL, deflection, kerr};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Format {
    Step,
    Iges,
    /// The exact B-rep in the BREP text format.
    Brep,
    /// ogeom's lossless native document.
    Native,
    StlBinary,
    StlAscii,
    Obj,
    Ply,
    Glb,
    ThreeMf,
}

impl Format {
    pub const ALL: [Self; 10] = [
        Self::Step,
        Self::Iges,
        Self::Brep,
        Self::Native,
        Self::StlBinary,
        Self::StlAscii,
        Self::Obj,
        Self::Ply,
        Self::Glb,
        Self::ThreeMf,
    ];

    pub fn extension(self) -> &'static str {
        match self {
            Self::Step => "step",
            Self::Iges => "iges",
            Self::Brep => "brep",
            Self::Native => "ogeom",
            Self::StlBinary | Self::StlAscii => "stl",
            Self::Obj => "obj",
            Self::Ply => "ply",
            Self::Glb => "glb",
            Self::ThreeMf => "3mf",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Step => "STEP",
            Self::Iges => "IGES",
            Self::Brep => "BREP",
            Self::Native => "ogeom",
            Self::StlBinary => "STL (binary)",
            Self::StlAscii => "STL (ASCII)",
            Self::Obj => "OBJ",
            Self::Ply => "PLY",
            Self::Glb => "glTF (GLB)",
            Self::ThreeMf => "3MF",
        }
    }

    /// Picks a format from a file extension; binary STL for `.stl`.
    pub fn from_extension(ext: &str) -> Result<Self> {
        Ok(match ext.to_ascii_lowercase().as_str() {
            "step" | "stp" => Self::Step,
            "iges" | "igs" => Self::Iges,
            "brep" => Self::Brep,
            "ogeom" => Self::Native,
            "stl" => Self::StlBinary,
            "obj" => Self::Obj,
            "ply" => Self::Ply,
            "glb" => Self::Glb,
            "3mf" => Self::ThreeMf,
            other => return Err(KernelError::Invalid(format!("unknown format .{other}"))),
        })
    }
}

impl Bench {
    /// Serialize `shape`. `chord` is the mesh deflection, ignored by exact formats.
    pub fn export(
        &self,
        shape: &Handle,
        name: &str,
        format: Format,
        chord: f64,
    ) -> Result<Vec<u8>> {
        let mesh = || triangulate(&self.model, &shape.0, deflection(chord)?, TOL).map_err(kerr);
        let document = || {
            let mut document = Document::over(self.model.clone());
            document.add_part(name, shape.0.clone());
            document
        };
        Ok(match format {
            Format::Step => io::write_step(&document(), TOL).map_err(kerr)?.into_bytes(),
            Format::Iges => io::write_iges(&document(), TOL).map_err(kerr)?.into_bytes(),
            Format::Brep => io::brep::write(&self.model, &shape.0, TOL).map_err(kerr)?.into_bytes(),
            Format::Native => {
                let options = io::native::WriteOptions::default();
                io::native::write(&self.model, std::slice::from_ref(&shape.0), options)
                    .map_err(kerr)?
                    .into_bytes()
            }
            Format::StlBinary => io::write(&mesh()?, io::Encoding::Binary).map_err(kerr)?,
            Format::StlAscii => io::write(&mesh()?, io::Encoding::Ascii).map_err(kerr)?,
            Format::Obj => {
                let mesh = mesh()?;
                io::write_obj(&[named(&mesh, name)]).into_bytes()
            }
            Format::Ply => {
                let mesh = mesh()?;
                io::write_ply(&named(&mesh, name)).into_bytes()
            }
            Format::Glb => {
                let mesh = mesh()?;
                io::write_glb(&[named(&mesh, name)])
            }
            Format::ThreeMf => {
                let mesh = mesh()?;
                let object = threemf::Object { mesh: &mesh, name: Some(name.into()) };
                io::write_3mf(&[object])
            }
        })
    }
}

fn named<'a>(mesh: &'a ogeom::topo::Triangulation, name: &str) -> ExportMesh<'a> {
    ExportMesh { name: Some(name.into()), ..ExportMesh::plain(mesh) }
}

/// Reads back the exact formats. Used by tests to prove an export is a real solid.
pub fn read_back_volume(bytes: &[u8], format: Format) -> Result<f64> {
    let text = std::str::from_utf8(bytes).map_err(kerr)?;
    let d = ogeom::mesh::Deflection::default();
    let (model, shape) = match format {
        Format::Step => {
            let import = io::read_step(text, TOL).map_err(kerr)?;
            let shape = import.solids.first().cloned();
            (import.document.model().clone(), shape)
        }
        Format::Iges => {
            let import = io::read_iges(text, TOL).map_err(kerr)?;
            let shape = import.solids.first().cloned();
            (import.document.model().clone(), shape)
        }
        Format::Brep => {
            let (model, shape) = io::brep::read(text, TOL).map_err(kerr)?;
            (model, Some(shape))
        }
        _ => return Err(KernelError::Invalid("not an exact text format".into())),
    };
    let shape = shape.ok_or_else(|| KernelError::Invalid("no solid in file".into()))?;
    Ok(ogeom::algo::volume_properties(&model, &shape, d, TOL).map_err(kerr)?.mass)
}
