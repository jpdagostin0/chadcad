//! The 3D view: a Z-up orbit camera, the body's mesh and edges, and the grid.

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::RenderLayers;
use bevy::camera::{CameraOutputMode, Viewport};
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::BlendState;
use bevy::window::PrimaryWindow;
use bevy_egui::{EguiGlobalSettings, PrimaryEguiContext};
use chadcad_kernel::MeshData;

use crate::studio::Studio;

const BACKGROUND: Color = Color::srgb(0.13, 0.14, 0.16);

#[derive(Component)]
pub struct Orbit {
    target: Vec3,
    distance: f32,
    yaw: f32,
    pitch: f32,
}

impl Default for Orbit {
    fn default() -> Self {
        Self { target: Vec3::ZERO, distance: 120., yaw: -0.8, pitch: 0.6 }
    }
}

impl Orbit {
    fn transform(&self) -> Transform {
        let (sin_pitch, cos_pitch) = self.pitch.sin_cos();
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        let offset = Vec3::new(cos_pitch * cos_yaw, cos_pitch * sin_yaw, sin_pitch);
        Transform::from_translation(self.target + offset * self.distance)
            .looking_at(self.target, Vec3::Z)
    }
}

#[derive(Component)]
pub struct BodyPart;

pub fn setup(mut commands: Commands, mut settings: ResMut<EguiGlobalSettings>) {
    settings.auto_create_primary_context = false;
    let orbit = Orbit::default();
    let transform = orbit.transform();
    commands
        .spawn((
            Camera3d::default(),
            Camera { clear_color: ClearColorConfig::Custom(BACKGROUND), ..default() },
            Projection::Perspective(PerspectiveProjection {
                near: 0.1,
                far: 100_000.,
                ..default()
            }),
            orbit,
            transform,
        ))
        // A headlight, so no side of the part is ever in the dark.
        .with_child((
            DirectionalLight { illuminance: 7_000., ..default() },
            Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.4, -0.5, 0.)),
        ));
    commands.insert_resource(GlobalAmbientLight { brightness: 500., ..default() });
    // egui draws over the 3D view from its own camera.
    commands.spawn((
        PrimaryEguiContext,
        Camera2d,
        RenderLayers::none(),
        Camera {
            order: 1,
            output_mode: CameraOutputMode::Write {
                blend_state: Some(BlendState::ALPHA_BLENDING),
                clear_color: ClearColorConfig::None,
            },
            clear_color: ClearColorConfig::Custom(Color::NONE),
            ..default()
        },
    ));
}

fn solid_mesh(data: &MeshData) -> Mesh {
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, data.positions.clone())
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, data.normals.clone())
        .with_inserted_indices(Indices::U32(data.indices.clone()))
}

fn edge_mesh(data: &MeshData) -> Mesh {
    let mut lines: Vec<[f32; 3]> = Vec::new();
    for polyline in &data.edges {
        for pair in polyline.windows(2) {
            lines.extend_from_slice(pair);
        }
    }
    Mesh::new(PrimitiveTopology::LineList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, lines)
}

pub fn update_scene(
    mut commands: Commands,
    mut studio: ResMut<Studio>,
    old: Query<Entity, With<BodyPart>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut camera: Query<&mut Orbit>,
) {
    if !studio.scene_stale {
        return;
    }
    studio.scene_stale = false;
    for entity in &old {
        commands.entity(entity).despawn();
    }
    let data = studio.build.mesh.clone();
    if data.indices.is_empty() {
        return;
    }
    commands.spawn((
        BodyPart,
        Mesh3d(meshes.add(solid_mesh(&data))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.62, 0.66, 0.72),
            perceptual_roughness: 0.55,
            metallic: 0.05,
            double_sided: true,
            cull_mode: None,
            // Line pipelines cannot take a depth bias, so push the faces back instead.
            depth_bias: -200.,
            ..default()
        })),
    ));
    if !data.edges.is_empty() {
        commands.spawn((
            BodyPart,
            Mesh3d(meshes.add(edge_mesh(&data))),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: Color::srgb(0.03, 0.04, 0.05),
                unlit: true,
                ..default()
            })),
        ));
    }
    if studio.fit_requested
        && let Some(properties) = studio.build.properties
        && let Ok(mut orbit) = camera.single_mut()
    {
        studio.fit_requested = false;
        let low = Vec3::from_array(properties.bbox_min.map(|c| c as f32));
        let high = Vec3::from_array(properties.bbox_max.map(|c| c as f32));
        orbit.target = (low + high) / 2.;
        orbit.distance = (high - low).length().max(1.) * 1.6;
    }
}

/// Requests a refit even when the body has not changed.
pub fn fit_now(mut studio: ResMut<Studio>) {
    if studio.fit_requested && !studio.building {
        studio.scene_stale = true;
    }
}

pub fn control_camera(
    studio: Res<Studio>,
    window: Single<&Window, With<PrimaryWindow>>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    mut camera: Query<(&mut Orbit, &mut Transform)>,
) {
    let Ok((mut orbit, mut transform)) = camera.single_mut() else { return };
    let scale = window.scale_factor();
    let inside = window
        .cursor_position()
        .is_some_and(|p| studio.viewport.contains(p * scale) && !studio.pointer_on_ui);
    if inside {
        let delta = motion.delta;
        let dragging = buttons.pressed(MouseButton::Right) || buttons.pressed(MouseButton::Middle);
        if dragging && keys.pressed(KeyCode::ShiftLeft) {
            let per_pixel = orbit.distance * 0.0012;
            let right = transform.right().as_vec3();
            let up = transform.up().as_vec3();
            orbit.target += (up * delta.y - right * delta.x) * per_pixel;
        } else if dragging {
            orbit.yaw -= delta.x * 0.006;
            orbit.pitch = (orbit.pitch + delta.y * 0.006).clamp(-1.55, 1.55);
        }
        let notches = match scroll.unit {
            MouseScrollUnit::Line => scroll.delta.y,
            MouseScrollUnit::Pixel => scroll.delta.y / 40.,
        };
        orbit.distance = (orbit.distance * 0.9_f32.powf(notches)).clamp(0.1, 50_000.);
    }
    let wanted = orbit.transform();
    if *transform != wanted {
        *transform = wanted;
    }
}

/// Keeps the 3D camera in the space the panels leave free.
pub fn fit_viewport(
    studio: Res<Studio>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut camera: Single<&mut Camera, With<Camera3d>>,
) {
    let rect = studio.viewport;
    let size = rect.size().max(Vec2::ONE).as_uvec2();
    let limit = UVec2::new(window.physical_width(), window.physical_height());
    let position = rect.min.as_uvec2().min(limit.saturating_sub(UVec2::ONE));
    let size = size.min(limit - position);
    if size.x == 0 || size.y == 0 {
        return;
    }
    let viewport = Viewport { physical_position: position, physical_size: size, ..default() };
    if camera.viewport.as_ref().is_none_or(|v| {
        v.physical_position != viewport.physical_position
            || v.physical_size != viewport.physical_size
    }) {
        camera.viewport = Some(viewport);
    }
}

/// A floor grid whose spacing follows the zoom, with the X, Y and Z axes coloured.
pub fn draw_grid(mut gizmos: Gizmos, camera: Query<&Orbit>) {
    let Ok(orbit) = camera.single() else { return };
    let cell = 10_f32.powf((orbit.distance / 8.).max(0.001).log10().floor());
    let lines = 40;
    let reach = cell * lines as f32;
    let centre = (orbit.target / cell).round() * cell;
    let shade = Color::srgba(0.5, 0.55, 0.6, 0.18);
    for i in -lines..=lines {
        let along = i as f32 * cell;
        let (cx, cy) = (centre.x + along, centre.y + along);
        gizmos.line(
            Vec3::new(cx, centre.y - reach, 0.),
            Vec3::new(cx, centre.y + reach, 0.),
            shade,
        );
        gizmos.line(
            Vec3::new(centre.x - reach, cy, 0.),
            Vec3::new(centre.x + reach, cy, 0.),
            shade,
        );
    }
    let axis = orbit.distance * 0.15;
    gizmos.line(Vec3::ZERO, Vec3::X * axis, Color::srgb(0.9, 0.25, 0.25));
    gizmos.line(Vec3::ZERO, Vec3::Y * axis, Color::srgb(0.3, 0.8, 0.3));
    gizmos.line(Vec3::ZERO, Vec3::Z * axis, Color::srgb(0.3, 0.45, 0.95));
}
