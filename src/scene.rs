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
//! Supports: transform, camera, active_camera, light, stats, parent.
//! Comments start with `#`.  Quoted strings use `"double quotes"`.

use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::path::Path;

use crate::ecs::components::{
    ActiveCamera, Camera, DirectionalLight, Stats, Transform,
};
use crate::ecs::Entity;
use crate::ecs::World;
use crate::math::{Quat, Vec3};
use crate::renderer::Color;

// ═══════════════════════════════════════════════════════════════════════════════
//  Error
// ═══════════════════════════════════════════════════════════════════════════════

#[derive(Debug)]
pub struct SceneError {
    pub line: usize,
    pub msg:  String,
}

impl SceneError {
    fn new(line: usize, msg: impl Into<String>) -> Self {
        Self { line, msg: msg.into() }
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
    pub name:     String,
    pub entities: Vec<EntityDef>,
}

/// One entity definition from the scene file.
pub struct EntityDef {
    pub name:          String,
    pub transform:     Option<TransformDef>,
    pub camera:        Option<CameraDef>,
    pub active_camera: bool,
    pub light:         Option<LightDef>,
    pub stats:         Option<StatsDef>,
    pub parent_name:   Option<String>,
}

pub struct TransformDef {
    pub position: Vec3,
    pub rotation: Quat,
    pub scale:    Vec3,
}

pub struct CameraDef {
    pub fov:  f32,
    pub near: f32,
    pub far:  f32,
}

pub struct LightDef {
    pub direction: Vec3,
    pub color:     Color,
    pub intensity: f32,
    pub ambient:   f32,
}

pub struct StatsDef {
    pub health:  f32,
    pub mana:    f32,
    pub attack:  f32,
    pub defense: f32,
    pub speed:   f32,
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Tokenizer
// ═══════════════════════════════════════════════════════════════════════════════

#[derive(Debug, PartialEq)]
enum Token<'a> {
    Word(&'a str),
    Str(&'a str),     // contents without quotes
    Number(f32),
    LBrace,
    RBrace,
}

struct Lexer<'a> {
    src:  &'a str,
    pos:  usize,
    line: usize,
}

impl<'a> Lexer<'a> {
    fn new(src: &'a str) -> Self {
        Self { src, pos: 0, line: 1 }
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
            b'{' => { self.pos += 1; Ok(Some(Token::LBrace)) }
            b'}' => { self.pos += 1; Ok(Some(Token::RBrace)) }
            b'"' => self.read_string(),
            _ if c == b'-' || c == b'+' || c == b'.' || c.is_ascii_digit() => {
                // Could be a number or a word starting with a digit.
                self.read_number_or_word()
            }
            _ if c.is_ascii_alphabetic() || c == b'_' => self.read_word(),
            _ => Err(SceneError::new(self.line, format!("unexpected character '{}'", c as char))),
        }
    }

    fn read_string(&mut self) -> Result<Option<Token<'a>>, SceneError> {
        let start_line = self.line;
        self.pos += 1; // skip opening "
        let start = self.pos;
        while self.pos < self.src.len() && self.src.as_bytes()[self.pos] != b'"' {
            if self.src.as_bytes()[self.pos] == b'\n' { self.line += 1; }
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
            let val: f32 = slice.parse().map_err(|_| {
                SceneError::new(self.line, format!("invalid number: {slice}"))
            })?;
            Ok(Some(Token::Number(val)))
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
    lex:     Lexer<'a>,
    peeked:  Option<Token<'a>>,
}

impl<'a> Parser<'a> {
    fn new(src: &'a str) -> Self {
        Self { lex: Lexer::new(src), peeked: None }
    }

    fn line(&self) -> usize { self.lex.line }

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

    fn expect_word(&mut self, expected: &str) -> Result<(), SceneError> {
        match self.next()? {
            Some(Token::Word(w)) if w == expected => Ok(()),
            _ => Err(SceneError::new(
                self.line(),
                format!("expected '{expected}', got unexpected token"),
            )),
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
            _ => Err(SceneError::new(
                self.line(),
                format!("expected number, got unexpected token"),
            )),
        }
    }

    // ── Top-level ─────────────────────────────────────────────────────────

    fn parse_scene(&mut self) -> Result<SceneData, SceneError> {
        let mut scene = SceneData {
            name:     String::new(),
            entities: Vec::new(),
        };

        // Optional "scene" header.
        if let Some(Token::Word("scene")) = self.peek()? {
            self.next()?; // consume "scene"
            scene.name = self.expect_str()?;
        }

        while self.peek()?.is_some() {
            self.expect_word("entity")?;
            scene.entities.push(self.parse_entity()?);
        }

        Ok(scene)
    }

    fn parse_entity(&mut self) -> Result<EntityDef, SceneError> {
        let name = self.expect_str()?;
        self.expect_lbrace()?;

        let mut def = EntityDef {
            name,
            transform:     None,
            camera:        None,
            active_camera: false,
            light:         None,
            stats:         None,
            parent_name:   None,
        };

        loop {
            let _ln = self.lex.line;
            match self.peek()? {
                Some(Token::RBrace) => { self.next()?; break; }
                Some(Token::Word(_)) => {
                    // Peek the word to decide which component.
                    if let Some(Token::Word(w)) = self.next()? {
                        match w {
                            "transform"     => def.transform     = Some(self.parse_transform()?),
                            "camera"        => def.camera        = Some(self.parse_camera()?),
                            "active_camera" => def.active_camera = true,
                            "light"         => def.light         = Some(self.parse_light()?),
                            "stats"         => def.stats         = Some(self.parse_stats()?),
                            "parent"        => def.parent_name   = Some(self.expect_str()?),
                            _ => {
                                return Err(SceneError::new(
                                    _ln,
                                    format!("unknown component"),
                                ));
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
            scale:    Vec3::ONE,
        };

        loop {
            let _ln = self.lex.line;
            match self.peek()? {
                Some(Token::RBrace) => { self.next()?; break; }
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
                                    deg_to_rad(x), deg_to_rad(y), deg_to_rad(z),
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
            fov:  60.0,
            near: 0.1,
            far:  1000.0,
        };

        loop {
            let _ln = self.lex.line;
            match self.peek()? {
                Some(Token::RBrace) => { self.next()?; break; }
                Some(Token::Word(_)) => {
                    if let Some(Token::Word(key)) = self.next()? {
                        match key {
                            "fov"  => c.fov  = self.expect_number()?,
                            "near" => c.near = self.expect_number()?,
                            "far"  => c.far  = self.expect_number()?,
                            _other  => {
                                return Err(SceneError::new(
                                    _ln,
                                    format!("unknown camera field"),
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

        Ok(c)
    }

    fn parse_light(&mut self) -> Result<LightDef, SceneError> {
        self.expect_lbrace()?;

        let mut l = LightDef {
            direction: Vec3::new(0.0, -1.0, 0.0),
            color:     Color::WHITE,
            intensity: 1.0,
            ambient:   0.15,
        };

        loop {
            let _ln = self.lex.line;
            match self.peek()? {
                Some(Token::RBrace) => { self.next()?; break; }
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
                            "ambient"   => l.ambient   = self.expect_number()?,
                            _ => {
                                return Err(SceneError::new(
                                    _ln,
                                    format!("unknown light field"),
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

        Ok(l)
    }

    fn parse_stats(&mut self) -> Result<StatsDef, SceneError> {
        self.expect_lbrace()?;

        let mut s = StatsDef {
            health:  100.0,
            mana:    0.0,
            attack:  0.0,
            defense: 0.0,
            speed:   0.0,
        };

        loop {
            let _ln = self.lex.line;
            match self.peek()? {
                Some(Token::RBrace) => { self.next()?; break; }
                Some(Token::Word(_)) => {
                    if let Some(Token::Word(key)) = self.next()? {
                        match key {
                            "health"  => s.health  = self.expect_number()?,
                            "mana"    => s.mana    = self.expect_number()?,
                            "attack"  => s.attack  = self.expect_number()?,
                            "defense" => s.defense = self.expect_number()?,
                            "speed"   => s.speed   = self.expect_number()?,
                            _other     => {
                                return Err(SceneError::new(
                                    _ln,
                                    format!("unknown stats field"),
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
            world.add_component(entity, Transform {
                position: t.position,
                rotation: t.rotation,
                scale:    t.scale,
            });
        }

        if let Some(c) = &def.camera {
            world.add_component(entity, Camera::perspective(c.fov, 16.0 / 9.0, c.near, c.far));
        }

        if def.active_camera {
            world.add_component(entity, ActiveCamera);
        }

        if let Some(l) = &def.light {
            world.add_component(entity, DirectionalLight {
                direction: l.direction.normalize(),
                color:     l.color,
                intensity: l.intensity,
                ambient:   l.ambient,
            });
        }

        if let Some(s) = &def.stats {
            world.add_component(entity, Stats::new(
                s.health, s.mana, s.attack, s.defense, s.speed,
            ));
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
            let parent = name_map.get(parent_name.as_str()).copied().ok_or_else(|| {
                SceneError::new(0, format!("parent '{}' not found", parent_name))
            })?;
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

    if !scene.name.is_empty() {
        out.push_str(&format!("scene \"{}\"\n\n", scene.name));
    }

    for def in &scene.entities {
        out.push_str(&format!("entity \"{}\" {{\n", def.name));

        if let Some(t) = &def.transform {
            out.push_str("  transform {\n");
            out.push_str(&format!(
                "    pos {} {} {}\n",
                fmt_f32(t.position.x), fmt_f32(t.position.y), fmt_f32(t.position.z),
            ));
            out.push_str(&format!(
                "    rot {} {} {} {}\n",
                fmt_f32(t.rotation.x), fmt_f32(t.rotation.y),
                fmt_f32(t.rotation.z), fmt_f32(t.rotation.w),
            ));
            out.push_str(&format!(
                "    scale {} {} {}\n",
                fmt_f32(t.scale.x), fmt_f32(t.scale.y), fmt_f32(t.scale.z),
            ));
            out.push_str("  }\n");
        }

        if let Some(c) = &def.camera {
            out.push_str(&format!(
                "  camera {{ fov {} near {} far {} }}\n",
                fmt_f32(c.fov), fmt_f32(c.near), fmt_f32(c.far),
            ));
        }

        if def.active_camera {
            out.push_str("  active_camera\n");
        }

        if let Some(l) = &def.light {
            out.push_str("  light {\n");
            out.push_str(&format!(
                "    dir {} {} {}\n",
                fmt_f32(l.direction.x), fmt_f32(l.direction.y), fmt_f32(l.direction.z),
            ));
            out.push_str(&format!(
                "    color {} {} {} {}\n",
                fmt_f32(l.color.r), fmt_f32(l.color.g),
                fmt_f32(l.color.b), fmt_f32(l.color.a),
            ));
            out.push_str(&format!("    intensity {}\n", fmt_f32(l.intensity)));
            out.push_str(&format!("    ambient {}\n", fmt_f32(l.ambient)));
            out.push_str("  }\n");
        }

        if let Some(s) = &def.stats {
            out.push_str(&format!(
                "  stats {{ health {} mana {} attack {} defense {} speed {} }}\n",
                fmt_f32(s.health), fmt_f32(s.mana), fmt_f32(s.attack),
                fmt_f32(s.defense), fmt_f32(s.speed),
            ));
        }

        if let Some(p) = &def.parent_name {
            out.push_str(&format!("  parent \"{}\"\n", p));
        }

        out.push_str("}\n\n");
    }

    out
}

/// Save a scene to a file.
pub fn save_scene<P: AsRef<Path>>(path: P, scene: &SceneData) -> Result<(), SceneError> {
    let text = serialize_scene(scene);
    fs::write(path, text).map_err(|e| SceneError::new(0, format!("IO: {e}")))
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
  camera { fov 60 near 0.1 far 1000 }
  active_camera
  stats { health 100 mana 50 attack 10 defense 5 speed 3 }
}

entity "sun" {
  light { dir 0.5 -1 0.3  color 1 1 1 1  intensity 1  ambient 0.15 }
}

entity "crate" {
  transform { pos 5 0 -3 }
  parent "player"
}
"#;

    #[test]
    fn parse_roundtrip() {
        let scene = parse_scene(SAMPLE).unwrap();
        assert_eq!(scene.name, "Test Level");
        assert_eq!(scene.entities.len(), 3);

        let player = &scene.entities[0];
        assert_eq!(player.name, "player");
        assert!(player.transform.is_some());
        assert!(player.camera.is_some());
        assert!(player.active_camera);
        assert!(player.stats.is_some());

        let sun = &scene.entities[1];
        assert_eq!(sun.name, "sun");
        assert!(sun.light.is_some());
        let l = sun.light.as_ref().unwrap();
        assert!((l.direction.y - (-1.0)).abs() < 0.001);

        let crate_e = &scene.entities[2];
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

        assert_eq!(map.len(), 3);
        let player = map["player"];
        assert!(world.has_component::<Transform>(player));
        assert!(world.has_component::<Camera>(player));
        assert!(world.has_component::<ActiveCamera>(player));
        assert!(world.has_component::<Stats>(player));

        let sun = map["sun"];
        assert!(world.has_component::<DirectionalLight>(sun));

        let crate_e = map["crate"];
        assert!(world.has_component::<Parent>(crate_e));
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
}
