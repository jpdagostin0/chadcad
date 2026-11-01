//! The ChadCAD desktop application: Bevy renders, egui draws the interface.
//!
//! `chadcad [file.chad]` opens a document. Setting `CHADCAD_SCREENSHOT=out.png`
//! saves one window capture and exits, after `CHADCAD_SCREENSHOT_DELAY` seconds
//! (default 2), which is how the UI is checked without a person at the screen.
//! `CHADCAD_SELECT=name` starts with that feature selected, and `CHADCAD_CLICKS=x,y;x,y`
//! replays clicks first.

mod editor;
mod studio;
mod ui;
mod viewport;

use std::path::PathBuf;
use std::time::Duration;

use bevy::input::ButtonState;
use bevy::input::mouse::MouseButtonInput;
use bevy::log::LogPlugin;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::window::{CursorMoved, PrimaryWindow, WindowEvent, WindowResolution};
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
        if let Ok(clicks) = std::env::var("CHADCAD_CLICKS") {
            app.insert_resource(Clicks(parse_clicks(&clicks))).add_systems(Update, replay_clicks);
        }
    }
    app.run();
}

/// Pointer clicks to replay, in logical pixels: `x,y;x,y`.
#[derive(Resource)]
struct Clicks(Vec<Vec2>);

fn parse_clicks(text: &str) -> Vec<Vec2> {
    let pair = |p: &str| {
        let (x, y) = p.split_once(',')?;
        Some(Vec2::new(x.trim().parse().ok()?, y.trim().parse().ok()?))
    };
    text.split(';').filter_map(pair).collect()
}

/// Feeds clicks into `WindowEvent`, the same stream winit writes and egui reads, so a
/// capture can show what the interface does when used. One click per 40 frames.
fn replay_clicks(
    clicks: Res<Clicks>,
    window: Single<Entity, With<PrimaryWindow>>,
    studio: Res<Studio>,
    mut frame: Local<usize>,
    mut events: MessageWriter<WindowEvent>,
) {
    if studio.building {
        return;
    }
    *frame += 1;
    let (index, phase) = (*frame / 40, *frame % 40);
    let Some(position) = clicks.0.get(index) else { return };
    let window = *window;
    let button = |state| MouseButtonInput { button: MouseButton::Left, state, window };
    match phase {
        5 => {
            let cursor = CursorMoved { window, position: *position, delta: None };
            events.write(WindowEvent::CursorMoved(cursor));
        }
        10 => {
            events.write(WindowEvent::MouseButtonInput(button(ButtonState::Pressed)));
        }
        13 => {
            events.write(WindowEvent::MouseButtonInput(button(ButtonState::Released)));
        }
        _ => {}
    }
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
