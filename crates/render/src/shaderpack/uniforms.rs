//! The standard Iris uniforms, computed once per view and frame from the extracted view and
//! [`ShaderPackFrameInputs`], and packed into each program's std140 uniform block by the
//! reflected member names and offsets.
//!
//! Matrices follow OpenGL conventions (clip depth `[-1, 1]`, right-handed view space looking
//! down -Z) because pack code reconstructs positions with them; the translator converts the
//! final clip position to wgpu's. As in Minecraft, `gbufferModelView` holds only the camera
//! rotation and positions are relative to `cameraPosition`.

use assets::facts::java_mapping::JavaBlockMapping;
use bevy::math::{Mat3, Mat4, Quat, Vec3, Vec4};
use render_scene::shaderpack::ShaderPackFrameInputs;
use shaderpack::{BiomeUniforms, UniformBlock, UniformValue, ValueType};
use std::collections::BTreeSet;

/// Near plane of every pack projection, as in Minecraft.
pub(crate) const GL_NEAR: f32 = 0.05;
/// Minecraft's projection far plane is four times the render distance (`far`).
pub(crate) const FAR_PLANE_SCALE: f32 = 4.0;
/// `frameCounter` wraps here, as in Iris.
const FRAME_COUNTER_PERIOD: u64 = 720_720;
/// `frameTimeCounter` wraps after an hour, as in Iris.
const FRAME_TIME_COUNTER_PERIOD: f64 = 3600.0;
/// Seconds per game tick, for the smoothing half-lives written in ticks.
const SECONDS_PER_TICK: f32 = 0.05;
/// `GL_LINEAR`, the fog mode packs see for vanilla's linear fog.
const GL_LINEAR: i32 = 0x2601;
/// Height of Bedrock's cloud layer.
const CLOUD_HEIGHT: f32 = 192.0;

/// A uniform value of any type the standard set uses.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum HostValue {
    Float(f32),
    Int(i32),
    Vec(Vec4, u8),
    IVec([i32; 4], u8),
    Mat3(Mat3),
    Mat4(Mat4),
}

impl HostValue {
    fn vec3(v: Vec3) -> Self {
        HostValue::Vec(v.extend(0.0), 3)
    }

    fn bool(b: bool) -> Self {
        HostValue::Int(i32::from(b))
    }

    /// Element `(column, row)` as f64; vectors are one column, scalars one element.
    fn element(&self, column: usize, row: usize) -> Option<f64> {
        match *self {
            HostValue::Float(v) => (column == 0 && row == 0).then_some(f64::from(v)),
            HostValue::Int(v) => (column == 0 && row == 0).then_some(f64::from(v)),
            HostValue::Vec(v, n) => {
                (column == 0 && row < usize::from(n)).then(|| f64::from(v[row]))
            }
            HostValue::IVec(v, n) => {
                (column == 0 && row < usize::from(n)).then(|| f64::from(v[row]))
            }
            HostValue::Mat3(m) => (column < 3 && row < 3).then(|| f64::from(m.col(column)[row])),
            HostValue::Mat4(m) => (column < 4 && row < 4).then(|| f64::from(m.col(column)[row])),
        }
    }

    /// The value custom uniform expressions see; matrices are not visible to them.
    pub(crate) fn to_custom(self) -> Option<UniformValue> {
        Some(match self {
            HostValue::Float(v) => UniformValue::Float(v),
            HostValue::Int(v) => UniformValue::Int(v),
            HostValue::Vec(v, 2) => UniformValue::Vec2([v.x, v.y]),
            HostValue::Vec(v, 3) => UniformValue::Vec3([v.x, v.y, v.z]),
            HostValue::Vec(v, _) => UniformValue::Vec4(v.to_array()),
            HostValue::IVec(v, 2) => UniformValue::Vec2([v[0] as f32, v[1] as f32]),
            HostValue::IVec(v, 3) => UniformValue::Vec3([v[0] as f32, v[1] as f32, v[2] as f32]),
            HostValue::IVec(v, _) => UniformValue::Vec4(v.map(|c| c as f32)),
            HostValue::Mat3(_) | HostValue::Mat4(_) => return None,
        })
    }

    pub(crate) fn from_custom(value: UniformValue) -> Self {
        match value {
            UniformValue::Bool(b) => HostValue::bool(b),
            UniformValue::Int(v) => HostValue::Int(v),
            UniformValue::Float(v) => HostValue::Float(v),
            UniformValue::Vec2([x, y]) => HostValue::Vec(Vec4::new(x, y, 0.0, 0.0), 2),
            UniformValue::Vec3([x, y, z]) => HostValue::Vec(Vec4::new(x, y, z, 0.0), 3),
            UniformValue::Vec4(v) => HostValue::Vec(Vec4::from_array(v), 4),
        }
    }
}

/// An OpenGL perspective projection with the given focal lengths and planes.
pub(crate) fn gl_perspective(focal: [f32; 2], near: f32, far: f32) -> Mat4 {
    Mat4::from_cols(
        Vec4::new(focal[0], 0.0, 0.0, 0.0),
        Vec4::new(0.0, focal[1], 0.0, 0.0),
        Vec4::new(0.0, 0.0, -(far + near) / (far - near), -1.0),
        Vec4::new(0.0, 0.0, -2.0 * far * near / (far - near), 0.0),
    )
}

/// An OpenGL orthographic projection (`glOrtho`).
pub(crate) fn gl_ortho(left: f32, right: f32, bottom: f32, top: f32, near: f32, far: f32) -> Mat4 {
    Mat4::from_cols(
        Vec4::new(2.0 / (right - left), 0.0, 0.0, 0.0),
        Vec4::new(0.0, 2.0 / (top - bottom), 0.0, 0.0),
        Vec4::new(0.0, 0.0, -2.0 / (far - near), 0.0),
        Vec4::new(
            -(right + left) / (right - left),
            -(top + bottom) / (top - bottom),
            -(far + near) / (far - near),
            1.0,
        ),
    )
}

/// Pack constants and `shaders.properties` settings the uniforms depend on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PackSettings {
    /// `const float sunPathRotation`, degrees.
    pub sun_path_rotation: f32,
    /// `const float shadowDistance`: half the shadow projection's width, blocks.
    pub shadow_distance: f32,
    /// `const float shadowIntervalSize`: shadow camera snapping, blocks.
    pub shadow_interval: f32,
    /// `wetnessHalflife`, `drynessHalflife` and `eyeBrightnessHalflife`, seconds.
    pub wetness_halflife: f32,
    pub dryness_halflife: f32,
    pub eye_brightness_halflife: f32,
}

impl Default for PackSettings {
    fn default() -> Self {
        Self {
            sun_path_rotation: 0.0,
            shadow_distance: 160.0,
            shadow_interval: 2.0,
            wetness_halflife: 600.0 * SECONDS_PER_TICK,
            dryness_halflife: 200.0 * SECONDS_PER_TICK,
            eye_brightness_halflife: 10.0 * SECONDS_PER_TICK,
        }
    }
}

impl PackSettings {
    /// Reads the settings from active `const` directives and `shaders.properties` entries.
    pub(crate) fn read<'a>(
        consts: impl Iterator<Item = &'a shaderpack::ConstDirective>,
        properties: &shaderpack::ShaderProperties,
    ) -> Self {
        let mut s = Self::default();
        let number = |v: &str| v.trim().trim_end_matches(['f', 'F']).parse::<f32>().ok();
        for c in consts {
            let slot = match c.name.as_str() {
                "sunPathRotation" => &mut s.sun_path_rotation,
                "shadowDistance" => &mut s.shadow_distance,
                "shadowIntervalSize" => &mut s.shadow_interval,
                _ => continue,
            };
            if let Some(v) = number(&c.value) {
                *slot = v;
            }
        }
        for (key, slot) in [
            ("wetnessHalflife", &mut s.wetness_halflife),
            ("drynessHalflife", &mut s.dryness_halflife),
            ("eyeBrightnessHalflife", &mut s.eye_brightness_halflife),
        ] {
            if let Some(v) = properties.get(key).and_then(number) {
                *slot = v * SECONDS_PER_TICK;
            }
        }
        s
    }
}

/// The camera of one view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ViewCamera {
    pub position: Vec3,
    pub rotation: Quat,
    /// Horizontal and vertical focal lengths of the view's projection.
    pub focal: [f32; 2],
    pub screen: [u32; 2],
}

/// Values that persist from frame to frame for one view.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct UniformHistory {
    pub previous: Option<(Mat4, Mat4, Vec3)>,
    pub wetness: f32,
    pub eye_brightness: [f32; 2],
    pub frame: u64,
    pub started: bool,
}

/// Frame timing from the render-world clock.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FrameClock {
    pub elapsed_seconds: f64,
    pub delta_seconds: f32,
}

/// Exponential approach with a half-life, as OptiFine smooths wetness and eye brightness.
fn smooth(current: f32, target: f32, delta: f32, halflife: f32) -> f32 {
    if halflife <= 0.0 {
        return target;
    }
    current + (target - current) * (1.0 - 0.5f32.powf(delta / halflife))
}

/// The standard uniforms of one view for one frame.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FrameUniforms {
    camera: ViewCamera,
    game: ShaderPackFrameInputs,
    model_view: Mat4,
    projection: Mat4,
    previous: (Mat4, Mat4, Vec3),
    shadow_model_view: Mat4,
    shadow_projection: Mat4,
    sun: Vec3,
    moon: Vec3,
    up: Vec3,
    sun_angle: f32,
    shadow_angle: f32,
    wetness: f32,
    eye_brightness_smooth: [i32; 2],
    frame_counter: i32,
    frame_time: f32,
    frame_time_counter: f32,
    biome: BiomeUniforms,
}

impl FrameUniforms {
    /// Computes this frame's values and advances `history`.
    pub(crate) fn new(
        camera: ViewCamera,
        game: &ShaderPackFrameInputs,
        settings: &PackSettings,
        clock: FrameClock,
        history: &mut UniformHistory,
    ) -> Self {
        let far = game.render_distance.max(16.0);
        let model_view = Mat4::from_quat(camera.rotation.inverse());
        let projection = gl_perspective(camera.focal, GL_NEAR, far * FAR_PLANE_SCALE);
        let sky_angle = game.celestial_angle;
        let sun_angle = if sky_angle < 0.75 {
            sky_angle + 0.25
        } else {
            sky_angle - 0.75
        };
        let shadow_angle = if sun_angle > 0.5 {
            sun_angle - 0.5
        } else {
            sun_angle
        };
        let celestial = model_view
            * Mat4::from_rotation_y((-90.0f32).to_radians())
            * Mat4::from_rotation_z(settings.sun_path_rotation.to_radians())
            * Mat4::from_rotation_x((sky_angle * 360.0).to_radians());
        let sun = celestial.transform_vector3(Vec3::new(0.0, 100.0, 0.0));
        let moon = -sun;
        let up = model_view.transform_vector3(Vec3::new(0.0, 100.0, 0.0));
        let interval = settings.shadow_interval;
        let snap = |v: f32| {
            if interval <= 0.0 {
                return 0.0;
            }
            let half = interval * 0.5;
            let offset = v % interval;
            if offset > half {
                offset - interval
            } else if offset < -half {
                offset + interval
            } else {
                offset
            }
        };
        // Iris's shadow camera: 100 blocks back, looking down, turned with the shadow angle
        // so that the direction to the shadow light (`shadowLightPosition`) is its +Z.
        // `shadowAngle` is 0.25 at noon, a quarter turn ahead of the sky angle.
        let shadow_model_view = Mat4::from_translation(Vec3::new(0.0, 0.0, -100.0))
            * Mat4::from_rotation_x(90.0f32.to_radians())
            * Mat4::from_rotation_z(((shadow_angle - 0.25) * -360.0).to_radians())
            * Mat4::from_rotation_x(settings.sun_path_rotation.to_radians())
            * Mat4::from_translation(Vec3::new(
                snap(camera.position.x),
                snap(camera.position.y),
                snap(camera.position.z),
            ));
        let d = settings.shadow_distance;
        let shadow_projection = gl_ortho(-d, d, -d, d, GL_NEAR, 256.0);
        let delta = clock.delta_seconds.max(0.0);
        let rain = game.rain_strength;
        let brightness = game.eye_brightness.map(|v| v as f32);
        if history.started {
            let halflife = if rain > history.wetness {
                settings.wetness_halflife
            } else {
                settings.dryness_halflife
            };
            history.wetness = smooth(history.wetness, rain, delta, halflife);
            for (smoothed, target) in history.eye_brightness.iter_mut().zip(brightness) {
                *smoothed = smooth(*smoothed, target, delta, settings.eye_brightness_halflife);
            }
        } else {
            history.wetness = rain;
            history.eye_brightness = brightness;
            history.started = true;
        }
        let previous = history
            .previous
            .unwrap_or((model_view, projection, camera.position));
        history.previous = Some((model_view, projection, camera.position));
        let frame_counter = (history.frame % FRAME_COUNTER_PERIOD) as i32;
        history.frame = history.frame.wrapping_add(1);
        Self {
            camera,
            game: *game,
            model_view,
            projection,
            previous,
            shadow_model_view,
            shadow_projection,
            sun,
            moon,
            up,
            sun_angle,
            shadow_angle,
            wetness: history.wetness,
            eye_brightness_smooth: history.eye_brightness.map(|v| v.round() as i32),
            frame_counter,
            frame_time: delta,
            frame_time_counter: (clock.elapsed_seconds % FRAME_TIME_COUNTER_PERIOD) as f32,
            biome: game
                .biome
                .and_then(|biome| {
                    let mapping = JavaBlockMapping::pinned().ok()?;
                    BiomeUniforms::at(mapping, biome, game.biome_block_y)
                })
                .unwrap_or(BiomeUniforms::NONE),
        }
    }

    /// `gbufferModelView`: the camera rotation.
    pub(crate) fn model_view(&self) -> Mat4 {
        self.model_view
    }

    /// `gbufferProjection`.
    pub(crate) fn projection(&self) -> Mat4 {
        self.projection
    }

    /// `shadowModelView`.
    pub(crate) fn shadow_model_view(&self) -> Mat4 {
        self.shadow_model_view
    }

    /// `shadowProjection`.
    pub(crate) fn shadow_projection(&self) -> Mat4 {
        self.shadow_projection
    }

    /// The value of a standard uniform, or `None` for names outside the standard set.
    /// Legacy and core-profile matrices take their full-screen pass values (identity
    /// model-view, unit-square orthographic projection); geometry draws override them with
    /// [`DrawUniforms`].
    pub(crate) fn value(&self, name: &str) -> Option<HostValue> {
        use HostValue::{Float, Int, Mat3 as M3, Mat4 as M4};
        let g = &self.game;
        let [w, h] = self.camera.screen.map(|v| v as f32);
        let position = self.camera.position;
        let previous_position = self.previous.2;
        let ivec3 = |v: Vec3| {
            let f = v.floor();
            HostValue::IVec([f.x as i32, f.y as i32, f.z as i32, 0], 3)
        };
        let fract = |v: Vec3| HostValue::vec3(v - v.floor());
        let fog_density = 1.0 / (g.fog_range[1] - g.fog_range[0]).max(1e-3);
        Some(match name {
            "gbufferModelView" => M4(self.model_view),
            "gbufferModelViewInverse" => M4(self.model_view.inverse()),
            "gbufferProjection" => M4(self.projection),
            "gbufferProjectionInverse" => M4(self.projection.inverse()),
            "gbufferPreviousModelView" => M4(self.previous.0),
            "gbufferPreviousProjection" => M4(self.previous.1),
            "shadowModelView" => M4(self.shadow_model_view),
            "shadowModelViewInverse" => M4(self.shadow_model_view.inverse()),
            "shadowProjection" => M4(self.shadow_projection),
            "shadowProjectionInverse" => M4(self.shadow_projection.inverse()),
            "iris_ModelViewMat"
            | "iris_ModelViewMatInverse"
            | "modelViewMatrix"
            | "modelViewMatrixInverse"
            | "iris_TextureMat"
            | "textureMatrix" => M4(Mat4::IDENTITY),
            "iris_ProjMat" | "projectionMatrix" => M4(gl_ortho(0.0, 1.0, 0.0, 1.0, -1.0, 1.0)),
            "iris_ProjMatInverse" | "projectionMatrixInverse" => {
                M4(gl_ortho(0.0, 1.0, 0.0, 1.0, -1.0, 1.0).inverse())
            }
            "iris_NormalMat" | "normalMatrix" => M3(Mat3::IDENTITY),
            "chunkOffset" => HostValue::vec3(Vec3::ZERO),
            "renderStage" => Int(0),
            "atlasSize" => HostValue::IVec([0; 4], 2),
            "iris_Fog.color" => HostValue::Vec(Vec3::from(g.fog_color).extend(1.0), 4),
            "iris_Fog.density" | "fogDensity" => Float(fog_density),
            "iris_Fog.start" | "fogStart" => Float(g.fog_range[0]),
            "iris_Fog.end" | "fogEnd" => Float(g.fog_range[1]),
            "iris_Fog.scale" => Float(fog_density),
            "fogMode" => Int(GL_LINEAR),
            "fogShape" => Int(0),
            "cameraPosition" | "eyePosition" => HostValue::vec3(position),
            "cameraPositionInt" => ivec3(position),
            "cameraPositionFract" => fract(position),
            "previousCameraPosition" => HostValue::vec3(previous_position),
            "previousCameraPositionInt" => ivec3(previous_position),
            "previousCameraPositionFract" => fract(previous_position),
            "eyeAltitude" => Float(position.y),
            "sunPosition" => HostValue::vec3(self.sun),
            "moonPosition" => HostValue::vec3(self.moon),
            "shadowLightPosition" => HostValue::vec3(if self.sun_angle <= 0.5 {
                self.sun
            } else {
                self.moon
            }),
            "upPosition" => HostValue::vec3(self.up),
            "sunAngle" => Float(self.sun_angle),
            "shadowAngle" => Float(self.shadow_angle),
            "worldTime" => Int(g.world_time),
            "worldDay" => Int(g.world_day),
            "moonPhase" => Int(g.moon_phase),
            "frameCounter" => Int(self.frame_counter),
            "frameTime" => Float(self.frame_time),
            "frameTimeCounter" => Float(self.frame_time_counter),
            "viewWidth" => Float(w),
            "viewHeight" => Float(h),
            "aspectRatio" => Float(w / h.max(1.0)),
            "near" => Float(GL_NEAR),
            "far" => Float(g.render_distance),
            "rainStrength" => Float(g.rain_strength),
            "thunderStrength" => Float(g.thunder_strength),
            "wetness" => Float(self.wetness),
            "eyeBrightness" => HostValue::IVec([g.eye_brightness[0], g.eye_brightness[1], 0, 0], 2),
            "eyeBrightnessSmooth" => HostValue::IVec(
                [
                    self.eye_brightness_smooth[0],
                    self.eye_brightness_smooth[1],
                    0,
                    0,
                ],
                2,
            ),
            "isEyeInWater" => Int(g.eye_medium),
            "skyColor" => HostValue::vec3(Vec3::from(g.sky_color)),
            "fogColor" => HostValue::vec3(Vec3::from(g.fog_color)),
            "nightVision" => Float(g.night_vision),
            "blindness" => Float(g.blindness),
            "darknessFactor" => Float(g.darkness),
            "darknessLightFactor" => Float(0.0),
            "heldItemId" => Int(g.held_item_id),
            "heldBlockLightValue" => Int(g.held_block_light),
            "heldItemId2" => Int(g.held_item_id2),
            "heldBlockLightValue2" => Int(g.held_block_light2),
            "currentPlayerHealth" => Float(g.player_health),
            "currentPlayerHunger" => Float(g.player_hunger),
            "currentPlayerAir" => Float(g.player_air),
            "maxPlayerHealth" => Float(20.0),
            "maxPlayerHunger" => Float(20.0),
            "maxPlayerAir" => Float(300.0),
            "biome" => Int(self.biome.biome),
            "biome_category" => Int(self.biome.category),
            "biome_precipitation" => Int(self.biome.precipitation),
            "temperature" => Float(self.biome.temperature),
            "rainfall" => Float(self.biome.rainfall),
            "relativeEyePosition" => HostValue::vec3(Vec3::ZERO),
            "lightningBoltPosition" => HostValue::Vec(Vec4::ZERO, 4),
            "is_sneaking" => HostValue::bool(g.sneaking),
            "is_sprinting" => HostValue::bool(g.sprinting),
            "is_hurt" => HostValue::bool(g.hurt),
            "is_burning" => HostValue::bool(g.on_fire),
            "is_on_ground" => HostValue::bool(g.on_ground),
            "is_invisible" => HostValue::bool(false),
            "isRightHanded" => HostValue::bool(true),
            "hideGUI" => Int(0),
            "screenBrightness" => Float(0.5),
            "hasSkylight" => HostValue::bool(g.dimension == 0),
            "hasCeiling" => HostValue::bool(g.dimension == 1),
            "ambientLight" => Float(if g.dimension == 1 { 0.1 } else { 0.0 }),
            "cloudHeight" => Float(CLOUD_HEIGHT),
            _ => return None,
        })
    }
}

/// The fixed-function state Iris sets per geometry draw: its model-view and projection
/// matrices, the terrain origin relative to the camera, the render stage and the entity id.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct DrawUniforms {
    /// Model-view without the chunk offset (`gbufferModelView`, or `shadowModelView` in the
    /// shadow pass): the core profile's `modelViewMatrix`.
    pub model_view: Mat4,
    pub projection: Mat4,
    /// `chunkOffset`: where vertex positions are relative to, from the camera.
    pub chunk_offset: Vec3,
    /// `renderStage`.
    pub render_stage: i32,
    /// Lightmap coordinates are raw `light × 16` values through `gl_TextureMatrix[1]`.
    pub lightmap_matrix: bool,
    /// `atlasSize`: the bound atlas's size, zero for draws without one.
    pub atlas_size: [i32; 2],
    /// `entityId`: the drawn entity's `entity.properties` id, Iris's -1 outside entities.
    pub entity_id: i32,
}

impl DrawUniforms {
    /// The value this draw gives `name`, or `None` to use the frame's.
    ///
    /// The legacy `gl_ModelViewMatrix` folds in the chunk offset so `gl_ModelViewMatrix *
    /// gl_Vertex` places origin-relative vertices; the core `modelViewMatrix` does not, as
    /// core programs add `chunkOffset` to `vaPosition` themselves.
    pub(crate) fn value(&self, name: &str) -> Option<HostValue> {
        use HostValue::{Mat3 as M3, Mat4 as M4};
        let legacy = self.model_view * Mat4::from_translation(self.chunk_offset);
        let normal = Mat3::from_mat4(self.model_view).inverse().transpose();
        Some(match name {
            "iris_ModelViewMat" => M4(legacy),
            "iris_ModelViewMatInverse" => M4(legacy.inverse()),
            "modelViewMatrix" => M4(self.model_view),
            "modelViewMatrixInverse" => M4(self.model_view.inverse()),
            "iris_ProjMat" | "projectionMatrix" => M4(self.projection),
            "iris_ProjMatInverse" | "projectionMatrixInverse" => M4(self.projection.inverse()),
            "iris_NormalMat" | "normalMatrix" => M3(normal),
            "iris_TextureMat[1]" if self.lightmap_matrix => {
                M4(super::geometry::LIGHTMAP_TEXTURE_MATRIX)
            }
            "chunkOffset" => HostValue::vec3(self.chunk_offset),
            "renderStage" => HostValue::Int(self.render_stage),
            "atlasSize" => HostValue::IVec([self.atlas_size[0], self.atlas_size[1], 0, 0], 2),
            "entityId" => HostValue::Int(self.entity_id),
            _ => return None,
        })
    }
}

/// Packs `block` from `lookup` into `out` (resized to the block size). Members whose name
/// `lookup` does not know are zero-filled and added to `unknown`.
pub(crate) fn pack_block(
    block: &UniformBlock,
    lookup: &dyn Fn(&str) -> Option<HostValue>,
    out: &mut Vec<u8>,
    unknown: &mut BTreeSet<String>,
) {
    out.clear();
    out.resize(block.size as usize, 0);
    for member in &block.members {
        let (count, stride) = member.array.unwrap_or((1, 0));
        for element in 0..count {
            let value = if member.array.is_some() {
                lookup(&format!("{}[{element}]", member.name)).or_else(|| lookup(&member.name))
            } else {
                lookup(&member.name)
            };
            let Some(value) = value else {
                unknown.insert(member.name.clone());
                break;
            };
            let offset = member.offset as usize + (element * stride) as usize;
            write_value(out, offset, member.ty, &value);
        }
    }
}

/// Writes `value` converted to `ty` at `offset` with std140 matrix column strides.
fn write_value(out: &mut [u8], offset: usize, ty: ValueType, value: &HostValue) {
    let (columns, rows, scalar) = match ty {
        ValueType::Scalar(s) => (1, 1, s),
        ValueType::Vector { size, scalar } => (1, usize::from(size), scalar),
        ValueType::Matrix {
            columns,
            rows,
            scalar,
        } => (usize::from(columns), usize::from(rows), scalar),
    };
    for column in 0..columns {
        for row in 0..rows {
            let Some(v) = value.element(column, row) else {
                continue;
            };
            let at = offset + column * 16 + row * 4;
            let bytes = match scalar {
                shaderpack::ScalarType::Float | shaderpack::ScalarType::Double => {
                    (v as f32).to_le_bytes()
                }
                shaderpack::ScalarType::Int | shaderpack::ScalarType::Bool => {
                    (v as i32).to_le_bytes()
                }
                shaderpack::ScalarType::Uint => (v.max(0.0) as u32).to_le_bytes(),
            };
            if let Some(slot) = out.get_mut(at..at + 4) {
                slot.copy_from_slice(&bytes);
            }
        }
    }
}

#[cfg(test)]
mod tests;
