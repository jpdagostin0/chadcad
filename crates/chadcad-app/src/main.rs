//! The ChadCAD desktop application: Bevy renders, egui draws the interface.
//!
//! `chadcad [file.chad]` opens a document. Setting `CHADCAD_SCREENSHOT=out.png`
//! saves one window capture and exits, after `CHADCAD_SCREENSHOT_DELAY` seconds
//! (default 2), which is how the UI is checked without a person at the screen.
//! `CHADCAD_SELECT=name` starts with that feature selected.

mod editor;
mod studio;
mod ui;
mod viewport;

use std::path::PathBuf;
use std::time::Duration;

use bevy::log::LogPlugin;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::window::WindowResolution;
use bevy::winit::{UpdateMode, WinitSettings};
use bevy_egui::{EguiPlugin, EguiPrimaryContextPass};
use chadcad_model::Document;
use studio::Studio;

fn main() {
    let path = std::env::args_os().nth(1).map(PathBuf::from);
    let mut studio = Studio::new(Document::default(), None);
    if let Some(path) = path {
        studio.open_path(path);
    }
    if let Ok(name) = std::env::var("CHADCAD_SELECT") {
        let id = studio.doc.features.iter().find(|f| f.name == name).map(|f| f.id);
        studio.select(id);
    }
    let shot = std::env::var_os("CHADCAD_SCREENSHOT").map(PathBuf::from);

    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "ChadCAD".into(),
                    resolution: WindowResolution::new(1500, 950),
                    ..default()
                }),
                ..default()
            })
            .set(LogPlugin { filter: "wgpu=error,naga=warn,salsa=warn".into(), ..default() }),
    )
    .add_plugins(EguiPlugin::default())
    .insert_resource(studio)
    // Redraw on input, and often enough to pick up finished background builds.
    .insert_resource(WinitSettings {
        focused_mode: UpdateMode::reactive(Duration::from_millis(100)),
        unfocused_mode: UpdateMode::reactive_low_power(Duration::from_millis(500)),
    })
    .add_systems(Startup, viewport::setup)
    .add_systems(EguiPrimaryContextPass, ui::draw)
    .add_systems(
        Update,
        (
            studio::poll,
            viewport::fit_now,
            viewport::update_scene,
            viewport::fit_viewport,
            viewport::control_camera,
            viewport::draw_grid,
        )
            .chain(),
    );
    if let Some(path) = shot {
        let delay = std::env::var("CHADCAD_SCREENSHOT_DELAY").ok();
        let delay = delay.and_then(|d| d.parse().ok()).unwrap_or(2.);
        app.insert_resource(ShotTo(path, delay)).add_systems(Update, take_screenshot);
    }
    app.run();
}

#[derive(Resource)]
struct ShotTo(PathBuf, f32);

/// Waits for the build to land and the UI to settle, captures, then exits.
fn take_screenshot(
    mut commands: Commands,
    shot: Res<ShotTo>,
    studio: Res<Studio>,
    time: Res<Time<Real>>,
    mut ready_since: Local<Option<f32>>,
    mut taken: Local<u32>,
    mut exit: MessageWriter<AppExit>,
) {
    let now = time.elapsed_secs();
    if studio.building || studio.scene_stale {
        return;
    }
    let since = *ready_since.get_or_insert(now);
    if now - since < shot.1 {
        return;
    }
    if *taken == 0 {
        commands.spawn(Screenshot::primary_window()).observe(save_to_disk(shot.0.clone()));
        *taken = 1;
    } else if now - since > shot.1 + 2. {
        exit.write(AppExit::Success);
    }
}
