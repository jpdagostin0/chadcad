# ChadCAD

A parametric CAD application in Rust. Parameters and expressions drive an ordered
feature timeline, and a change rebuilds only what it actually affects.

| Part | Built on |
|---|---|
| B-rep kernel | [ogeom](https://github.com/gilbertorconde/ogeom-rs) |
| Renderer | [Bevy](https://bevyengine.org) |
| Interface | [egui](https://github.com/emilk/egui), through bevy_egui |
| Incremental timeline | [Salsa](https://github.com/salsa-rs/salsa) |
| Scripting | Lua 5.4, through [mlua](https://github.com/mlua-rs/mlua) |
| AI assistant | [genai](https://github.com/jeremychone/rust-genai) |
| Translations | [Fluent](https://projectfluent.org): English, Spanish, French, German |

## Running

Windows, Linux and macOS need Rust 1.90+ and a C compiler (for the vendored Lua).

    cargo run -p chadcad-app -- examples/bracket.chad

Right or middle drag orbits, Shift+drag pans and the wheel zooms. Click a feature in
the timeline to edit it, and right-click it to suppress, move, delete or roll back to it.

Headless tools:

    cargo run -p chadcad-cli -- info examples/bracket.chad
    cargo run -p chadcad-cli -- export examples/bracket.chad out.step
    cargo run -p chadcad-cli -- lua examples/bracket.lua part.chad
    cargo run -p chadcad-cli -- ask part.chad "add a 2 mm chamfer to the top edges"

## Features

- **Modelling:** box, cylinder, cone, sphere, torus, extrude, revolve, fillet, chamfer,
  shell, move, rotate, mirror, linear and polar patterns, and scripted features. Solids
  join, cut or intersect with the body.
- **Parameters:** named expressions such as `height * 0.6`, usable in any dimension.
- **Timeline:** rollback, suppress, reorder and undo. A feature that fails reports the
  reason and passes the body on, so the rest of the timeline keeps building.
- **Export:** STEP, IGES, BREP and ogeom native (exact); STL, OBJ, PLY, GLB and 3MF (mesh).
- **Lua:** a sandbox with no file, OS or network access. See
  [the API reference](crates/chadcad-lua/src/api.md).
- **AI:** the assistant edits the model through that same Lua sandbox and reads back
  each build report. A model name selects the provider (`claude-…`, `gpt-…`, `gemini-…`,
  or `ollama::name` for local models). `CHADCAD_MODEL` sets the default, and keys come
  from the provider's usual environment variable and are never stored.

## Limits

- Sketches are dimensioned profiles (rectangle, circle, polygon, regular polygon) driven
  by expressions. There is no interactive sketcher or geometric constraint solver yet.
- Edges and faces are chosen by geometric selectors (side, axis direction, curved,
  nearest to a point), not by clicking in the viewport. Nearest-point selectors take
  plain numbers, so they do not follow parameters.
- There is one body per document, and no STEP or IGES import.
- ogeom is young, and some blends it refuses (a chamfer next to a curved face, for
  example). Those show as feature errors rather than wrong geometry.

## Layout

    crates/chadcad-kernel    the only crate that touches ogeom
    crates/chadcad-model     the document, as plain serde data
    crates/chadcad-lua       the Lua sandbox
    crates/chadcad-timeline  Salsa queries from document to geometry
    crates/chadcad-ai        the assistant
    crates/chadcad-i18n      Fluent translations
    crates/chadcad-app       the desktop application
    crates/chadcad-cli       headless tools

To add a language, copy `crates/chadcad-i18n/locales/en.ftl`, translate it and add a row
to `LANGUAGES`. A test fails if any locale is missing a key.

## License

ChadCAD's own source is licensed under [Apache-2.0](LICENSE).

It depends on ogeom, which is **LGPL-2.1-only**. Distributed ChadCAD binaries link ogeom
statically, so whoever distributes one must also meet the LGPL for that part: chiefly,
give recipients ogeom's license and source and the means to relink against a modified
ogeom. Publishing ChadCAD's source alongside the binary satisfies the relinking part.
See [NOTICE](NOTICE).
