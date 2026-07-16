//! Hand-written scene file parser and serializer — zero dependencies.
//!
//! # Format (.hsc)
//!
//! ```text
//! scene "My Level"
//!
//! entity "player" {
//!   transform { pos 0 1 0  rot 0 0 0 1  scale 1 1 1 }
//!   camera { fov 60  near 0.1  far 1000 }
//!   active_camera
//!   stats { health 100  mana 50  attack 10  defense 5  speed 3 }
//! }
//!
//! entity "sun" {
//!   light { dir 0.5 -1 0.3  color 1 1 1 1  intensity 1  ambient 0.15 }
//! }
//!
//! entity "crate" {
//!   transform { pos 5 0 -3 }
//!   parent "player"
//! }
//! ```
//!
//! Supports: transform, velocity, rigid_body, sphere_collider, particle_emitter,
//! camera, active_camera, light, stats, parent.
//! Comments start with `#`.  Quoted strings use `"double quotes"`.

use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::path::Path;

use crate::platform::atomic_write::{atomic_write, AtomicWriteError};

// ── Scene format version + truncation sentinel (M11) ─────────────────────────
//
// A `.hsc` scene is a bag of top-level blocks with no length prefix and no
// terminator, so `parse_scene` treated EOF as success: a file truncated on an
// entity boundary parsed as a valid *smaller* scene and the trailing entities
// vanished silently. That is the worst kind of data loss — it looks like it
// worked.
//
// Fix: a version header at the top, and a sentinel at the bottom. A file that
// declares the header MUST close with the sentinel, so a cut-short file is a
// typed error instead of a quiet truncation.
//
// Deliberately NOT a checksum. The charter requires scene files to stay
// human-readable and hand-editable; a CRC would be invalidated by every manual
// edit. A sentinel is something a person can type. Atomic writes (M11.3)
// already make a torn *destination* impossible — this catches the cases atomic
// write cannot: interrupted file copies, bit rot, pre-M11 torn files, and
// third-party writers.
//
// Back-compat: the header is OPTIONAL on read. Files without it (every scene
// authored before M11) parse leniently, exactly as before. The engine always
// writes the modern form.

/// First-token marker declaring the scene-format version. Presence switches
/// the parser into strict mode.
pub const SCENE_FORMAT_HEADER: &str = "hold_scene";

/// The scene format version this engine reads and writes. Version 1 was the
/// implicit, unmarked, unterminated format; version 2 adds the header and the
/// [`SCENE_END_SENTINEL`].
pub const SCENE_FORMAT_VERSION: u32 = 2;

/// Terminator that must close a scene declaring [`SCENE_FORMAT_HEADER`].
/// Its absence means the file was cut short.
pub const SCENE_END_SENTINEL: &str = "end_scene";

use crate::ecs::components::{
    ActiveCamera, Camera, DirectionalLight, GlobalTransform, ParticleEmitter, RigidBody,
    SphereCollider, Stats, Transform, Velocity,
};
use crate::ecs::Entity;
use crate::ecs::World;
use crate::math::{Quat, Vec3};
use crate::renderer::Color;
use crate::runtime_slice::{
    BreakablePropDef, PatrolRouteDef, RuntimeSliceAuthoring, SpawnPointDef, SpawnPointKind,
    StaticMeshActorDef, TerrainAuthoring, TerrainPaintLayer, VegetationPaintStroke,
    VegetationPrototypeDef, VehicleSpawnerDef, WaterVolumeDef,
};

// ═══════════════════════════════════════════════════════════════════════════════
//  Error
// ═══════════════════════════════════════════════════════════════════════════════

#[derive(Debug)]
pub struct SceneError {
    pub line: usize,
    pub msg: String,
}

impl SceneError {
    fn new(line: usize, msg: impl Into<String>) -> Self {
        Self {
            line,
            msg: msg.into(),
        }
    }
}

impl fmt::Display for SceneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "scene error (line {}): {}", self.line, self.msg)
    }
}

impl std::error::Error for SceneError {}

// ═══════════════════════════════════════════════════════════════════════════════
//  Scene data model (intermediate representation)
// ═══════════════════════════════════════════════════════════════════════════════

/// A parsed scene ready to be spawned into a [`World`].
pub struct SceneData {
    pub name: String,
    pub runtime_slice: RuntimeSliceAuthoring,
    pub entities: Vec<EntityDef>,
}

/// One entity definition from the scene file.
pub struct EntityDef {
    pub name: String,
    pub transform: Option<TransformDef>,
    pub velocity: Option<VelocityDef>,
    pub rigid_body: Option<RigidBodyDef>,
    pub sphere_collider: Option<SphereColliderDef>,
    pub particle_emitter: Option<ParticleEmitterDef>,
    pub camera: Option<CameraDef>,
    pub active_camera: bool,
    pub light: Option<LightDef>,
    pub stats: Option<StatsDef>,
    pub parent_name: Option<String>,
}

pub struct TransformDef {
    pub position: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

pub struct VelocityDef {
    pub linear: Vec3,
    pub angular: Vec3,
}

pub struct RigidBodyDef {
    pub gravity_scale: f32,
    pub linear_damping: f32,
    pub angular_damping: f32,
    pub restitution: f32,
}

pub struct SphereColliderDef {
    pub radius: f32,
}

pub struct ParticleEmitterDef {
    pub emitting: bool,
    pub rate_per_second: f32,
    pub max_particles: u32,
    pub spawn_radius: f32,
    pub direction: Vec3,
    pub spread: f32,
    pub speed_min: f32,
    pub speed_max: f32,
    pub lifetime_min: f32,
    pub lifetime_max: f32,
    pub gravity_scale: f32,
    pub linear_damping: f32,
    pub start_size: f32,
    pub end_size: f32,
    pub start_color: Color,
    pub end_color: Color,
    pub seed: u32,
}

pub struct CameraDef {
    pub fov: f32,
    pub near: f32,
    pub far: f32,
}

pub struct LightDef {
    pub direction: Vec3,
    pub color: Color,
    pub intensity: f32,
    pub ambient: f32,
}

pub struct StatsDef {
    pub health: f32,
    pub mana: f32,
    pub attack: f32,
    pub defense: f32,
    pub speed: f32,
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Tokenizer
// ═══════════════════════════════════════════════════════════════════════════════

#[derive(Debug, PartialEq)]
enum Token<'a> {
    Word(&'a str),
    Str(&'a str), // contents without quotes
    /// A numeric literal written WITH a decimal point (e.g. `1.5`, `-0.25`).
    /// Float fields (positions, colours, fov, …) read these.
    Number(f32),
    /// A numeric literal written WITHOUT a decimal point (e.g. `128`,
    /// `3000000000`). Lexed as `i64` so integer fields keep full precision:
    /// an f32 only represents integers exactly up to 2^24, so a u32 seed
    /// parsed through f32 would not round-trip. See `expect_uint`.
    Integer(i64),
    LBrace,
    RBrace,
}

struct Lexer<'a> {
    src: &'a str,
    pos: usize,
    line: usize,
}

impl<'a> Lexer<'a> {
    fn new(src: &'a str) -> Self {
        Self {
            src,
            pos: 0,
            line: 1,
        }
    }

    fn skip_ws_and_comments(&mut self) {
        loop {
            // Skip whitespace.
            while self.pos < self.src.len() {
                let c = self.src.as_bytes()[self.pos];
                if c == b' ' || c == b'\t' || c == b'\r' {
                    self.pos += 1;
                } else if c == b'\n' {
                    self.pos += 1;
                    self.line += 1;
                } else {
                    break;
                }
            }
            // Skip # comments to end of line.
            if self.pos < self.src.len() && self.src.as_bytes()[self.pos] == b'#' {
                while self.pos < self.src.len() && self.src.as_bytes()[self.pos] != b'\n' {
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }

    fn next_token(&mut self) -> Result<Option<Token<'a>>, SceneError> {
        self.skip_ws_and_comments();
        if self.pos >= self.src.len() {
            return Ok(None);
        }

        let c = self.src.as_bytes()[self.pos];
        match c {
            b'{' => {
                self.pos += 1;
                Ok(Some(Token::LBrace))
            }
            b'}' => {
                self.pos += 1;
                Ok(Some(Token::RBrace))
            }
            b'"' => self.read_string(),
            _ if c == b'-' || c == b'+' || c == b'.' || c.is_ascii_digit() => {
                // Could be a number or a word starting with a digit.
                self.read_number_or_word()
            }
            _ if c.is_ascii_alphabetic() || c == b'_' => self.read_word(),
            _ => Err(SceneError::new(
                self.line,
                format!("unexpected character '{}'", c as char),
            )),
        }
    }

    fn read_string(&mut self) -> Result<Option<Token<'a>>, SceneError> {
        let start_line = self.line;
        self.pos += 1; // skip opening "
        let start = self.pos;
        while self.pos < self.src.len() && self.src.as_bytes()[self.pos] != b'"' {
            if self.src.as_bytes()[self.pos] == b'\n' {
                self.line += 1;
            }
            self.pos += 1;
        }
        if self.pos >= self.src.len() {
            return Err(SceneError::new(start_line, "unterminated string"));
        }
        let s = &self.src[start..self.pos];
        self.pos += 1; // skip closing "
        Ok(Some(Token::Str(s)))
    }

    fn read_word(&mut self) -> Result<Option<Token<'a>>, SceneError> {
        let start = self.pos;
        while self.pos < self.src.len() {
            let c = self.src.as_bytes()[self.pos];
            if c.is_ascii_alphanumeric() || c == b'_' {
                self.pos += 1;
            } else {
                break;
            }
        }
        Ok(Some(Token::Word(&self.src[start..self.pos])))
    }

    fn read_number_or_word(&mut self) -> Result<Option<Token<'a>>, SceneError> {
        let start = self.pos;
        // Consume sign.
        if self.pos < self.src.len()
            && (self.src.as_bytes()[self.pos] == b'-' || self.src.as_bytes()[self.pos] == b'+')
        {
            self.pos += 1;
        }
        let mut has_digit = false;
        let mut has_dot = false;
        while self.pos < self.src.len() {
            let c = self.src.as_bytes()[self.pos];
            if c.is_ascii_digit() {
                has_digit = true;
                self.pos += 1;
            } else if c == b'.' && !has_dot {
                has_dot = true;
                self.pos += 1;
            } else {
                break;
            }
        }
        let slice = &self.src[start..self.pos];
        if has_digit {
            if has_dot {
                let val: f32 = slice
                    .parse()
                    .map_err(|_| SceneError::new(self.line, format!("invalid number: {slice}")))?;
                Ok(Some(Token::Number(val)))
            } else {
                // No decimal point: an integer literal, parsed with full
                // precision. A value too large for i64 falls back to a float
                // token — it is out of range for any integer field anyway, and
                // this keeps float parsing lenient for astronomically large
                // magnitudes.
                match slice.parse::<i64>() {
                    Ok(n) => Ok(Some(Token::Integer(n))),
                    Err(_) => {
                        let val: f32 = slice.parse().map_err(|_| {
                            SceneError::new(self.line, format!("invalid number: {slice}"))
                        })?;
                        Ok(Some(Token::Number(val)))
                    }
                }
            }
        } else {
            // Not a valid number — treat as word.
            while self.pos < self.src.len() {
                let c = self.src.as_bytes()[self.pos];
                if c.is_ascii_alphanumeric() || c == b'_' {
                    self.pos += 1;
                } else {
                    break;
                }
            }
            Ok(Some(Token::Word(&self.src[start..self.pos])))
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Parser
// ═══════════════════════════════════════════════════════════════════════════════

struct Parser<'a> {
    lex: Lexer<'a>,
    peeked: Option<Token<'a>>,
}

impl<'a> Parser<'a> {
    fn new(src: &'a str) -> Self {
        Self {
            lex: Lexer::new(src),
            peeked: None,
        }
    }

    fn line(&self) -> usize {
        self.lex.line
    }

    fn peek(&mut self) -> Result<Option<&Token<'a>>, SceneError> {
        if self.peeked.is_none() {
            self.peeked = self.lex.next_token()?;
        }
        Ok(self.peeked.as_ref())
    }

    fn next(&mut self) -> Result<Option<Token<'a>>, SceneError> {
        if let Some(tok) = self.peeked.take() {
            Ok(Some(tok))
        } else {
            self.lex.next_token()
        }
    }

    fn expect_str(&mut self) -> Result<String, SceneError> {
        match self.next()? {
            Some(Token::Str(s)) => Ok(s.to_owned()),
            _ => Err(SceneError::new(
                self.line(),
                format!("expected quoted string, got unexpected token"),
            )),
        }
    }

    fn expect_lbrace(&mut self) -> Result<(), SceneError> {
        match self.next()? {
            Some(Token::LBrace) => Ok(()),
            _ => Err(SceneError::new(
                self.line(),
                format!("expected '{{', got unexpected token"),
            )),
        }
    }

    fn expect_number(&mut self) -> Result<f32, SceneError> {
        match self.next()? {
            Some(Token::Number(n)) => Ok(n),
            // An integer literal where a float is expected is fine
            // (`pos 0 1 0`). i64 -> f32 is lossy above 2^24, but a float field
            // does not need exact large-integer precision.
            Some(Token::Integer(n)) => Ok(n as f32),
            _ => Err(SceneError::new(
                self.line(),
                "expected number, got unexpected token".to_string(),
            )),
        }
    }

    /// Read a field that must be a non-negative integer in `[0, max]`.
    ///
    /// Accepts an integer literal (`128`, exact even above 2^24) or an
    /// integer-valued float written with a point (`128.0`), and REJECTS a
    /// fractional value, a negative, or an out-of-range one with a typed,
    /// line-numbered error. This replaces the `expect_number()? as uN` pattern,
    /// which silently truncated a fractional value (`2.5` -> `2`) AND lost
    /// precision on large integers (a `u32` seed parsed through `f32` did not
    /// round-trip) — the latter a determinism hazard (ADR 0005).
    fn expect_uint(&mut self, max: u64) -> Result<u64, SceneError> {
        let line = self.line();
        let value = match self.next()? {
            Some(Token::Integer(n)) => {
                if n < 0 {
                    return Err(SceneError::new(
                        line,
                        format!("expected a non-negative integer, got {n}"),
                    ));
                }
                n as u64
            }
            // A float token only reaches here if the author wrote a decimal
            // point. Accept it only if it is exactly integer-valued.
            Some(Token::Number(f)) => {
                if !f.is_finite() || f < 0.0 || f.fract() != 0.0 || f > u64::MAX as f32 {
                    return Err(SceneError::new(
                        line,
                        format!("expected a non-negative integer, got {f}"),
                    ));
                }
                f as u64
            }
            _ => {
                return Err(SceneError::new(line, "expected an integer".to_string()));
            }
        };
        if value > max {
            return Err(SceneError::new(
                line,
                format!("integer value {value} exceeds the maximum {max} for this field"),
            ));
        }
        Ok(value)
    }

    fn expect_bool_flag(&mut self) -> Result<bool, SceneError> {
        Ok(self.expect_number()? > 0.0)
    }

    fn expect_vec3(&mut self) -> Result<Vec3, SceneError> {
        Ok(Vec3::new(
            self.expect_number()?,
            self.expect_number()?,
            self.expect_number()?,
        ))
    }

    // ── Top-level ─────────────────────────────────────────────────────────

    fn parse_scene(&mut self) -> Result<SceneData, SceneError> {
        let mut scene = SceneData {
            name: String::new(),
            runtime_slice: RuntimeSliceAuthoring::default(),
            entities: Vec::new(),
        };

        // Optional format-version header (M11). Its presence puts the parser
        // in STRICT mode, which requires the trailing sentinel — see
        // SCENE_END_SENTINEL for why truncation would otherwise be silent.
        // Its absence means a pre-M11 file, parsed leniently.
        let mut strict = false;
        if let Some(Token::Word(SCENE_FORMAT_HEADER)) = self.peek()? {
            let line = self.lex.line;
            self.next()?;
            // A version is an exact non-negative integer. expect_uint rejects a
            // fractional literal (a bare `expect_number()? as u32` would
            // truncate `2.5` to `2` and slip a version the engine cannot read
            // through the gate) and reads large integers exactly.
            let version = self.expect_uint(u32::MAX as u64)? as u32;
            if version != SCENE_FORMAT_VERSION {
                return Err(SceneError::new(
                    line,
                    format!(
                        "unsupported scene format version {version}; \
                         this engine reads and writes version {SCENE_FORMAT_VERSION}"
                    ),
                ));
            }
            strict = true;
        }

        // Optional "scene" header.
        if let Some(Token::Word("scene")) = self.peek()? {
            self.next()?; // consume "scene"
            scene.name = self.expect_str()?;
        }

        let mut saw_end = false;
        while self.peek()?.is_some() {
            let line = self.lex.line;
            match self.peek()? {
                // Accepted in both modes; only *required* in strict mode, so a
                // hand-written legacy file may add it without a version header.
                Some(Token::Word(SCENE_END_SENTINEL)) => {
                    self.next()?;
                    if self.peek()?.is_some() {
                        return Err(SceneError::new(
                            line,
                            format!("content after '{SCENE_END_SENTINEL}' terminator"),
                        ));
                    }
                    saw_end = true;
                    break;
                }
                Some(Token::Word("runtime_slice")) => {
                    self.next()?;
                    if !scene.runtime_slice.is_empty() {
                        return Err(SceneError::new(line, "duplicate runtime_slice block"));
                    }
                    scene.runtime_slice = self.parse_runtime_slice()?;
                }
                Some(Token::Word("entity")) => {
                    self.next()?;
                    scene.entities.push(self.parse_entity()?);
                }
                _ => {
                    return Err(SceneError::new(
                        line,
                        "expected top-level 'runtime_slice' or 'entity' block",
                    ));
                }
            }
        }

        // The whole point of the sentinel: a file that *declares* the modern
        // format but stops early was cut short. Without this check a scene
        // truncated on an entity boundary parses as a valid SMALLER scene and
        // the missing entities vanish with no error.
        if strict && !saw_end {
            return Err(SceneError::new(
                self.lex.line,
                format!(
                    "truncated scene: file declares '{SCENE_FORMAT_HEADER}' but is missing the \
                     '{SCENE_END_SENTINEL}' terminator — it was likely cut short"
                ),
            ));
        }

        Ok(scene)
    }

    fn parse_runtime_slice(&mut self) -> Result<RuntimeSliceAuthoring, SceneError> {
        self.expect_lbrace()?;

        let mut slice = RuntimeSliceAuthoring::default();
        let mut saw_terrain = false;

        loop {
            let line = self.lex.line;
            match self.peek()? {
                Some(Token::RBrace) => {
                    self.next()?;
                    break;
                }
                Some(Token::Word(_)) => {
                    if let Some(Token::Word(kind)) = self.next()? {
                        match kind {
                            "terrain" => {
                                if saw_terrain {
                                    return Err(SceneError::new(
                                        line,
                                        "duplicate terrain block in runtime_slice",
                                    ));
                                }
                                slice.terrain = self.parse_runtime_terrain()?;
                                saw_terrain = true;
                            }
                            "paint_layer" => {
                                let name = self.expect_str()?;
                                slice
                                    .terrain
                                    .paint_layers
                                    .push(self.parse_terrain_paint_layer(name)?);
                            }
                            "vegetation_prototype" => {
                                let name = self.expect_str()?;
                                slice
                                    .vegetation_prototypes
                                    .push(self.parse_vegetation_prototype(name)?);
                            }
                            "vegetation_paint" => {
                                let prototype = self.expect_str()?;
                                slice
                                    .vegetation_paints
                                    .push(self.parse_vegetation_paint(prototype)?);
                            }
                            "static_mesh_actor" => {
                                let name = self.expect_str()?;
                                slice
                                    .static_mesh_actors
                                    .push(self.parse_static_mesh_actor(name)?);
                            }
                            "water_volume" => {
                                let name = self.expect_str()?;
                                slice.water_volumes.push(self.parse_water_volume(name)?);
                            }
                            "spawn_point" => {
                                let name = self.expect_str()?;
                                slice.spawn_points.push(self.parse_spawn_point(name)?);
                            }
                            "patrol_route" => {
                                let name = self.expect_str()?;
                                slice.patrol_routes.push(self.parse_patrol_route(name)?);
                            }
                            "vehicle_spawner" => {
                                let name = self.expect_str()?;
                                slice
                                    .vehicle_spawners
                                    .push(self.parse_vehicle_spawner(name)?);
                            }
                            "breakable_prop" => {
                                let name = self.expect_str()?;
                                slice.breakable_props.push(self.parse_breakable_prop(name)?);
                            }
                            _ => {
                                return Err(SceneError::new(
                                    line,
                                    "unknown runtime_slice block entry",
                                ));
                            }
                        }
                    }
                }
                _ => {
                    return Err(SceneError::new(line, "expected runtime_slice entry or '}'"));
                }
            }
        }

        Ok(slice)
    }

    fn parse_runtime_terrain(&mut self) -> Result<TerrainAuthoring, SceneError> {
        self.expect_lbrace()?;

        let mut terrain = TerrainAuthoring::default();

        loop {
            let line = self.lex.line;
            match self.peek()? {
                Some(Token::RBrace) => {
                    self.next()?;
                    break;
                }
                Some(Token::Word(_)) => {
                    if let Some(Token::Word(key)) = self.next()? {
                        match key {
                            "size" | "size_meters" => terrain.size_meters = self.expect_vec3()?,
                            "chunk_size" | "chunk_size_meters" => {
                                terrain.chunk_size_meters = self.expect_uint(u32::MAX as u64)? as u32
                            }
                            "height_scale" | "height_scale_meters" => {
                                terrain.height_scale_meters = self.expect_number()?
                            }
                            _ => return Err(SceneError::new(line, "unknown terrain field")),
                        }
                    }
                }
                _ => return Err(SceneError::new(line, "expected terrain field or '}'")),
            }
        }

        Ok(terrain)
    }

    fn parse_terrain_paint_layer(&mut self, name: String) -> Result<TerrainPaintLayer, SceneError> {
        self.expect_lbrace()?;

        let mut layer = TerrainPaintLayer {
            name,
            material: None,
            tiling: 1.0,
            weightmap_channel: 0,
        };

        loop {
            let line = self.lex.line;
            match self.peek()? {
                Some(Token::RBrace) => {
                    self.next()?;
                    break;
                }
                Some(Token::Word(_)) => {
                    if let Some(Token::Word(key)) = self.next()? {
                        match key {
                            "material" => layer.material = Some(self.expect_str()?),
                            "tiling" => layer.tiling = self.expect_number()?,
                            "channel" | "weightmap_channel" => {
                                layer.weightmap_channel = self.expect_uint(u8::MAX as u64)? as u8
                            }
                            _ => return Err(SceneError::new(line, "unknown paint_layer field")),
                        }
                    }
                }
                _ => return Err(SceneError::new(line, "expected paint_layer field or '}'")),
            }
        }

        Ok(layer)
    }

    fn parse_vegetation_prototype(
        &mut self,
        name: String,
    ) -> Result<VegetationPrototypeDef, SceneError> {
        self.expect_lbrace()?;

        let mut prototype = VegetationPrototypeDef {
            name,
            mesh: None,
            material: None,
            density_per_100m2: 1.0,
            random_yaw: true,
            align_to_normal: true,
        };

        loop {
            let line = self.lex.line;
            match self.peek()? {
                Some(Token::RBrace) => {
                    self.next()?;
                    break;
                }
                Some(Token::Word(_)) => {
                    if let Some(Token::Word(key)) = self.next()? {
                        match key {
                            "mesh" => prototype.mesh = Some(self.expect_str()?),
                            "material" => prototype.material = Some(self.expect_str()?),
                            "density" | "density_per_100m2" => {
                                prototype.density_per_100m2 = self.expect_number()?
                            }
                            "random_yaw" => prototype.random_yaw = self.expect_bool_flag()?,
                            "align_to_normal" => {
                                prototype.align_to_normal = self.expect_bool_flag()?
                            }
                            _ => {
                                return Err(SceneError::new(
                                    line,
                                    "unknown vegetation_prototype field",
                                ));
                            }
                        }
                    }
                }
                _ => {
                    return Err(SceneError::new(
                        line,
                        "expected vegetation_prototype field or '}'",
                    ));
                }
            }
        }

        Ok(prototype)
    }

    fn parse_vegetation_paint(
        &mut self,
        prototype: String,
    ) -> Result<VegetationPaintStroke, SceneError> {
        self.expect_lbrace()?;

        let mut stroke = VegetationPaintStroke {
            prototype,
            center: Vec3::ZERO,
            radius: 1.0,
            density: 1.0,
        };

        loop {
            let line = self.lex.line;
            match self.peek()? {
                Some(Token::RBrace) => {
                    self.next()?;
                    break;
                }
                Some(Token::Word(_)) => {
                    if let Some(Token::Word(key)) = self.next()? {
                        match key {
                            "center" => stroke.center = self.expect_vec3()?,
                            "radius" => stroke.radius = self.expect_number()?,
                            "density" => stroke.density = self.expect_number()?,
                            _ => {
                                return Err(SceneError::new(
                                    line,
                                    "unknown vegetation_paint field",
                                ));
                            }
                        }
                    }
                }
                _ => {
                    return Err(SceneError::new(
                        line,
                        "expected vegetation_paint field or '}'",
                    ));
                }
            }
        }

        Ok(stroke)
    }

    fn parse_static_mesh_actor(&mut self, name: String) -> Result<StaticMeshActorDef, SceneError> {
        self.expect_lbrace()?;

        let mut actor = StaticMeshActorDef {
            name,
            mesh: None,
            material: None,
            position: Vec3::ZERO,
            rotation_euler_deg: Vec3::ZERO,
            scale: Vec3::ONE,
        };

        loop {
            let line = self.lex.line;
            match self.peek()? {
                Some(Token::RBrace) => {
                    self.next()?;
                    break;
                }
                Some(Token::Word(_)) => {
                    if let Some(Token::Word(key)) = self.next()? {
                        match key {
                            "mesh" => actor.mesh = Some(self.expect_str()?),
                            "material" => actor.material = Some(self.expect_str()?),
                            "pos" | "position" => actor.position = self.expect_vec3()?,
                            "rot" | "rotation" => actor.rotation_euler_deg = self.expect_vec3()?,
                            "scale" => actor.scale = self.expect_vec3()?,
                            _ => {
                                return Err(SceneError::new(
                                    line,
                                    "unknown static_mesh_actor field",
                                ));
                            }
                        }
                    }
                }
                _ => {
                    return Err(SceneError::new(
                        line,
                        "expected static_mesh_actor field or '}'",
                    ));
                }
            }
        }

        Ok(actor)
    }

    fn parse_water_volume(&mut self, name: String) -> Result<WaterVolumeDef, SceneError> {
        self.expect_lbrace()?;

        let mut volume = WaterVolumeDef {
            name,
            material: None,
            center: Vec3::ZERO,
            extents: Vec3::ONE,
            wave_strength: 0.0,
        };

        loop {
            let line = self.lex.line;
            match self.peek()? {
                Some(Token::RBrace) => {
                    self.next()?;
                    break;
                }
                Some(Token::Word(_)) => {
                    if let Some(Token::Word(key)) = self.next()? {
                        match key {
                            "material" => volume.material = Some(self.expect_str()?),
                            "center" => volume.center = self.expect_vec3()?,
                            "extents" => volume.extents = self.expect_vec3()?,
                            "wave_strength" => volume.wave_strength = self.expect_number()?,
                            _ => return Err(SceneError::new(line, "unknown water_volume field")),
                        }
                    }
                }
                _ => return Err(SceneError::new(line, "expected water_volume field or '}'")),
            }
        }

        Ok(volume)
    }

    fn parse_spawn_point(&mut self, name: String) -> Result<SpawnPointDef, SceneError> {
        self.expect_lbrace()?;

        let mut spawn = SpawnPointDef {
            name,
            kind: SpawnPointKind::Generic,
            position: Vec3::ZERO,
            facing_yaw_deg: 0.0,
        };

        loop {
            let line = self.lex.line;
            match self.peek()? {
                Some(Token::RBrace) => {
                    self.next()?;
                    break;
                }
                Some(Token::Word(_)) => {
                    if let Some(Token::Word(key)) = self.next()? {
                        match key {
                            "kind" => {
                                spawn.kind = match self.expect_str()?.as_str() {
                                    "generic" => SpawnPointKind::Generic,
                                    "player" => SpawnPointKind::Player,
                                    "enemy" => SpawnPointKind::Enemy,
                                    "vehicle" => SpawnPointKind::Vehicle,
                                    _ => {
                                        return Err(SceneError::new(
                                            line,
                                            "unknown spawn_point kind",
                                        ));
                                    }
                                };
                            }
                            "pos" | "position" => spawn.position = self.expect_vec3()?,
                            "yaw" | "facing_yaw_deg" => {
                                spawn.facing_yaw_deg = self.expect_number()?
                            }
                            _ => return Err(SceneError::new(line, "unknown spawn_point field")),
                        }
                    }
                }
                _ => return Err(SceneError::new(line, "expected spawn_point field or '}'")),
            }
        }

        Ok(spawn)
    }

    fn parse_patrol_route(&mut self, name: String) -> Result<PatrolRouteDef, SceneError> {
        self.expect_lbrace()?;

        let mut route = PatrolRouteDef {
            name,
            points: Vec::new(),
            looped: false,
        };

        loop {
            let line = self.lex.line;
            match self.peek()? {
                Some(Token::RBrace) => {
                    self.next()?;
                    break;
                }
                Some(Token::Word(_)) => {
                    if let Some(Token::Word(key)) = self.next()? {
                        match key {
                            "looped" => route.looped = self.expect_bool_flag()?,
                            "point" => route.points.push(self.expect_vec3()?),
                            _ => return Err(SceneError::new(line, "unknown patrol_route field")),
                        }
                    }
                }
                _ => return Err(SceneError::new(line, "expected patrol_route field or '}'")),
            }
        }

        Ok(route)
    }

    fn parse_vehicle_spawner(&mut self, name: String) -> Result<VehicleSpawnerDef, SceneError> {
        self.expect_lbrace()?;

        let mut spawner = VehicleSpawnerDef {
            name,
            mesh: None,
            material: None,
            position: Vec3::ZERO,
            facing_yaw_deg: 0.0,
        };

        loop {
            let line = self.lex.line;
            match self.peek()? {
                Some(Token::RBrace) => {
                    self.next()?;
                    break;
                }
                Some(Token::Word(_)) => {
                    if let Some(Token::Word(key)) = self.next()? {
                        match key {
                            "mesh" => spawner.mesh = Some(self.expect_str()?),
                            "material" => spawner.material = Some(self.expect_str()?),
                            "pos" | "position" => spawner.position = self.expect_vec3()?,
                            "yaw" | "facing_yaw_deg" => {
                                spawner.facing_yaw_deg = self.expect_number()?
                            }
                            _ => {
                                return Err(SceneError::new(line, "unknown vehicle_spawner field"));
                            }
                        }
                    }
                }
                _ => {
                    return Err(SceneError::new(
                        line,
                        "expected vehicle_spawner field or '}'",
                    ));
                }
            }
        }

        Ok(spawner)
    }

    fn parse_breakable_prop(&mut self, name: String) -> Result<BreakablePropDef, SceneError> {
        self.expect_lbrace()?;

        let mut prop = BreakablePropDef {
            name,
            intact_mesh: None,
            broken_mesh: None,
            material: None,
            position: Vec3::ZERO,
            rotation_euler_deg: Vec3::ZERO,
            scale: Vec3::ONE,
            hit_points: 100.0,
            break_impulse: 0.0,
        };

        loop {
            let line = self.lex.line;
            match self.peek()? {
                Some(Token::RBrace) => {
                    self.next()?;
                    break;
                }
                Some(Token::Word(_)) => {
                    if let Some(Token::Word(key)) = self.next()? {
                        match key {
                            "intact_mesh" => prop.intact_mesh = Some(self.expect_str()?),
                            "broken_mesh" => prop.broken_mesh = Some(self.expect_str()?),
                            "material" => prop.material = Some(self.expect_str()?),
                            "pos" | "position" => prop.position = self.expect_vec3()?,
                            "rot" | "rotation" => prop.rotation_euler_deg = self.expect_vec3()?,
                            "scale" => prop.scale = self.expect_vec3()?,
                            "hit_points" => prop.hit_points = self.expect_number()?,
                            "break_impulse" => prop.break_impulse = self.expect_number()?,
                            _ => {
                                return Err(SceneError::new(line, "unknown breakable_prop field"));
                            }
                        }
                    }
                }
                _ => {
                    return Err(SceneError::new(
                        line,
                        "expected breakable_prop field or '}'",
                    ));
                }
            }
        }

        Ok(prop)
    }

    fn parse_entity(&mut self) -> Result<EntityDef, SceneError> {
        let name = self.expect_str()?;
        self.expect_lbrace()?;

        let mut def = EntityDef {
            name,
            transform: None,
            velocity: None,
            rigid_body: None,
            sphere_collider: None,
            particle_emitter: None,
            camera: None,
            active_camera: false,
            light: None,
            stats: None,
            parent_name: None,
        };

        loop {
            let _ln = self.lex.line;
            match self.peek()? {
                Some(Token::RBrace) => {
                    self.next()?;
                    break;
                }
                Some(Token::Word(_)) => {
                    // Peek the word to decide which component.
                    if let Some(Token::Word(w)) = self.next()? {
                        match w {
                            "transform" => def.transform = Some(self.parse_transform()?),
                            "velocity" => def.velocity = Some(self.parse_velocity()?),
                            "rigid_body" => def.rigid_body = Some(self.parse_rigid_body()?),
                            "sphere_collider" => {
                                def.sphere_collider = Some(self.parse_sphere_collider()?)
                            }
                            "particle_emitter" => {
                                def.particle_emitter = Some(self.parse_particle_emitter()?)
                            }
                            "camera" => def.camera = Some(self.parse_camera()?),
                            "active_camera" => def.active_camera = true,
                            "light" => def.light = Some(self.parse_light()?),
                            "stats" => def.stats = Some(self.parse_stats()?),
                            "parent" => def.parent_name = Some(self.expect_str()?),
                            _ => {
                                return Err(SceneError::new(_ln, format!("unknown component")));
                            }
                        }
                    }
                }
                _ => {
                    return Err(SceneError::new(
                        _ln,
                        format!("expected component or '}}', got unexpected token"),
                    ));
                }
            }
        }

        Ok(def)
    }

    // ── Component parsers ─────────────────────────────────────────────────

    fn parse_transform(&mut self) -> Result<TransformDef, SceneError> {
        self.expect_lbrace()?;

        let mut t = TransformDef {
            position: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        };

        loop {
            let _ln = self.lex.line;
            match self.peek()? {
                Some(Token::RBrace) => {
                    self.next()?;
                    break;
                }
                Some(Token::Word(_)) => {
                    if let Some(Token::Word(key)) = self.next()? {
                        match key {
                            "pos" | "position" => {
                                let x = self.expect_number()?;
                                let y = self.expect_number()?;
                                let z = self.expect_number()?;
                                t.position = Vec3::new(x, y, z);
                            }
                            "rot" | "rotation" => {
                                let x = self.expect_number()?;
                                let y = self.expect_number()?;
                                let z = self.expect_number()?;
                                let w = self.expect_number()?;
                                t.rotation = Quat::new(x, y, z, w);
                            }
                            "scale" => {
                                let x = self.expect_number()?;
                                let y = self.expect_number()?;
                                let z = self.expect_number()?;
                                t.scale = Vec3::new(x, y, z);
                            }
                            "euler" => {
                                // Convenience: euler <pitch_deg> <yaw_deg> <roll_deg>
                                let x = self.expect_number()?;
                                let y = self.expect_number()?;
                                let z = self.expect_number()?;
                                use crate::math::deg_to_rad;
                                t.rotation = Quat::from_euler_xyz(
                                    deg_to_rad(x),
                                    deg_to_rad(y),
                                    deg_to_rad(z),
                                );
                            }
                            _ => {
                                return Err(SceneError::new(
                                    _ln,
                                    format!("unknown transform field"),
                                ));
                            }
                        }
                    }
                }
                _ => {
                    return Err(SceneError::new(
                        _ln,
                        format!("expected field or '}}', got unexpected token"),
                    ));
                }
            }
        }

        Ok(t)
    }

    fn parse_camera(&mut self) -> Result<CameraDef, SceneError> {
        self.expect_lbrace()?;

        let mut c = CameraDef {
            fov: 60.0,
            near: 0.1,
            far: 1000.0,
        };

        loop {
            let _ln = self.lex.line;
            match self.peek()? {
                Some(Token::RBrace) => {
                    self.next()?;
                    break;
                }
                Some(Token::Word(_)) => {
                    if let Some(Token::Word(key)) = self.next()? {
                        match key {
                            "fov" => c.fov = self.expect_number()?,
                            "near" => c.near = self.expect_number()?,
                            "far" => c.far = self.expect_number()?,
                            _other => {
                                return Err(SceneError::new(_ln, format!("unknown camera field")));
                            }
                        }
                    }
                }
                _ => {
                    return Err(SceneError::new(
                        _ln,
                        format!("expected field or '}}', got unexpected token"),
                    ));
                }
            }
        }

        Ok(c)
    }

    fn parse_velocity(&mut self) -> Result<VelocityDef, SceneError> {
        self.expect_lbrace()?;

        let mut velocity = VelocityDef {
            linear: Vec3::ZERO,
            angular: Vec3::ZERO,
        };

        loop {
            let line = self.lex.line;
            match self.peek()? {
                Some(Token::RBrace) => {
                    self.next()?;
                    break;
                }
                Some(Token::Word(_)) => {
                    if let Some(Token::Word(key)) = self.next()? {
                        match key {
                            "linear" => {
                                let x = self.expect_number()?;
                                let y = self.expect_number()?;
                                let z = self.expect_number()?;
                                velocity.linear = Vec3::new(x, y, z);
                            }
                            "angular" => {
                                let x = self.expect_number()?;
                                let y = self.expect_number()?;
                                let z = self.expect_number()?;
                                velocity.angular = Vec3::new(x, y, z);
                            }
                            _ => {
                                return Err(SceneError::new(line, "unknown velocity field"));
                            }
                        }
                    }
                }
                _ => {
                    return Err(SceneError::new(
                        line,
                        "expected field or '}', got unexpected token",
                    ));
                }
            }
        }

        Ok(velocity)
    }

    fn parse_rigid_body(&mut self) -> Result<RigidBodyDef, SceneError> {
        self.expect_lbrace()?;

        let mut rigid_body = RigidBodyDef {
            gravity_scale: 1.0,
            linear_damping: 0.0,
            angular_damping: 0.0,
            restitution: 0.5,
        };

        loop {
            let line = self.lex.line;
            match self.peek()? {
                Some(Token::RBrace) => {
                    self.next()?;
                    break;
                }
                Some(Token::Word(_)) => {
                    if let Some(Token::Word(key)) = self.next()? {
                        match key {
                            "gravity_scale" => rigid_body.gravity_scale = self.expect_number()?,
                            "linear_damping" => rigid_body.linear_damping = self.expect_number()?,
                            "angular_damping" => {
                                rigid_body.angular_damping = self.expect_number()?
                            }
                            "restitution" => rigid_body.restitution = self.expect_number()?,
                            _ => {
                                return Err(SceneError::new(line, "unknown rigid_body field"));
                            }
                        }
                    }
                }
                _ => {
                    return Err(SceneError::new(
                        line,
                        "expected field or '}', got unexpected token",
                    ));
                }
            }
        }

        Ok(rigid_body)
    }

    fn parse_sphere_collider(&mut self) -> Result<SphereColliderDef, SceneError> {
        self.expect_lbrace()?;

        let mut collider = SphereColliderDef { radius: 0.5 };

        loop {
            let line = self.lex.line;
            match self.peek()? {
                Some(Token::RBrace) => {
                    self.next()?;
                    break;
                }
                Some(Token::Word(_)) => {
                    if let Some(Token::Word(key)) = self.next()? {
                        match key {
                            "radius" => collider.radius = self.expect_number()?,
                            _ => {
                                return Err(SceneError::new(line, "unknown sphere_collider field"));
                            }
                        }
                    }
                }
                _ => {
                    return Err(SceneError::new(
                        line,
                        "expected field or '}', got unexpected token",
                    ));
                }
            }
        }

        Ok(collider)
    }

    fn parse_particle_emitter(&mut self) -> Result<ParticleEmitterDef, SceneError> {
        self.expect_lbrace()?;

        let default = ParticleEmitter::spark_fountain();
        let mut emitter = ParticleEmitterDef {
            emitting: default.emitting,
            rate_per_second: default.rate_per_second,
            max_particles: default.max_particles,
            spawn_radius: default.spawn_radius,
            direction: default.direction,
            spread: default.spread,
            speed_min: default.speed_min,
            speed_max: default.speed_max,
            lifetime_min: default.lifetime_min,
            lifetime_max: default.lifetime_max,
            gravity_scale: default.gravity_scale,
            linear_damping: default.linear_damping,
            start_size: default.start_size,
            end_size: default.end_size,
            start_color: default.start_color,
            end_color: default.end_color,
            seed: default.seed,
        };

        loop {
            let line = self.lex.line;
            match self.peek()? {
                Some(Token::RBrace) => {
                    self.next()?;
                    break;
                }
                Some(Token::Word(_)) => {
                    if let Some(Token::Word(key)) = self.next()? {
                        match key {
                            "emitting" => emitter.emitting = self.expect_number()? > 0.0,
                            "rate" | "rate_per_second" => {
                                emitter.rate_per_second = self.expect_number()?
                            }
                            "max" | "max_particles" => {
                                emitter.max_particles = self.expect_uint(u32::MAX as u64)? as u32
                            }
                            "spawn_radius" => emitter.spawn_radius = self.expect_number()?,
                            "dir" | "direction" => {
                                let x = self.expect_number()?;
                                let y = self.expect_number()?;
                                let z = self.expect_number()?;
                                emitter.direction = Vec3::new(x, y, z);
                            }
                            "spread" => emitter.spread = self.expect_number()?,
                            "speed_min" => emitter.speed_min = self.expect_number()?,
                            "speed_max" => emitter.speed_max = self.expect_number()?,
                            "lifetime_min" => emitter.lifetime_min = self.expect_number()?,
                            "lifetime_max" => emitter.lifetime_max = self.expect_number()?,
                            "gravity_scale" => emitter.gravity_scale = self.expect_number()?,
                            "linear_damping" => emitter.linear_damping = self.expect_number()?,
                            "start_size" => emitter.start_size = self.expect_number()?,
                            "end_size" => emitter.end_size = self.expect_number()?,
                            "start_color" => {
                                let r = self.expect_number()?;
                                let g = self.expect_number()?;
                                let b = self.expect_number()?;
                                let a = self.expect_number()?;
                                emitter.start_color = Color::new(r, g, b, a);
                            }
                            "end_color" => {
                                let r = self.expect_number()?;
                                let g = self.expect_number()?;
                                let b = self.expect_number()?;
                                let a = self.expect_number()?;
                                emitter.end_color = Color::new(r, g, b, a);
                            }
                            "seed" => emitter.seed = self.expect_uint(u32::MAX as u64)? as u32,
                            _ => {
                                return Err(SceneError::new(
                                    line,
                                    "unknown particle_emitter field",
                                ));
                            }
                        }
                    }
                }
                _ => {
                    return Err(SceneError::new(
                        line,
                        "expected field or '}', got unexpected token",
                    ));
                }
            }
        }

        Ok(emitter)
    }

    fn parse_light(&mut self) -> Result<LightDef, SceneError> {
        self.expect_lbrace()?;

        let mut l = LightDef {
            direction: Vec3::new(0.0, -1.0, 0.0),
            color: Color::WHITE,
            intensity: 1.0,
            ambient: 0.15,
        };

        loop {
            let _ln = self.lex.line;
            match self.peek()? {
                Some(Token::RBrace) => {
                    self.next()?;
                    break;
                }
                Some(Token::Word(_)) => {
                    if let Some(Token::Word(key)) = self.next()? {
                        match key {
                            "dir" | "direction" => {
                                let x = self.expect_number()?;
                                let y = self.expect_number()?;
                                let z = self.expect_number()?;
                                l.direction = Vec3::new(x, y, z);
                            }
                            "color" => {
                                let r = self.expect_number()?;
                                let g = self.expect_number()?;
                                let b = self.expect_number()?;
                                let a = self.expect_number()?;
                                l.color = Color::new(r, g, b, a);
                            }
                            "intensity" => l.intensity = self.expect_number()?,
                            "ambient" => l.ambient = self.expect_number()?,
                            _ => {
                                return Err(SceneError::new(_ln, format!("unknown light field")));
                            }
                        }
                    }
                }
                _ => {
                    return Err(SceneError::new(
                        _ln,
                        format!("expected field or '}}', got unexpected token"),
                    ));
                }
            }
        }

        Ok(l)
    }

    fn parse_stats(&mut self) -> Result<StatsDef, SceneError> {
        self.expect_lbrace()?;

        let mut s = StatsDef {
            health: 100.0,
            mana: 0.0,
            attack: 0.0,
            defense: 0.0,
            speed: 0.0,
        };

        loop {
            let _ln = self.lex.line;
            match self.peek()? {
                Some(Token::RBrace) => {
                    self.next()?;
                    break;
                }
                Some(Token::Word(_)) => {
                    if let Some(Token::Word(key)) = self.next()? {
                        match key {
                            "health" => s.health = self.expect_number()?,
                            "mana" => s.mana = self.expect_number()?,
                            "attack" => s.attack = self.expect_number()?,
                            "defense" => s.defense = self.expect_number()?,
                            "speed" => s.speed = self.expect_number()?,
                            _other => {
                                return Err(SceneError::new(_ln, format!("unknown stats field")));
                            }
                        }
                    }
                }
                _ => {
                    return Err(SceneError::new(
                        _ln,
                        format!("expected field or '}}', got unexpected token"),
                    ));
                }
            }
        }

        Ok(s)
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Public API — parse
// ═══════════════════════════════════════════════════════════════════════════════

/// Parse a `.hsc` scene file from a path.
pub fn load_scene<P: AsRef<Path>>(path: P) -> Result<SceneData, SceneError> {
    let src = fs::read_to_string(path).map_err(|e| SceneError::new(0, format!("IO: {e}")))?;
    parse_scene(&src)
}

/// Parse a scene from a string.
pub fn parse_scene(src: &str) -> Result<SceneData, SceneError> {
    Parser::new(src).parse_scene()
}

/// Spawn the entities described by `scene` into `world`.
///
/// Returns a map from entity name → [`Entity`] handle for further setup
/// (e.g. attaching meshes or textures that the scene format does not describe).
pub fn spawn_scene(
    world: &mut World,
    scene: &SceneData,
) -> Result<HashMap<String, Entity>, SceneError> {
    let mut name_map: HashMap<String, Entity> = HashMap::new();

    // First pass: spawn entities and attach direct components.
    for def in &scene.entities {
        let entity = world.spawn();

        if let Some(t) = &def.transform {
            world.add_component(
                entity,
                Transform {
                    position: t.position,
                    rotation: t.rotation,
                    scale: t.scale,
                },
            );
            world.add_component(entity, GlobalTransform::identity());
        }

        if let Some(velocity) = &def.velocity {
            world.add_component(
                entity,
                Velocity {
                    linear: velocity.linear,
                    angular: velocity.angular,
                },
            );
        }

        if let Some(rigid_body) = &def.rigid_body {
            world.add_component(
                entity,
                RigidBody {
                    gravity_scale: rigid_body.gravity_scale,
                    linear_damping: rigid_body.linear_damping,
                    angular_damping: rigid_body.angular_damping,
                    restitution: rigid_body.restitution,
                },
            );
        }

        if let Some(collider) = &def.sphere_collider {
            world.add_component(entity, SphereCollider::new(collider.radius));
        }

        if let Some(emitter) = &def.particle_emitter {
            world.add_component(
                entity,
                ParticleEmitter {
                    emitting: emitter.emitting,
                    rate_per_second: emitter.rate_per_second,
                    max_particles: emitter.max_particles,
                    spawn_radius: emitter.spawn_radius,
                    direction: emitter.direction,
                    spread: emitter.spread,
                    speed_min: emitter.speed_min,
                    speed_max: emitter.speed_max,
                    lifetime_min: emitter.lifetime_min,
                    lifetime_max: emitter.lifetime_max,
                    gravity_scale: emitter.gravity_scale,
                    linear_damping: emitter.linear_damping,
                    start_size: emitter.start_size,
                    end_size: emitter.end_size,
                    start_color: emitter.start_color,
                    end_color: emitter.end_color,
                    accumulator: 0.0,
                    seed: emitter.seed,
                },
            );
        }

        if let Some(c) = &def.camera {
            world.add_component(
                entity,
                Camera::perspective(c.fov, 16.0 / 9.0, c.near, c.far),
            );
        }

        if def.active_camera {
            world.add_component(entity, ActiveCamera);
        }

        if let Some(l) = &def.light {
            world.add_component(
                entity,
                DirectionalLight {
                    direction: l.direction.normalize(),
                    color: l.color,
                    intensity: l.intensity,
                    ambient: l.ambient,
                },
            );
        }

        if let Some(s) = &def.stats {
            world.add_component(
                entity,
                Stats::new(s.health, s.mana, s.attack, s.defense, s.speed),
            );
        }

        if !def.name.is_empty() {
            name_map.insert(def.name.clone(), entity);
        }
    }

    // Second pass: resolve parent references.
    for def in &scene.entities {
        if let Some(parent_name) = &def.parent_name {
            let child = name_map.get(&def.name).copied().ok_or_else(|| {
                SceneError::new(0, format!("entity '{}' not found in name map", def.name))
            })?;
            let parent = name_map
                .get(parent_name.as_str())
                .copied()
                .ok_or_else(|| SceneError::new(0, format!("parent '{}' not found", parent_name)))?;
            world.set_parent(child, parent);
        }
    }

    Ok(name_map)
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Public API — serialize (save)
// ═══════════════════════════════════════════════════════════════════════════════

/// Serialise a [`SceneData`] back to the `.hsc` text format.
pub fn serialize_scene(scene: &SceneData) -> String {
    let mut out = String::with_capacity(1024);

    // Version header first: it is what puts a reader into strict mode and so
    // makes the trailing sentinel mandatory (M11).
    out.push_str(&format!("{SCENE_FORMAT_HEADER} {SCENE_FORMAT_VERSION}\n\n"));

    if !scene.name.is_empty() {
        out.push_str(&format!("scene \"{}\"\n\n", scene.name));
    }

    if !scene.runtime_slice.is_empty() {
        write_runtime_slice(&mut out, &scene.runtime_slice);
        out.push('\n');
    }

    for def in &scene.entities {
        out.push_str(&format!("entity \"{}\" {{\n", def.name));

        if let Some(t) = &def.transform {
            out.push_str("  transform {\n");
            out.push_str(&format!(
                "    pos {} {} {}\n",
                fmt_f32(t.position.x),
                fmt_f32(t.position.y),
                fmt_f32(t.position.z),
            ));
            out.push_str(&format!(
                "    rot {} {} {} {}\n",
                fmt_f32(t.rotation.x),
                fmt_f32(t.rotation.y),
                fmt_f32(t.rotation.z),
                fmt_f32(t.rotation.w),
            ));
            out.push_str(&format!(
                "    scale {} {} {}\n",
                fmt_f32(t.scale.x),
                fmt_f32(t.scale.y),
                fmt_f32(t.scale.z),
            ));
            out.push_str("  }\n");
        }

        if let Some(velocity) = &def.velocity {
            out.push_str("  velocity {\n");
            out.push_str(&format!(
                "    linear {} {} {}\n",
                fmt_f32(velocity.linear.x),
                fmt_f32(velocity.linear.y),
                fmt_f32(velocity.linear.z),
            ));
            out.push_str(&format!(
                "    angular {} {} {}\n",
                fmt_f32(velocity.angular.x),
                fmt_f32(velocity.angular.y),
                fmt_f32(velocity.angular.z),
            ));
            out.push_str("  }\n");
        }

        if let Some(rigid_body) = &def.rigid_body {
            out.push_str("  rigid_body {\n");
            out.push_str(&format!(
                "    gravity_scale {}\n",
                fmt_f32(rigid_body.gravity_scale),
            ));
            out.push_str(&format!(
                "    linear_damping {}\n",
                fmt_f32(rigid_body.linear_damping),
            ));
            out.push_str(&format!(
                "    angular_damping {}\n",
                fmt_f32(rigid_body.angular_damping),
            ));
            out.push_str(&format!(
                "    restitution {}\n",
                fmt_f32(rigid_body.restitution),
            ));
            out.push_str("  }\n");
        }

        if let Some(collider) = &def.sphere_collider {
            out.push_str(&format!(
                "  sphere_collider {{ radius {} }}\n",
                fmt_f32(collider.radius),
            ));
        }

        if let Some(emitter) = &def.particle_emitter {
            out.push_str("  particle_emitter {\n");
            out.push_str(&format!(
                "    emitting {}\n",
                if emitter.emitting { "1" } else { "0" },
            ));
            out.push_str(&format!(
                "    rate_per_second {}\n",
                fmt_f32(emitter.rate_per_second),
            ));
            // Integer fields are written directly, NOT via `as f32` + fmt_f32:
            // an f32 cannot represent every u32 exactly (only up to 2^24), so
            // routing a large value through f32 would serialize the rounded
            // integer and break the round trip.
            out.push_str(&format!("    max_particles {}\n", emitter.max_particles));
            out.push_str(&format!(
                "    spawn_radius {}\n",
                fmt_f32(emitter.spawn_radius),
            ));
            out.push_str(&format!(
                "    direction {} {} {}\n",
                fmt_f32(emitter.direction.x),
                fmt_f32(emitter.direction.y),
                fmt_f32(emitter.direction.z),
            ));
            out.push_str(&format!("    spread {}\n", fmt_f32(emitter.spread)));
            out.push_str(&format!("    speed_min {}\n", fmt_f32(emitter.speed_min)));
            out.push_str(&format!("    speed_max {}\n", fmt_f32(emitter.speed_max)));
            out.push_str(&format!(
                "    lifetime_min {}\n",
                fmt_f32(emitter.lifetime_min),
            ));
            out.push_str(&format!(
                "    lifetime_max {}\n",
                fmt_f32(emitter.lifetime_max),
            ));
            out.push_str(&format!(
                "    gravity_scale {}\n",
                fmt_f32(emitter.gravity_scale),
            ));
            out.push_str(&format!(
                "    linear_damping {}\n",
                fmt_f32(emitter.linear_damping),
            ));
            out.push_str(&format!("    start_size {}\n", fmt_f32(emitter.start_size)));
            out.push_str(&format!("    end_size {}\n", fmt_f32(emitter.end_size)));
            out.push_str(&format!(
                "    start_color {} {} {} {}\n",
                fmt_f32(emitter.start_color.r),
                fmt_f32(emitter.start_color.g),
                fmt_f32(emitter.start_color.b),
                fmt_f32(emitter.start_color.a),
            ));
            out.push_str(&format!(
                "    end_color {} {} {} {}\n",
                fmt_f32(emitter.end_color.r),
                fmt_f32(emitter.end_color.g),
                fmt_f32(emitter.end_color.b),
                fmt_f32(emitter.end_color.a),
            ));
            out.push_str(&format!("    seed {}\n", emitter.seed));
            out.push_str("  }\n");
        }

        if let Some(c) = &def.camera {
            out.push_str(&format!(
                "  camera {{ fov {} near {} far {} }}\n",
                fmt_f32(c.fov),
                fmt_f32(c.near),
                fmt_f32(c.far),
            ));
        }

        if def.active_camera {
            out.push_str("  active_camera\n");
        }

        if let Some(l) = &def.light {
            out.push_str("  light {\n");
            out.push_str(&format!(
                "    dir {} {} {}\n",
                fmt_f32(l.direction.x),
                fmt_f32(l.direction.y),
                fmt_f32(l.direction.z),
            ));
            out.push_str(&format!(
                "    color {} {} {} {}\n",
                fmt_f32(l.color.r),
                fmt_f32(l.color.g),
                fmt_f32(l.color.b),
                fmt_f32(l.color.a),
            ));
            out.push_str(&format!("    intensity {}\n", fmt_f32(l.intensity)));
            out.push_str(&format!("    ambient {}\n", fmt_f32(l.ambient)));
            out.push_str("  }\n");
        }

        if let Some(s) = &def.stats {
            out.push_str(&format!(
                "  stats {{ health {} mana {} attack {} defense {} speed {} }}\n",
                fmt_f32(s.health),
                fmt_f32(s.mana),
                fmt_f32(s.attack),
                fmt_f32(s.defense),
                fmt_f32(s.speed),
            ));
        }

        if let Some(p) = &def.parent_name {
            out.push_str(&format!("  parent \"{}\"\n", p));
        }

        out.push_str("}\n\n");
    }

    // Terminator. A reader that declared strict mode via the header and then
    // hits EOF without this knows the file was cut short (M11).
    out.push_str(SCENE_END_SENTINEL);
    out.push('\n');

    out
}

/// Save a scene to a file, atomically and durably.
///
/// The scene is serialized fully in memory first, then written through
/// [`atomic_write`]: a power loss leaves either the previous scene or the new
/// scene on disk, never a half-written one (ADR 0003 Pillar 3). On any error
/// the destination retains its previous complete contents.
///
/// The error type is [`AtomicWriteError`], not [`SceneError`]: serialization
/// is infallible, so the only way this fails is the write itself. Previously
/// the `io::Error` was laundered into `SceneError::new(0, format!("IO: {e}"))`
/// — a `String` error with a meaningless line number that made a disk failure
/// type-indistinguishable from a parse failure.
pub fn save_scene<P: AsRef<Path>>(path: P, scene: &SceneData) -> Result<(), AtomicWriteError> {
    let text = serialize_scene(scene);
    atomic_write(path.as_ref(), text.as_bytes())
}

fn write_runtime_slice(out: &mut String, slice: &RuntimeSliceAuthoring) {
    out.push_str("runtime_slice {\n");

    if slice.terrain != TerrainAuthoring::default() {
        out.push_str("  terrain {\n");
        out.push_str("    size ");
        write_runtime_vec3(out, slice.terrain.size_meters);
        out.push('\n');
        out.push_str(&format!(
            "    chunk_size {}\n",
            slice.terrain.chunk_size_meters,
        ));
        out.push_str(&format!(
            "    height_scale {}\n",
            fmt_f32(slice.terrain.height_scale_meters),
        ));
        out.push_str("  }\n");
    }

    for layer in &slice.terrain.paint_layers {
        out.push_str(&format!("  paint_layer \"{}\" {{\n", layer.name));
        if let Some(material) = &layer.material {
            out.push_str(&format!("    material \"{}\"\n", material));
        }
        out.push_str(&format!("    tiling {}\n", fmt_f32(layer.tiling)));
        out.push_str(&format!("    channel {}\n", layer.weightmap_channel));
        out.push_str("  }\n");
    }

    for prototype in &slice.vegetation_prototypes {
        out.push_str(&format!(
            "  vegetation_prototype \"{}\" {{\n",
            prototype.name
        ));
        if let Some(mesh) = &prototype.mesh {
            out.push_str(&format!("    mesh \"{}\"\n", mesh));
        }
        if let Some(material) = &prototype.material {
            out.push_str(&format!("    material \"{}\"\n", material));
        }
        out.push_str(&format!(
            "    density {}\n",
            fmt_f32(prototype.density_per_100m2),
        ));
        out.push_str(&format!(
            "    random_yaw {}\n",
            if prototype.random_yaw { 1 } else { 0 }
        ));
        out.push_str(&format!(
            "    align_to_normal {}\n",
            if prototype.align_to_normal { 1 } else { 0 }
        ));
        out.push_str("  }\n");
    }

    for stroke in &slice.vegetation_paints {
        out.push_str(&format!("  vegetation_paint \"{}\" {{\n", stroke.prototype));
        out.push_str("    center ");
        write_runtime_vec3(out, stroke.center);
        out.push('\n');
        out.push_str(&format!("    radius {}\n", fmt_f32(stroke.radius)));
        out.push_str(&format!("    density {}\n", fmt_f32(stroke.density)));
        out.push_str("  }\n");
    }

    for actor in &slice.static_mesh_actors {
        out.push_str(&format!("  static_mesh_actor \"{}\" {{\n", actor.name));
        if let Some(mesh) = &actor.mesh {
            out.push_str(&format!("    mesh \"{}\"\n", mesh));
        }
        if let Some(material) = &actor.material {
            out.push_str(&format!("    material \"{}\"\n", material));
        }
        out.push_str("    pos ");
        write_runtime_vec3(out, actor.position);
        out.push('\n');
        out.push_str("    rot ");
        write_runtime_vec3(out, actor.rotation_euler_deg);
        out.push('\n');
        out.push_str("    scale ");
        write_runtime_vec3(out, actor.scale);
        out.push('\n');
        out.push_str("  }\n");
    }

    for volume in &slice.water_volumes {
        out.push_str(&format!("  water_volume \"{}\" {{\n", volume.name));
        if let Some(material) = &volume.material {
            out.push_str(&format!("    material \"{}\"\n", material));
        }
        out.push_str("    center ");
        write_runtime_vec3(out, volume.center);
        out.push('\n');
        out.push_str("    extents ");
        write_runtime_vec3(out, volume.extents);
        out.push('\n');
        out.push_str(&format!(
            "    wave_strength {}\n",
            fmt_f32(volume.wave_strength),
        ));
        out.push_str("  }\n");
    }

    for spawn in &slice.spawn_points {
        out.push_str(&format!("  spawn_point \"{}\" {{\n", spawn.name));
        let kind = match spawn.kind {
            SpawnPointKind::Generic => "generic",
            SpawnPointKind::Player => "player",
            SpawnPointKind::Enemy => "enemy",
            SpawnPointKind::Vehicle => "vehicle",
        };
        out.push_str(&format!("    kind \"{}\"\n", kind));
        out.push_str("    pos ");
        write_runtime_vec3(out, spawn.position);
        out.push('\n');
        out.push_str(&format!("    yaw {}\n", fmt_f32(spawn.facing_yaw_deg)));
        out.push_str("  }\n");
    }

    for route in &slice.patrol_routes {
        out.push_str(&format!("  patrol_route \"{}\" {{\n", route.name));
        out.push_str(&format!(
            "    looped {}\n",
            if route.looped { 1 } else { 0 }
        ));
        for point in &route.points {
            out.push_str("    point ");
            write_runtime_vec3(out, *point);
            out.push('\n');
        }
        out.push_str("  }\n");
    }

    for spawner in &slice.vehicle_spawners {
        out.push_str(&format!("  vehicle_spawner \"{}\" {{\n", spawner.name));
        if let Some(mesh) = &spawner.mesh {
            out.push_str(&format!("    mesh \"{}\"\n", mesh));
        }
        if let Some(material) = &spawner.material {
            out.push_str(&format!("    material \"{}\"\n", material));
        }
        out.push_str("    pos ");
        write_runtime_vec3(out, spawner.position);
        out.push('\n');
        out.push_str(&format!("    yaw {}\n", fmt_f32(spawner.facing_yaw_deg)));
        out.push_str("  }\n");
    }

    for prop in &slice.breakable_props {
        out.push_str(&format!("  breakable_prop \"{}\" {{\n", prop.name));
        if let Some(mesh) = &prop.intact_mesh {
            out.push_str(&format!("    intact_mesh \"{}\"\n", mesh));
        }
        if let Some(mesh) = &prop.broken_mesh {
            out.push_str(&format!("    broken_mesh \"{}\"\n", mesh));
        }
        if let Some(material) = &prop.material {
            out.push_str(&format!("    material \"{}\"\n", material));
        }
        out.push_str("    pos ");
        write_runtime_vec3(out, prop.position);
        out.push('\n');
        out.push_str("    rot ");
        write_runtime_vec3(out, prop.rotation_euler_deg);
        out.push('\n');
        out.push_str("    scale ");
        write_runtime_vec3(out, prop.scale);
        out.push('\n');
        out.push_str(&format!("    hit_points {}\n", fmt_f32(prop.hit_points)));
        out.push_str(&format!(
            "    break_impulse {}\n",
            fmt_f32(prop.break_impulse),
        ));
        out.push_str("  }\n");
    }

    out.push_str("}\n");
}

fn write_runtime_vec3(out: &mut String, value: Vec3) {
    out.push_str(&fmt_f32(value.x));
    out.push(' ');
    out.push_str(&fmt_f32(value.y));
    out.push(' ');
    out.push_str(&fmt_f32(value.z));
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Format an f32 without unnecessary trailing zeros.
fn fmt_f32(v: f32) -> String {
    if v == v.trunc() {
        format!("{}", v as i64)
    } else {
        // Up to 6 decimal places, strip trailing zeros.
        let s = format!("{:.6}", v);
        s.trim_end_matches('0').trim_end_matches('.').to_owned()
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Tests
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Parent;

    const SAMPLE: &str = r#"
scene "Test Level"

# A comment
entity "player" {
  transform { pos 0 1 0  rot 0 0 0 1  scale 1 1 1 }
  velocity { linear 0 0 0  angular 0 1.2 0 }
  rigid_body { gravity_scale 1 linear_damping 0.2 angular_damping 0.1 restitution 0.6 }
  sphere_collider { radius 0.5 }
  camera { fov 60 near 0.1 far 1000 }
  active_camera
  stats { health 100 mana 50 attack 10 defense 5 speed 3 }
}

entity "fx" {
  transform { pos 0 2.6 0 }
  particle_emitter {
    emitting 1
    rate_per_second 24
    max_particles 96
    spawn_radius 0.1
    direction 0 1 0
    spread 0.35
    speed_min 1.5
    speed_max 3
    lifetime_min 0.5
    lifetime_max 1.2
    gravity_scale 0.8
    linear_damping 1.4
    start_size 0.14
    end_size 0.02
    start_color 1 0.8 0.2 1
    end_color 1 0.25 0.05 0
    seed 9
  }
  parent "player"
}

entity "sun" {
  light { dir 0.5 -1 0.3  color 1 1 1 1  intensity 1  ambient 0.15 }
}

entity "crate" {
  transform { pos 5 0 -3 }
  parent "player"
}
"#;

    const RUNTIME_SLICE_SAMPLE: &str = r#"
scene "Island"

runtime_slice {
  terrain { size 1024 256 1024 chunk_size 64 height_scale 80 }
  paint_layer "sand" { material "materials/sand" tiling 6 channel 0 }
  vegetation_prototype "palm" { mesh "meshes/palm" material "materials/palm" density 2.5 random_yaw 1 align_to_normal 1 }
  vegetation_paint "palm" { center 12 0 18 radius 9 density 1.25 }
  static_mesh_actor "rock_large" { mesh "meshes/rock_large" material "materials/rock" pos 1 0 2 rot 0 45 0 scale 2 2 2 }
  water_volume "ocean" { material "materials/ocean" center 512 0 512 extents 1024 8 1024 wave_strength 0.35 }
  spawn_point "player_start" { kind "player" pos 32 4 32 yaw 180 }
  patrol_route "north_path" { looped 1 point 0 0 0 point 10 0 10 }
  vehicle_spawner "jeep" { mesh "meshes/jeep" material "materials/jeep" pos 40 0 40 yaw 90 }
  breakable_prop "crate" { intact_mesh "meshes/crate" broken_mesh "meshes/crate_broken" material "materials/crate" pos 14 0 20 rot 0 0 0 scale 1 1 1 hit_points 40 break_impulse 22 }
}

entity "player" {
  transform { pos 0 1 0 }
}
"#;

    #[test]
    fn parse_roundtrip() {
        let scene = parse_scene(SAMPLE).unwrap();
        assert_eq!(scene.name, "Test Level");
        assert_eq!(scene.entities.len(), 4);

        let player = &scene.entities[0];
        assert_eq!(player.name, "player");
        assert!(player.transform.is_some());
        assert!(player.velocity.is_some());
        assert!(player.rigid_body.is_some());
        assert!(player.sphere_collider.is_some());
        assert!(player.camera.is_some());
        assert!(player.active_camera);
        assert!(player.stats.is_some());

        let fx = &scene.entities[1];
        assert!(fx.particle_emitter.is_some());
        assert_eq!(fx.parent_name.as_deref(), Some("player"));

        let sun = &scene.entities[2];
        assert_eq!(sun.name, "sun");
        assert!(sun.light.is_some());
        let l = sun.light.as_ref().unwrap();
        assert!((l.direction.y - (-1.0)).abs() < 0.001);

        let crate_e = &scene.entities[3];
        assert_eq!(crate_e.name, "crate");
        assert_eq!(crate_e.parent_name.as_deref(), Some("player"));

        // Roundtrip: serialize and re-parse.
        let text = serialize_scene(&scene);
        let scene2 = parse_scene(&text).unwrap();
        assert_eq!(scene2.name, scene.name);
        assert_eq!(scene2.entities.len(), scene.entities.len());
    }

    #[test]
    fn spawn_into_world() {
        let scene = parse_scene(SAMPLE).unwrap();
        let mut world = World::new();
        let map = spawn_scene(&mut world, &scene).unwrap();

        assert_eq!(map.len(), 4);
        let player = map["player"];
        assert!(world.has_component::<Transform>(player));
        assert!(world.has_component::<GlobalTransform>(player));
        assert!(world.has_component::<Velocity>(player));
        assert!(world.has_component::<RigidBody>(player));
        assert!(world.has_component::<SphereCollider>(player));
        assert!(world.has_component::<Camera>(player));
        assert!(world.has_component::<ActiveCamera>(player));
        assert!(world.has_component::<Stats>(player));

        let fx = map["fx"];
        assert!(world.has_component::<ParticleEmitter>(fx));
        let fx_parent = world.get::<Parent>(fx).unwrap();
        assert_eq!(fx_parent.entity, player);

        let sun = map["sun"];
        assert!(world.has_component::<DirectionalLight>(sun));

        let crate_e = map["crate"];
        assert!(world.has_component::<Parent>(crate_e));
        let parent = world.get::<Parent>(crate_e).unwrap();
        assert_eq!(parent.entity, player);
    }

    #[test]
    fn error_on_unknown_component() {
        let bad = r#"entity "e" { foobar }"#;
        assert!(parse_scene(bad).is_err());
    }

    #[test]
    fn empty_scene() {
        let scene = parse_scene("").unwrap();
        assert!(scene.entities.is_empty());
    }

    #[test]
    fn euler_syntax() {
        let src = r#"entity "e" { transform { pos 0 0 0  euler 90 0 0 } }"#;
        let scene = parse_scene(src).unwrap();
        let t = scene.entities[0].transform.as_ref().unwrap();
        // euler 90 0 0 should produce a non-identity rotation.
        assert!((t.rotation.w - 1.0).abs() > 0.01);
    }

    #[test]
    fn runtime_slice_roundtrip_preserves_authored_content() {
        let scene = parse_scene(RUNTIME_SLICE_SAMPLE).unwrap();
        assert_eq!(scene.name, "Island");
        assert_eq!(scene.runtime_slice.terrain.height_scale_meters, 80.0);
        assert_eq!(scene.runtime_slice.terrain.paint_layers.len(), 1);
        assert_eq!(scene.runtime_slice.vegetation_prototypes.len(), 1);
        assert_eq!(
            scene.runtime_slice.static_mesh_actors[0].mesh.as_deref(),
            Some("meshes/rock_large")
        );
        assert_eq!(
            scene.runtime_slice.spawn_points[0].kind,
            SpawnPointKind::Player
        );
        assert!(scene.runtime_slice.patrol_routes[0].looped);
        assert_eq!(scene.runtime_slice.breakable_props[0].break_impulse, 22.0);

        let text = serialize_scene(&scene);
        let roundtrip = parse_scene(&text).unwrap();
        assert_eq!(roundtrip.runtime_slice, scene.runtime_slice);
        assert_eq!(roundtrip.entities.len(), scene.entities.len());
    }

    // ── M11: truncation detection via the format header + sentinel ───────

    /// `SceneData` deliberately does not derive `Debug`, so `unwrap_err()` is
    /// unavailable on a parse result. Extract the error explicitly.
    fn expect_parse_err(src: &str) -> SceneError {
        match parse_scene(src) {
            Err(e) => e,
            Ok(_) => panic!("expected a parse error, but the scene parsed successfully"),
        }
    }

    /// The serializer emits the version header and the terminator, and the
    /// result round-trips through the strict parser.
    #[test]
    fn serialized_scene_carries_header_and_sentinel_and_round_trips() {
        let scene = parse_scene(SAMPLE).unwrap();
        let text = serialize_scene(&scene);

        assert!(
            text.starts_with(&format!("{SCENE_FORMAT_HEADER} {SCENE_FORMAT_VERSION}")),
            "serialized scene must declare the format version first"
        );
        assert!(
            text.trim_end().ends_with(SCENE_END_SENTINEL),
            "serialized scene must close with the terminator"
        );

        let roundtrip = parse_scene(&text).unwrap();
        assert_eq!(roundtrip.entities.len(), scene.entities.len());
    }

    /// THE load-bearing test. A modern scene cut short on an entity boundary
    /// is a typed error — the entities do not silently vanish.
    ///
    /// The second half of this test demonstrates the hazard the header exists
    /// to close: the *identical* truncated text, stripped of its header, parses
    /// happily as a valid smaller scene. That was the pre-M11 behaviour for
    /// every file.
    #[test]
    fn truncated_scene_is_an_error_not_a_silently_smaller_scene() {
        let scene = parse_scene(SAMPLE).unwrap();
        assert!(scene.entities.len() >= 2, "sample needs several entities");
        let text = serialize_scene(&scene);

        // Cut on an entity boundary: right after the first entity's closing
        // brace. This is exactly the shape a torn write produces.
        let first_close = text.find("}\n\n").expect("entity block") + 3;
        let truncated = &text[..first_close];

        // Strict mode (header present): refused.
        let err = expect_parse_err(truncated);
        assert!(
            err.to_string().contains(SCENE_END_SENTINEL),
            "error should name the missing terminator, got: {err}"
        );

        // The hazard, demonstrated: strip the header and the very same bytes
        // parse as a valid — but silently incomplete — scene.
        let header_line = format!("{SCENE_FORMAT_HEADER} {SCENE_FORMAT_VERSION}\n\n");
        let legacy = truncated.strip_prefix(header_line.as_str()).unwrap();
        let silently_smaller = parse_scene(legacy).expect("legacy files stay lenient");
        assert!(
            silently_smaller.entities.len() < scene.entities.len(),
            "this is the silent data loss the sentinel prevents"
        );
    }

    /// Pre-M11 scenes (no header) keep parsing exactly as before — the
    /// sentinel is opt-in on read, so existing content is not orphaned.
    #[test]
    fn legacy_scene_without_header_still_parses() {
        let legacy = "scene \"Old\"\n\nentity \"a\" {\n  transform { pos 1 2 3 }\n}\n";
        let scene = parse_scene(legacy).unwrap();
        assert_eq!(scene.name, "Old");
        assert_eq!(scene.entities.len(), 1);
    }

    /// A legacy file may adopt the terminator without the header; it is
    /// accepted (and still terminates the parse) rather than being a syntax
    /// error.
    #[test]
    fn sentinel_is_accepted_without_header() {
        let text = format!("scene \"X\"\n\n{SCENE_END_SENTINEL}\n");
        let scene = parse_scene(&text).unwrap();
        assert_eq!(scene.name, "X");
    }

    /// Anything after the terminator means the file is not what it claims.
    #[test]
    fn content_after_sentinel_is_rejected() {
        let scene = parse_scene(SAMPLE).unwrap();
        let mut text = serialize_scene(&scene);
        text.push_str("entity \"sneaky\" {\n}\n");
        let err = expect_parse_err(&text);
        assert!(err.to_string().contains("content after"), "got: {err}");
    }

    /// A future format version is refused loudly rather than mis-parsed.
    #[test]
    fn unsupported_scene_format_version_is_rejected() {
        let text = format!("{SCENE_FORMAT_HEADER} 99\n\nscene \"X\"\n\n{SCENE_END_SENTINEL}\n");
        let err = expect_parse_err(&text);
        assert!(
            err.to_string().contains("unsupported scene format version 99"),
            "got: {err}"
        );
    }

    /// A fractional version must be rejected, not truncated to 2. `f32 as u32`
    /// truncates toward zero, so a bare cast would silently accept `2.5` as the
    /// current version — a version the engine does not actually understand.
    #[test]
    fn fractional_scene_format_version_is_rejected() {
        for v in ["2.5", "2.999", "1.5"] {
            let text = format!("{SCENE_FORMAT_HEADER} {v}\n\nscene \"X\"\n\n{SCENE_END_SENTINEL}\n");
            let err = expect_parse_err(&text);
            assert!(
                err.to_string().contains("non-negative integer"),
                "version {v} must be rejected as non-integer, got: {err}"
            );
        }
        // The exact integer still works.
        let ok = format!("{SCENE_FORMAT_HEADER} 2\n\nscene \"X\"\n\n{SCENE_END_SENTINEL}\n");
        assert!(parse_scene(&ok).is_ok(), "integer version 2 must still parse");
    }

    /// Integer fields keep FULL precision through a round trip. A u32 seed
    /// above 2^24 (which most random seeds are) would not survive being
    /// serialized/parsed through f32 — a determinism hazard, since the seed is
    /// exactly what must reproduce a recorded run (ADR 0005). Written directly
    /// as an integer and read via Token::Integer, it round-trips exactly.
    #[test]
    fn large_integer_fields_round_trip_exactly() {
        // 3_000_000_003 is > 2^24 and is NOT representable exactly in f32
        // (nearest f32 differs by ~256), so an f32 round trip would corrupt it.
        let seed = 3_000_000_003u32;
        let max_particles = 20_000_001u32; // also > 2^24
        let src = format!(
            "scene \"S\"\n\
             entity \"fx\" {{\n  \
               particle_emitter {{ max_particles {max_particles} seed {seed} }}\n\
             }}\n"
        );
        let scene = parse_scene(&src).unwrap();
        let emitter = scene.entities[0].particle_emitter.as_ref().unwrap();
        assert_eq!(emitter.seed, seed, "seed must parse exactly");
        assert_eq!(emitter.max_particles, max_particles);

        // And survive a full serialize -> parse cycle.
        let round = parse_scene(&serialize_scene(&scene)).unwrap();
        let round_emitter = round.entities[0].particle_emitter.as_ref().unwrap();
        assert_eq!(round_emitter.seed, seed, "seed must survive serialization");
        assert_eq!(round_emitter.max_particles, max_particles);
    }

    /// A fractional value in an integer field is a typed error, not a silent
    /// truncation (`seed 42.9` must NOT become 42).
    #[test]
    fn fractional_integer_field_is_rejected() {
        let src = "scene \"S\"\n\
                   entity \"fx\" {\n  particle_emitter { seed 42.9 }\n}\n";
        let err = expect_parse_err(src);
        assert!(
            err.to_string().contains("non-negative integer"),
            "a fractional seed must be rejected, got: {err}"
        );
    }

    /// An integer field written with a trailing `.0` (integer-valued float) is
    /// still accepted — back-compat for anything the old serializer emitted.
    #[test]
    fn integer_valued_float_is_accepted_in_integer_field() {
        let src = "scene \"S\"\n\
                   entity \"fx\" {\n  particle_emitter { seed 128.0 }\n}\n";
        let scene = parse_scene(src).unwrap();
        assert_eq!(
            scene.entities[0].particle_emitter.as_ref().unwrap().seed,
            128
        );
    }

    // ── M11: the authored save path is atomic ────────────────────────────

    fn scene_test_dir(label: &str) -> std::path::PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "hold_scene_save_{}_{}_{}",
            label,
            std::process::id(),
            nonce
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// `save_scene` writes through the atomic primitive: the file round-trips,
    /// and a successful save leaves no temp litter beside it.
    #[test]
    fn save_scene_round_trips_and_leaves_no_temp_litter() {
        let dir = scene_test_dir("roundtrip");
        let path = dir.join("level.hsc");

        let scene = parse_scene(SAMPLE).unwrap();
        save_scene(&path, &scene).unwrap();

        let reloaded = load_scene(&path).unwrap();
        assert_eq!(reloaded.name, scene.name);
        assert_eq!(reloaded.entities.len(), scene.entities.len());

        let temps = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with(crate::platform::atomic_write::TEMP_PREFIX)
            })
            .count();
        assert_eq!(temps, 0, "a successful save must leave no temp file");
        std::fs::remove_dir_all(dir).ok();
    }

    /// Overwriting an existing scene never leaves the destination truncated:
    /// Saving a smaller scene over a larger one leaves the file holding exactly
    /// the new content (no stale bytes), and it reloads correctly.
    ///
    /// Note: this checks the SUCCESS-path result, which a plain truncating
    /// `fs::write` would also satisfy (`File::create` truncates, so it does not
    /// leave trailing bytes either). The atomicity that distinguishes
    /// `save_scene` — old-or-new under a crash, and no temp litter — is proven
    /// in `platform::atomic_write` (the fault-injection suite) and by
    /// `save_scene_round_trips_and_leaves_no_temp_litter`. This test guards the
    /// plumbing: that the smaller document round-trips through the real save
    /// path without corruption.
    #[test]
    fn save_scene_over_existing_file_round_trips_smaller_content() {
        let dir = scene_test_dir("overwrite");
        let path = dir.join("level.hsc");

        let big = parse_scene(SAMPLE).unwrap();
        save_scene(&path, &big).unwrap();
        let small = parse_scene("scene \"Tiny\"\n").unwrap();
        save_scene(&path, &small).unwrap();

        let reloaded = load_scene(&path).unwrap();
        assert_eq!(reloaded.name, "Tiny");
        assert!(reloaded.entities.is_empty());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), serialize_scene(&small));
        std::fs::remove_dir_all(dir).ok();
    }

    /// A save into a directory that does not exist surfaces a typed
    /// AtomicWriteError naming the failing phase — not a stringified
    /// `SceneError { line: 0, msg: "IO: ..." }` (the pre-M11 behaviour, which
    /// made a disk failure indistinguishable from a parse failure).
    #[test]
    fn save_scene_surfaces_typed_write_error() {
        let dir = scene_test_dir("typed_err");
        let path = dir.join("missing").join("level.hsc");
        let scene = parse_scene(SAMPLE).unwrap();
        match save_scene(&path, &scene) {
            Err(AtomicWriteError::TempCreate { win32_error, .. }) => {
                assert_ne!(win32_error, 0);
            }
            other => panic!("expected typed TempCreate error, got {other:?}"),
        }
        std::fs::remove_dir_all(dir).ok();
    }
}
