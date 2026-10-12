//! Playground 3D — a desktop 3D gem-collector game built with Rust + Bevy.
//!
//! Controls:
//! - Move: WASD / arrow keys
//! - Jump: Space
//! - Orbit camera: hold right mouse button and drag
//! - Zoom: mouse wheel
//! - Restart: R (after winning or anytime)
//!
//! Goal: collect all gems scattered around the arena.

use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::prelude::*;
use bevy::text::FontSize;
use bevy::window::{Window, WindowPlugin};
#[cfg(not(target_arch = "wasm32"))]
use bevy::window::PresentMode;
use std::f32::consts::PI;

// ---------------------------------------------------------------------------
// Tuning constants (pure data, unit-tested below)
// ---------------------------------------------------------------------------

/// Half-extent of the square arena on X/Z.
pub const ARENA_HALF: f32 = 14.0;
/// Player move speed in units/sec.
pub const PLAYER_SPEED: f32 = 8.0;
/// Jump impulse (initial vertical velocity).
pub const JUMP_SPEED: f32 = 7.0;
/// Gravity applied to the player.
pub const GRAVITY: f32 = 20.0;
/// Distance at which a gem counts as collected.
pub const COLLECT_RADIUS: f32 = 1.4;
/// How many gems to collect to win.
pub const GEM_COUNT: usize = 12;

// ---------------------------------------------------------------------------
// Components & resources
// ---------------------------------------------------------------------------

#[derive(Component)]
struct Player {
    vel_y: f32,
}

#[derive(Component)]
struct Gem {
    base_y: f32,
    phase: f32,
}

#[derive(Component)]
struct Obstacle {
    half_extents: Vec2,
}

#[derive(Component)]
struct FollowCamera {
    yaw: f32,
    pitch: f32,
    distance: f32,
}

#[derive(Component)]
struct ScoreText;

#[derive(Component)]
struct MessageText;

#[derive(Resource, Default)]
struct Game {
    score: u32,
    elapsed: f32,
    won: bool,
}

// ---------------------------------------------------------------------------
// Pure helpers (kept dependency-free so `cargo test` needs no GPU/window)
// ---------------------------------------------------------------------------

/// Clamp a position into the arena bounds.
pub fn clamp_to_arena(p: Vec3) -> Vec3 {
    Vec3::new(
        p.x.clamp(-ARENA_HALF, ARENA_HALF),
        p.y,
        p.z.clamp(-ARENA_HALF, ARENA_HALF),
    )
}

/// True when the player is close enough to collect a gem (XZ + Y aware).
pub fn is_collected(player: Vec3, gem: Vec3) -> bool {
    player.distance(gem) < COLLECT_RADIUS
}

/// Push a circle (player) out of an axis-aligned box (obstacle) on XZ.
/// Returns the corrected XZ position.
pub fn resolve_circle_vs_box(pos: Vec2, center: Vec2, half: Vec2, radius: f32) -> Vec2 {
    let closest = Vec2::new(
        pos.x.clamp(center.x - half.x, center.x + half.x),
        pos.y.clamp(center.y - half.y, center.y + half.y),
    );
    let delta = pos - closest;
    let dist = delta.length();
    if dist < radius {
        if dist > f32::EPSILON {
            closest + delta / dist * radius
        } else {
            // Center inside the box: push out along the smallest penetration axis.
            let dx = half.x - (pos.x - center.x).abs();
            let dz = half.y - (pos.y - center.y).abs();
            if dx < dz {
                Vec2::new(
                    center.x + (pos.x - center.x).signum().max(1.0) * (half.x + radius),
                    pos.y,
                )
            } else {
                Vec2::new(
                    pos.x,
                    center.y + (pos.y - center.y).signum().max(1.0) * (half.y + radius),
                )
            }
        }
    } else {
        pos
    }
}

/// Deterministic scatter position for gem `index` with a `salt` (no RNG dep).
pub fn scatter_pos(index: usize, salt: u32) -> Vec3 {
    // Simple integer hash -> pseudo random in [-ARENA_HALF+1, ARENA_HALF-1].
    let mut h = (index as u32)
        .wrapping_mul(2654435761)
        .wrapping_add((salt.wrapping_mul(40503)).wrapping_add(0x9E37_79B9));
    h ^= h >> 15;
    h = h.wrapping_mul(0x85EB_CA6B);
    h ^= h >> 13;
    let fx = ((h & 0xFFFF) as f32 / 65535.0) * 2.0 - 1.0;
    let fz = (((h >> 16) & 0xFFFF) as f32 / 65535.0) * 2.0 - 1.0;
    let r = ARENA_HALF - 2.0;
    Vec3::new(fx * r, 1.0, fz * r)
}

// ---------------------------------------------------------------------------
// App entry
// ---------------------------------------------------------------------------

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(game_window()),
            ..default()
        }))
        .init_resource::<Game>()
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (
                player_movement,
                collect_gems,
                spin_gems,
                follow_camera,
                update_ui,
                restart_game,
            ),
        )
        .run();
}

// ---------------------------------------------------------------------------
// Scene setup
// ---------------------------------------------------------------------------

// Native desktop window: fixed 720p, vsync.
#[cfg(not(target_arch = "wasm32"))]
fn game_window() -> Window {
    Window {
        title: "Playground 3D — Rust Gem Collector (Desktop)".into(),
        resolution: (1280, 720).into(),
        present_mode: PresentMode::AutoVsync,
        ..default()
    }
}

// Browser window: render into the #bevy canvas and track the page size.
#[cfg(target_arch = "wasm32")]
fn game_window() -> Window {
    Window {
        title: "Playground 3D — Rust Gem Collector".into(),
        canvas: Some("#bevy".into()),
        fit_canvas_to_parent: true,
        prevent_default_event_handling: false,
        ..default()
    }
}

#[allow(clippy::too_many_arguments)]
fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // Soft global ambient so shadows are never pitch black.
    commands.insert_resource(GlobalAmbientLight {
        color: Color::srgb(0.9, 0.95, 1.0).into(),
        brightness: 350.0,
        ..default()
    });

    // Sun.
    commands.spawn((
        DirectionalLight {
            illuminance: 8_000.0,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform {
            translation: Vec3::new(6.0, 12.0, 4.0),
            rotation: Quat::from_euler(EulerRot::XYZ, -0.9, 0.5, 0.0),
            ..default()
        },
    ));

    // Warm point light over the middle for night-time pop.
    commands.spawn((
        PointLight {
            intensity: 60_000.0,
            color: Color::srgb(1.0, 0.8, 0.55).into(),
            shadow_maps_enabled: false,
            range: 30.0,
            ..default()
        },
        Transform::from_xyz(0.0, 6.0, 0.0),
    ));

    // Ground.
    commands.spawn((
        Mesh3d(
            meshes.add(
                Plane3d::default()
                    .mesh()
                    .size(2.0 * ARENA_HALF + 6.0, 2.0 * ARENA_HALF + 6.0),
            ),
        ),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.16, 0.32, 0.20),
            perceptual_roughness: 1.0,
            ..default()
        })),
        Transform::IDENTITY,
    ));

    // Arena border walls (low rims so the player sees the bounds).
    let wall_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.45, 0.48, 0.55),
        perceptual_roughness: 0.9,
        ..default()
    });
    let rim_h = 0.6;
    let rim_t = 0.4;
    let span = 2.0 * ARENA_HALF + rim_t;
    for (pos, size) in [
        (
            Vec3::new(0.0, rim_h / 2.0, ARENA_HALF),
            Vec3::new(span, rim_h, rim_t),
        ),
        (
            Vec3::new(0.0, rim_h / 2.0, -ARENA_HALF),
            Vec3::new(span, rim_h, rim_t),
        ),
        (
            Vec3::new(ARENA_HALF, rim_h / 2.0, 0.0),
            Vec3::new(rim_t, rim_h, span),
        ),
        (
            Vec3::new(-ARENA_HALF, rim_h / 2.0, 0.0),
            Vec3::new(rim_t, rim_h, span),
        ),
    ] {
        commands.spawn((
            Mesh3d(meshes.add(Cuboid::new(size.x, size.y, size.z))),
            MeshMaterial3d(wall_mat.clone()),
            Transform::from_translation(pos),
        ));
    }

    // Obstacle pillars.
    let pillar_mesh = meshes.add(Cuboid::new(1.6, 3.0, 1.6));
    let pillar_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.55, 0.36, 0.22),
        perceptual_roughness: 0.85,
        ..default()
    });
    for (x, z) in [
        (-6.0, -4.0),
        (5.5, -6.0),
        (-5.0, 6.0),
        (6.0, 5.0),
        (0.0, -10.0),
    ] {
        commands.spawn((
            Mesh3d(pillar_mesh.clone()),
            MeshMaterial3d(pillar_mat.clone()),
            Transform::from_xyz(x, 1.5, z),
            Obstacle {
                half_extents: Vec2::new(0.8, 0.8),
            },
        ));
    }

    // Player: bright cube.
    let player_mesh = meshes.add(Cuboid::new(0.9, 0.9, 0.9));
    let player_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.25, 0.55, 1.0),
        perceptual_roughness: 0.4,
        metallic: 0.1,
        ..default()
    });
    commands.spawn((
        Mesh3d(player_mesh),
        MeshMaterial3d(player_mat),
        Transform::from_xyz(0.0, 0.45, 8.0),
        Player { vel_y: 0.0 },
    ));

    // Gems: small unlit golden spheres (unlit => always bright, cheap glow look).
    let gem_mesh = meshes.add(Sphere::new(0.42));
    let gem_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(1.0, 0.78, 0.15),
        unlit: true,
        ..default()
    });
    for i in 0..GEM_COUNT {
        let p = scatter_pos(i, 1);
        commands.spawn((
            Mesh3d(gem_mesh.clone()),
            MeshMaterial3d(gem_mat.clone()),
            Transform::from_translation(p),
            Gem {
                base_y: 1.0,
                phase: i as f32 * 0.7,
            },
        ));
    }

    // Follow camera.
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 9.0, 16.0).looking_at(Vec3::ZERO, Vec3::Y),
        FollowCamera {
            yaw: 0.0,
            pitch: 0.55,
            distance: 13.0,
        },
    ));

    // UI: score (top-left).
    commands.spawn((
        Text::new("Gems: 0/12"),
        TextFont {
            font_size: FontSize::Px(28.0),
            ..default()
        },
        TextColor(Color::WHITE.into()),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(12.0),
            left: Val::Px(14.0),
            ..default()
        },
        ScoreText,
    ));

    // UI: help (bottom-left).
    commands.spawn((
        Text::new("WASD/arrows move · Space jump · RMB-drag orbit · wheel zoom · R restart"),
        TextFont {
            font_size: FontSize::Px(18.0),
            ..default()
        },
        TextColor(Color::srgb(0.85, 0.88, 0.92).into()),
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(12.0),
            left: Val::Px(14.0),
            ..default()
        },
    ));

    // UI: center win message (hidden until win).
    commands.spawn((
        Text::new("You collected all gems!  Press R to play again."),
        TextFont {
            font_size: FontSize::Px(34.0),
            ..default()
        },
        TextColor(Color::srgb(1.0, 0.9, 0.4).into()),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Percent(40.0),
            left: Val::Percent(50.0),
            ..default()
        },
        Visibility::Hidden,
        MessageText,
    ));
}

// ---------------------------------------------------------------------------
// Gameplay systems
// ---------------------------------------------------------------------------

fn player_movement(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    obstacles: Query<(&Transform, &Obstacle), (Without<Player>, Without<FollowCamera>)>,
    mut player: Query<(&mut Transform, &mut Player), (Without<Obstacle>, Without<FollowCamera>)>,
    camera: Query<&Transform, (With<FollowCamera>, Without<Player>, Without<Obstacle>)>,
) {
    let Ok(cam_t) = camera.single() else { return };
    let Ok((mut t, mut p)) = player.single_mut() else {
        return;
    };
    let dt = time.delta_secs();

    // Camera-relative move axes projected on the ground plane.
    let mut fwd = cam_t.forward().as_vec3();
    fwd.y = 0.0;
    let fwd = fwd.try_normalize().unwrap_or(Vec3::NEG_Z);
    let right = Vec3::new(-fwd.z, 0.0, fwd.x);

    // W moves away from the camera (into the scene), S toward it.
    let mut dir = Vec3::ZERO;
    if keys.pressed(KeyCode::KeyW) || keys.pressed(KeyCode::ArrowUp) {
        dir += fwd;
    }
    if keys.pressed(KeyCode::KeyS) || keys.pressed(KeyCode::ArrowDown) {
        dir -= fwd;
    }
    if keys.pressed(KeyCode::KeyA) || keys.pressed(KeyCode::ArrowLeft) {
        dir -= right;
    }
    if keys.pressed(KeyCode::KeyD) || keys.pressed(KeyCode::ArrowRight) {
        dir += right;
    }

    if dir.length_squared() > 0.0 {
        dir = dir.normalize();
        t.translation += dir * PLAYER_SPEED * dt;
        // Face travel direction with a snappy turn.
        let target_yaw = dir.x.atan2(dir.z);
        let current = t.rotation.to_euler(EulerRot::YXZ).0;
        let mut delta = target_yaw - current;
        while delta > PI {
            delta -= 2.0 * PI;
        }
        while delta < -PI {
            delta += 2.0 * PI;
        }
        t.rotation = Quat::from_rotation_y(current + delta * (12.0 * dt).min(1.0));
    }

    // Jump + gravity (ground at y = 0.45 = half cube height).
    let grounded = t.translation.y <= 0.451;
    if grounded && keys.just_pressed(KeyCode::Space) {
        p.vel_y = JUMP_SPEED;
    }
    if !grounded || p.vel_y > 0.0 {
        p.vel_y -= GRAVITY * dt;
        t.translation.y += p.vel_y * dt;
        if t.translation.y < 0.45 {
            t.translation.y = 0.45;
            p.vel_y = 0.0;
        }
    }

    // Arena clamp.
    t.translation = clamp_to_arena(t.translation);

    // Obstacle push-out (XZ only).
    let mut xz = Vec2::new(t.translation.x, t.translation.z);
    for (ot, ob) in obstacles.iter() {
        let c = Vec2::new(ot.translation.x, ot.translation.z);
        xz = resolve_circle_vs_box(xz, c, ob.half_extents, 0.75);
    }
    t.translation.x = xz.x;
    t.translation.z = xz.y;
}

fn collect_gems(
    mut commands: Commands,
    mut game: ResMut<Game>,
    player: Query<&Transform, With<Player>>,
    gems: Query<(Entity, &Transform), With<Gem>>,
) {
    if game.won {
        return;
    }
    let Ok(pt) = player.single() else { return };
    for (e, gt) in gems.iter() {
        if is_collected(pt.translation, gt.translation) {
            commands.entity(e).despawn();
            game.score += 1;
            if game.score as usize >= GEM_COUNT {
                game.won = true;
            }
        }
    }
}

fn spin_gems(time: Res<Time>, mut gems: Query<(&mut Transform, &Gem)>) {
    let t = time.elapsed_secs();
    for (mut tr, gem) in gems.iter_mut() {
        tr.rotation = Quat::from_rotation_y(t * 2.2 + gem.phase);
        tr.translation.y = gem.base_y + (t * 2.5 + gem.phase).sin() * 0.25;
    }
}

fn follow_camera(
    mouse_btn: Res<ButtonInput<MouseButton>>,
    mouse_motion: Res<AccumulatedMouseMotion>,
    mouse_scroll: Res<AccumulatedMouseScroll>,
    player: Query<&Transform, (With<Player>, Without<FollowCamera>)>,
    mut camera: Query<(&mut Transform, &mut FollowCamera), (Without<Player>, Without<Obstacle>)>,
) {
    let Ok(pt) = player.single() else { return };
    let Ok((mut ct, mut follow)) = camera.single_mut() else {
        return;
    };

    if mouse_scroll.delta.y != 0.0 {
        follow.distance = (follow.distance - mouse_scroll.delta.y * 1.2).clamp(6.0, 24.0);
    }
    if mouse_btn.pressed(MouseButton::Right) && mouse_motion.delta != Vec2::ZERO {
        follow.yaw -= mouse_motion.delta.x * 0.008;
        follow.pitch = (follow.pitch + mouse_motion.delta.y * 0.006).clamp(0.12, 1.35);
    }

    let (sy, cy) = follow.yaw.sin_cos();
    let (sp, cp) = follow.pitch.sin_cos();
    let offset = Vec3::new(sy * cp, sp, cy * cp) * follow.distance;
    let target = pt.translation + Vec3::Y * 1.2;
    ct.translation = target + offset;
    ct.look_at(target, Vec3::Y);
}

fn update_ui(
    game: Res<Game>,
    mut score_q: Query<&mut Text, With<ScoreText>>,
    mut msg_q: Query<(&mut Visibility, &mut Text), (With<MessageText>, Without<ScoreText>)>,
) {
    if game.is_changed() {
        for mut text in score_q.iter_mut() {
            *text = Text::new(format!("Gems: {}/{}", game.score, GEM_COUNT));
        }
        for (mut vis, mut text) in msg_q.iter_mut() {
            if game.won {
                *vis = Visibility::Visible;
                *text = Text::new(format!(
                    "You collected all {} gems in {:.1}s!  Press R to play again.",
                    GEM_COUNT, game.elapsed
                ));
            } else {
                *vis = Visibility::Hidden;
            }
        }
    }
}

fn restart_game(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mut game: ResMut<Game>,
    mut player: Query<(&mut Transform, &mut Player)>,
    gems: Query<Entity, With<Gem>>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // Track elapsed play time.
    if !game.won {
        game.elapsed += time.delta_secs();
    }

    if !keys.just_pressed(KeyCode::KeyR) {
        return;
    }

    game.score = 0;
    game.elapsed = 0.0;
    game.won = false;

    if let Ok((mut t, mut p)) = player.single_mut() {
        t.translation = Vec3::new(0.0, 0.45, 8.0);
        t.rotation = Quat::IDENTITY;
        p.vel_y = 0.0;
    }

    // Remove leftover gems and scatter fresh ones.
    for e in gems.iter() {
        commands.entity(e).despawn();
    }
    let gem_mesh = meshes.add(Sphere::new(0.42));
    let gem_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(1.0, 0.78, 0.15),
        unlit: true,
        ..default()
    });
    for i in 0..GEM_COUNT {
        let p = scatter_pos(i, 7);
        commands.spawn((
            Mesh3d(gem_mesh.clone()),
            MeshMaterial3d(gem_mat.clone()),
            Transform::from_translation(p),
            Gem {
                base_y: 1.0,
                phase: i as f32 * 0.7,
            },
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arena_clamp_keeps_player_inside() {
        let p = clamp_to_arena(Vec3::new(100.0, 0.45, -100.0));
        assert_eq!(p.x, ARENA_HALF);
        assert_eq!(p.z, -ARENA_HALF);
        let q = clamp_to_arena(Vec3::new(3.0, 0.45, 4.0));
        assert_eq!(q, Vec3::new(3.0, 0.45, 4.0));
    }

    #[test]
    fn collection_radius_works() {
        assert!(is_collected(Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0)));
        assert!(!is_collected(Vec3::ZERO, Vec3::new(5.0, 0.0, 0.0)));
        assert!(!is_collected(
            Vec3::ZERO,
            Vec3::new(0.0, 0.0, COLLECT_RADIUS + 0.01)
        ));
    }

    #[test]
    fn circle_box_resolution_pushes_out() {
        let out = resolve_circle_vs_box(
            Vec2::new(1.0, 0.0),
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 1.0),
            0.5,
        );
        assert!(out.x >= 1.5 - 1e-5, "got {out:?}");
        let far = resolve_circle_vs_box(
            Vec2::new(5.0, 5.0),
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 1.0),
            0.5,
        );
        assert_eq!(far, Vec2::new(5.0, 5.0));
    }

    #[test]
    fn scatter_stays_in_arena_and_varies() {
        let a = scatter_pos(0, 1);
        let b = scatter_pos(1, 1);
        for p in [a, b] {
            assert!(p.x.abs() <= ARENA_HALF);
            assert!(p.z.abs() <= ARENA_HALF);
        }
        assert_ne!(a, b);
        // Same inputs => same outputs (deterministic).
        assert_eq!(a, scatter_pos(0, 1));
    }
}
