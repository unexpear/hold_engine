import { MorphicObject } from "./MorphicObject.js";
import { MorphicTree } from "./MorphicTree.js";

const IDENTITY_ROTATION = Object.freeze({ x: 0, y: 0, z: 0, w: 1 });
const UNIT_SCALE = Object.freeze({ x: 1, y: 1, z: 1 });
const ZERO_VEC3 = Object.freeze({ x: 0, y: 0, z: 0 });
const WHITE = Object.freeze({ r: 1, g: 1, b: 1, a: 1 });
const DEFAULT_TERRAIN = Object.freeze({
  sizeMeters: Object.freeze({ x: 1024, y: 256, z: 1024 }),
  chunkSizeMeters: 64,
  heightScaleMeters: 64,
});
const DEFAULT_PARTICLE_EMITTER = Object.freeze({
  emitting: true,
  ratePerSecond: 40,
  maxParticles: 160,
  spawnRadius: 0.15,
  direction: Object.freeze({ x: 0, y: 1, z: 0 }),
  spread: 0.45,
  speedMin: 2.5,
  speedMax: 5.5,
  lifetimeMin: 0.4,
  lifetimeMax: 1.1,
  gravityScale: 0.9,
  linearDamping: 1.6,
  startSize: 0.18,
  endSize: 0.02,
  startColor: Object.freeze({ r: 1, g: 0.85, b: 0.25, a: 1 }),
  endColor: Object.freeze({ r: 1, g: 0.2, b: 0.05, a: 0 }),
  seed: 1,
});

function toNumber(value, fallback = 0) {
  const number = Number(value);
  return Number.isFinite(number) ? number : fallback;
}

function toInteger(value, fallback = 0) {
  return Math.trunc(toNumber(value, fallback));
}

function toBooleanValue(value, fallback = false) {
  if (typeof value === "boolean") {
    return value;
  }
  if (typeof value === "number") {
    return value > 0;
  }
  if (typeof value === "string") {
    const normalized = value.trim().toLowerCase();
    if (normalized === "true") {
      return true;
    }
    if (normalized === "false") {
      return false;
    }
    if (normalized === "1") {
      return true;
    }
    if (normalized === "0") {
      return false;
    }
  }
  return fallback;
}

function normalizeOptionalString(value) {
  if (value == null) {
    return null;
  }
  return String(value);
}

function normalizeNamedString(value, fallback) {
  return normalizeOptionalString(value) ?? fallback;
}

function cloneVec3(value = {}, fallback = ZERO_VEC3) {
  const source = Array.isArray(value)
    ? { x: value[0], y: value[1], z: value[2] }
    : (value ?? {});
  return {
    x: toNumber(source.x, fallback.x ?? 0),
    y: toNumber(source.y, fallback.y ?? 0),
    z: toNumber(source.z, fallback.z ?? 0),
  };
}

function cloneQuat(value = {}, fallback = IDENTITY_ROTATION) {
  const source = Array.isArray(value)
    ? { x: value[0], y: value[1], z: value[2], w: value[3] }
    : (value ?? {});
  return {
    x: toNumber(source.x, fallback.x),
    y: toNumber(source.y, fallback.y),
    z: toNumber(source.z, fallback.z),
    w: toNumber(source.w, fallback.w),
  };
}

function cloneColor(value = {}, fallback = WHITE) {
  const source = Array.isArray(value)
    ? { r: value[0], g: value[1], b: value[2], a: value[3] }
    : (value ?? {});
  return {
    r: toNumber(source.r, fallback.r ?? 1),
    g: toNumber(source.g, fallback.g ?? 1),
    b: toNumber(source.b, fallback.b ?? 1),
    a: toNumber(source.a, fallback.a ?? 1),
  };
}

function normalizeTransform(transform, fallbackPosition = ZERO_VEC3) {
  return {
    position: cloneVec3(transform?.position, {
      x: fallbackPosition.x ?? 0,
      y: fallbackPosition.y ?? 0,
      z: fallbackPosition.z ?? 0,
    }),
    rotation: cloneQuat(transform?.rotation, IDENTITY_ROTATION),
    scale: cloneVec3(transform?.scale, UNIT_SCALE),
  };
}

function normalizeVelocity(velocity) {
  if (!velocity) {
    return null;
  }

  return {
    linear: cloneVec3(velocity.linear, ZERO_VEC3),
    angular: cloneVec3(velocity.angular, ZERO_VEC3),
  };
}

function normalizeRigidBody(rigidBody) {
  if (!rigidBody) {
    return null;
  }

  return {
    gravityScale: toNumber(rigidBody.gravityScale ?? rigidBody.gravity_scale, 1),
    linearDamping: toNumber(rigidBody.linearDamping ?? rigidBody.linear_damping, 0),
    angularDamping: toNumber(rigidBody.angularDamping ?? rigidBody.angular_damping, 0),
    restitution: toNumber(rigidBody.restitution, 0.5),
  };
}

function normalizeSphereCollider(sphereCollider) {
  if (!sphereCollider) {
    return null;
  }

  return {
    radius: toNumber(sphereCollider.radius, 0.5),
  };
}

function normalizeParticleEmitter(particleEmitter) {
  if (!particleEmitter) {
    return null;
  }

  return {
    emitting: toBooleanValue(particleEmitter.emitting, DEFAULT_PARTICLE_EMITTER.emitting),
    ratePerSecond: toNumber(
      particleEmitter.ratePerSecond ?? particleEmitter.rate_per_second,
      DEFAULT_PARTICLE_EMITTER.ratePerSecond,
    ),
    maxParticles: Math.max(
      0,
      toInteger(
        particleEmitter.maxParticles ?? particleEmitter.max_particles,
        DEFAULT_PARTICLE_EMITTER.maxParticles,
      ),
    ),
    spawnRadius: toNumber(
      particleEmitter.spawnRadius ?? particleEmitter.spawn_radius,
      DEFAULT_PARTICLE_EMITTER.spawnRadius,
    ),
    direction: cloneVec3(
      particleEmitter.direction,
      DEFAULT_PARTICLE_EMITTER.direction,
    ),
    spread: toNumber(particleEmitter.spread, DEFAULT_PARTICLE_EMITTER.spread),
    speedMin: toNumber(
      particleEmitter.speedMin ?? particleEmitter.speed_min,
      DEFAULT_PARTICLE_EMITTER.speedMin,
    ),
    speedMax: toNumber(
      particleEmitter.speedMax ?? particleEmitter.speed_max,
      DEFAULT_PARTICLE_EMITTER.speedMax,
    ),
    lifetimeMin: toNumber(
      particleEmitter.lifetimeMin ?? particleEmitter.lifetime_min,
      DEFAULT_PARTICLE_EMITTER.lifetimeMin,
    ),
    lifetimeMax: toNumber(
      particleEmitter.lifetimeMax ?? particleEmitter.lifetime_max,
      DEFAULT_PARTICLE_EMITTER.lifetimeMax,
    ),
    gravityScale: toNumber(
      particleEmitter.gravityScale ?? particleEmitter.gravity_scale,
      DEFAULT_PARTICLE_EMITTER.gravityScale,
    ),
    linearDamping: toNumber(
      particleEmitter.linearDamping ?? particleEmitter.linear_damping,
      DEFAULT_PARTICLE_EMITTER.linearDamping,
    ),
    startSize: toNumber(
      particleEmitter.startSize ?? particleEmitter.start_size,
      DEFAULT_PARTICLE_EMITTER.startSize,
    ),
    endSize: toNumber(
      particleEmitter.endSize ?? particleEmitter.end_size,
      DEFAULT_PARTICLE_EMITTER.endSize,
    ),
    startColor: cloneColor(
      particleEmitter.startColor ?? particleEmitter.start_color,
      DEFAULT_PARTICLE_EMITTER.startColor,
    ),
    endColor: cloneColor(
      particleEmitter.endColor ?? particleEmitter.end_color,
      DEFAULT_PARTICLE_EMITTER.endColor,
    ),
    seed: Math.max(0, toInteger(particleEmitter.seed, DEFAULT_PARTICLE_EMITTER.seed)),
  };
}

function normalizeCamera(camera) {
  if (!camera) {
    return null;
  }

  return {
    fov: toNumber(camera.fov, 60),
    near: toNumber(camera.near, 0.1),
    far: toNumber(camera.far, 1000),
  };
}

function normalizeLight(light) {
  if (!light) {
    return null;
  }

  return {
    direction: cloneVec3(light.direction, { x: 0, y: -1, z: 0 }),
    color: cloneColor(light.color, WHITE),
    intensity: toNumber(light.intensity, 1),
    ambient: toNumber(light.ambient, 0.15),
  };
}

function normalizeStats(stats) {
  if (!stats) {
    return null;
  }

  return {
    health: toNumber(stats.health, 100),
    mana: toNumber(stats.mana, 0),
    attack: toNumber(stats.attack, 0),
    defense: toNumber(stats.defense, 0),
    speed: toNumber(stats.speed, 0),
  };
}

function normalizeTerrainPaintLayer(layer) {
  return {
    name: normalizeNamedString(layer?.name, "paint_layer"),
    material: normalizeOptionalString(layer?.material),
    tiling: toNumber(layer?.tiling, 1),
    weightmapChannel: Math.max(
      0,
      toInteger(layer?.weightmapChannel ?? layer?.weightmap_channel, 0),
    ),
  };
}

function normalizeTerrain(terrain) {
  const source = terrain ?? {};
  const sizeMeters = cloneVec3(
    source.sizeMeters ?? source.size_meters,
    DEFAULT_TERRAIN.sizeMeters,
  );
  const paintLayers = source.paintLayers ?? source.paint_layers;

  return {
    sizeMeters,
    chunkSizeMeters: Math.max(
      0,
      toInteger(
        source.chunkSizeMeters ?? source.chunk_size_meters,
        DEFAULT_TERRAIN.chunkSizeMeters,
      ),
    ),
    heightScaleMeters: toNumber(
      source.heightScaleMeters ?? source.height_scale_meters,
      DEFAULT_TERRAIN.heightScaleMeters,
    ),
    paintLayers: Array.isArray(paintLayers)
      ? paintLayers.map(normalizeTerrainPaintLayer)
      : [],
  };
}

function normalizeVegetationPrototype(prototype) {
  return {
    name: normalizeNamedString(prototype?.name, "vegetation_prototype"),
    mesh: normalizeOptionalString(prototype?.mesh),
    material: normalizeOptionalString(prototype?.material),
    densityPer100m2: toNumber(
      prototype?.densityPer100m2 ?? prototype?.density_per_100m2 ?? prototype?.density,
      1,
    ),
    randomYaw: toBooleanValue(prototype?.randomYaw ?? prototype?.random_yaw, true),
    alignToNormal: toBooleanValue(
      prototype?.alignToNormal ?? prototype?.align_to_normal,
      true,
    ),
  };
}

function normalizeVegetationPaint(stroke) {
  return {
    prototype: normalizeNamedString(stroke?.prototype, "vegetation"),
    center: cloneVec3(stroke?.center, ZERO_VEC3),
    radius: toNumber(stroke?.radius, 1),
    density: toNumber(stroke?.density, 1),
  };
}

function normalizeStaticMeshActor(actor) {
  return {
    name: normalizeNamedString(actor?.name, "static_mesh_actor"),
    mesh: normalizeOptionalString(actor?.mesh),
    material: normalizeOptionalString(actor?.material),
    position: cloneVec3(actor?.position, ZERO_VEC3),
    rotationEulerDeg: cloneVec3(
      actor?.rotationEulerDeg ?? actor?.rotation_euler_deg,
      ZERO_VEC3,
    ),
    scale: cloneVec3(actor?.scale, UNIT_SCALE),
  };
}

function normalizeWaterVolume(volume) {
  return {
    name: normalizeNamedString(volume?.name, "water_volume"),
    material: normalizeOptionalString(volume?.material),
    center: cloneVec3(volume?.center, ZERO_VEC3),
    extents: cloneVec3(volume?.extents, UNIT_SCALE),
    waveStrength: toNumber(volume?.waveStrength ?? volume?.wave_strength, 0),
  };
}

function normalizeSpawnPointKind(kind) {
  const normalized = String(kind ?? "generic").trim().toLowerCase();
  if (normalized === "player" || normalized === "enemy" || normalized === "vehicle") {
    return normalized;
  }
  return "generic";
}

function normalizeSpawnPoint(spawnPoint) {
  return {
    name: normalizeNamedString(spawnPoint?.name, "spawn_point"),
    kind: normalizeSpawnPointKind(spawnPoint?.kind),
    position: cloneVec3(spawnPoint?.position, ZERO_VEC3),
    facingYawDeg: toNumber(
      spawnPoint?.facingYawDeg ?? spawnPoint?.facing_yaw_deg ?? spawnPoint?.yaw,
      0,
    ),
  };
}

function normalizePatrolRoute(route) {
  return {
    name: normalizeNamedString(route?.name, "patrol_route"),
    looped: toBooleanValue(route?.looped, false),
    points: Array.isArray(route?.points) ? route.points.map((point) => cloneVec3(point, ZERO_VEC3)) : [],
  };
}

function normalizeVehicleSpawner(spawner) {
  return {
    name: normalizeNamedString(spawner?.name, "vehicle_spawner"),
    mesh: normalizeOptionalString(spawner?.mesh),
    material: normalizeOptionalString(spawner?.material),
    position: cloneVec3(spawner?.position, ZERO_VEC3),
    facingYawDeg: toNumber(
      spawner?.facingYawDeg ?? spawner?.facing_yaw_deg ?? spawner?.yaw,
      0,
    ),
  };
}

function normalizeBreakableProp(prop) {
  return {
    name: normalizeNamedString(prop?.name, "breakable_prop"),
    intactMesh: normalizeOptionalString(prop?.intactMesh ?? prop?.intact_mesh),
    brokenMesh: normalizeOptionalString(prop?.brokenMesh ?? prop?.broken_mesh),
    material: normalizeOptionalString(prop?.material),
    position: cloneVec3(prop?.position, ZERO_VEC3),
    rotationEulerDeg: cloneVec3(
      prop?.rotationEulerDeg ?? prop?.rotation_euler_deg,
      ZERO_VEC3,
    ),
    scale: cloneVec3(prop?.scale, UNIT_SCALE),
    hitPoints: toNumber(prop?.hitPoints ?? prop?.hit_points, 100),
    breakImpulse: toNumber(prop?.breakImpulse ?? prop?.break_impulse, 0),
  };
}

function normalizeRuntimeSlice(runtimeSlice) {
  const source = runtimeSlice ?? {};
  return {
    terrain: normalizeTerrain(source.terrain),
    vegetationPrototypes: Array.isArray(source.vegetationPrototypes ?? source.vegetation_prototypes)
      ? (source.vegetationPrototypes ?? source.vegetation_prototypes).map(normalizeVegetationPrototype)
      : [],
    vegetationPaints: Array.isArray(source.vegetationPaints ?? source.vegetation_paints)
      ? (source.vegetationPaints ?? source.vegetation_paints).map(normalizeVegetationPaint)
      : [],
    staticMeshActors: Array.isArray(source.staticMeshActors ?? source.static_mesh_actors)
      ? (source.staticMeshActors ?? source.static_mesh_actors).map(normalizeStaticMeshActor)
      : [],
    waterVolumes: Array.isArray(source.waterVolumes ?? source.water_volumes)
      ? (source.waterVolumes ?? source.water_volumes).map(normalizeWaterVolume)
      : [],
    spawnPoints: Array.isArray(source.spawnPoints ?? source.spawn_points)
      ? (source.spawnPoints ?? source.spawn_points).map(normalizeSpawnPoint)
      : [],
    patrolRoutes: Array.isArray(source.patrolRoutes ?? source.patrol_routes)
      ? (source.patrolRoutes ?? source.patrol_routes).map(normalizePatrolRoute)
      : [],
    vehicleSpawners: Array.isArray(source.vehicleSpawners ?? source.vehicle_spawners)
      ? (source.vehicleSpawners ?? source.vehicle_spawners).map(normalizeVehicleSpawner)
      : [],
    breakableProps: Array.isArray(source.breakableProps ?? source.breakable_props)
      ? (source.breakableProps ?? source.breakable_props).map(normalizeBreakableProp)
      : [],
  };
}

function normalizeSceneEntity(entity) {
  return {
    name: normalizeNamedString(entity?.name, "Entity"),
    transform: entity?.transform ? normalizeTransform(entity.transform) : null,
    velocity: normalizeVelocity(entity?.velocity),
    rigidBody: normalizeRigidBody(entity?.rigidBody ?? entity?.rigid_body),
    sphereCollider: normalizeSphereCollider(entity?.sphereCollider ?? entity?.sphere_collider),
    particleEmitter: normalizeParticleEmitter(
      entity?.particleEmitter ?? entity?.particle_emitter,
    ),
    camera: normalizeCamera(entity?.camera),
    activeCamera: toBooleanValue(entity?.activeCamera ?? entity?.active_camera, false),
    light: normalizeLight(entity?.light),
    stats: normalizeStats(entity?.stats),
    parentName: normalizeOptionalString(entity?.parentName ?? entity?.parent_name),
  };
}

function isDefaultTerrain(terrain) {
  return (
    terrain.sizeMeters.x === DEFAULT_TERRAIN.sizeMeters.x &&
    terrain.sizeMeters.y === DEFAULT_TERRAIN.sizeMeters.y &&
    terrain.sizeMeters.z === DEFAULT_TERRAIN.sizeMeters.z &&
    terrain.chunkSizeMeters === DEFAULT_TERRAIN.chunkSizeMeters &&
    terrain.heightScaleMeters === DEFAULT_TERRAIN.heightScaleMeters &&
    terrain.paintLayers.length === 0
  );
}

function isRuntimeSliceEmpty(runtimeSlice) {
  const slice = normalizeRuntimeSlice(runtimeSlice);
  return (
    isDefaultTerrain(slice.terrain) &&
    slice.vegetationPrototypes.length === 0 &&
    slice.vegetationPaints.length === 0 &&
    slice.staticMeshActors.length === 0 &&
    slice.waterVolumes.length === 0 &&
    slice.spawnPoints.length === 0 &&
    slice.patrolRoutes.length === 0 &&
    slice.vehicleSpawners.length === 0 &&
    slice.breakableProps.length === 0
  );
}

function sceneMetadataForObject(object) {
  const scene = object.metadata?.scene ?? {};
  return {
    transform: normalizeTransform(scene.transform, {
      x: object.position?.x ?? 0,
      y: object.position?.y ?? 0,
      z: 0,
    }),
    velocity: normalizeVelocity(scene.velocity),
    rigidBody: normalizeRigidBody(scene.rigidBody ?? scene.rigid_body),
    sphereCollider: normalizeSphereCollider(scene.sphereCollider ?? scene.sphere_collider),
    particleEmitter: normalizeParticleEmitter(
      scene.particleEmitter ?? scene.particle_emitter,
    ),
    camera: normalizeCamera(scene.camera),
    activeCamera: Boolean(scene.activeCamera),
    light: normalizeLight(scene.light),
    stats: normalizeStats(scene.stats),
  };
}

function sceneRuntimeSliceForTree(tree) {
  return normalizeRuntimeSlice(tree?.root?.metadata?.scene?.runtimeSlice);
}

function sanitizeQuotedText(value, fallback) {
  return String(value ?? fallback ?? "").replaceAll('"', "'");
}

function formatNumber(value) {
  const number = toNumber(value, 0);

  if (Math.abs(number - Math.trunc(number)) < 1e-9) {
    return String(Math.trunc(number));
  }

  return number.toFixed(6).replace(/\.?0+$/, "");
}

function formatBoolFlag(value) {
  return value ? "1" : "0";
}

function createToken(type, value, line) {
  return { type, value, line };
}

function tokenize(source) {
  const tokens = [];
  let index = 0;
  let line = 1;

  while (index < source.length) {
    const char = source[index];

    if (char === " " || char === "\t" || char === "\r") {
      index += 1;
      continue;
    }

    if (char === "\n") {
      line += 1;
      index += 1;
      continue;
    }

    if (char === "#") {
      while (index < source.length && source[index] !== "\n") {
        index += 1;
      }
      continue;
    }

    if (char === "{" || char === "}") {
      tokens.push(createToken("brace", char, line));
      index += 1;
      continue;
    }

    if (char === '"') {
      const startLine = line;
      index += 1;
      const start = index;

      while (index < source.length && source[index] !== '"') {
        if (source[index] === "\n") {
          line += 1;
        }
        index += 1;
      }

      if (index >= source.length) {
        throw new Error(`scene error (line ${startLine}): unterminated string`);
      }

      tokens.push(createToken("string", source.slice(start, index), startLine));
      index += 1;
      continue;
    }

    if (/[A-Za-z_]/.test(char)) {
      const start = index;
      index += 1;
      while (index < source.length && /[A-Za-z0-9_]/.test(source[index])) {
        index += 1;
      }
      tokens.push(createToken("word", source.slice(start, index), line));
      continue;
    }

    if (char === "-" || char === "+" || char === "." || /\d/.test(char)) {
      const start = index;

      if (char === "-" || char === "+") {
        index += 1;
      }

      let hasDigit = false;
      let hasDot = false;

      while (index < source.length) {
        const current = source[index];
        if (/\d/.test(current)) {
          hasDigit = true;
          index += 1;
          continue;
        }
        if (current === "." && !hasDot) {
          hasDot = true;
          index += 1;
          continue;
        }
        break;
      }

      const value = source.slice(start, index);
      if (hasDigit && /^[-+]?(?:\d+\.?\d*|\.\d+)$/.test(value)) {
        tokens.push(createToken("number", Number(value), line));
        continue;
      }

      throw new Error(`scene error (line ${line}): invalid number '${value}'`);
    }

    throw new Error(`scene error (line ${line}): unexpected character '${char}'`);
  }

  return tokens;
}

class SceneParser {
  constructor(tokens) {
    this.tokens = tokens;
    this.index = 0;
  }

  peek() {
    return this.tokens[this.index] ?? null;
  }

  next() {
    const token = this.peek();
    this.index += 1;
    return token;
  }

  expect(type, value) {
    const token = this.next();
    if (!token || token.type !== type || (value !== undefined && token.value !== value)) {
      const line = token?.line ?? this.tokens[this.tokens.length - 1]?.line ?? 1;
      const wanted = value === undefined ? type : `${type} '${value}'`;
      throw new Error(`scene error (line ${line}): expected ${wanted}`);
    }
    return token;
  }

  expectNumber() {
    return this.expect("number").value;
  }

  expectString() {
    return this.expect("string").value;
  }

  expectVec3() {
    return {
      x: this.expectNumber(),
      y: this.expectNumber(),
      z: this.expectNumber(),
    };
  }

  expectColor() {
    return {
      r: this.expectNumber(),
      g: this.expectNumber(),
      b: this.expectNumber(),
      a: this.expectNumber(),
    };
  }

  expectBoolFlag() {
    const token = this.next();
    if (!token) {
      const line = this.tokens[this.tokens.length - 1]?.line ?? 1;
      throw new Error(`scene error (line ${line}): expected bool flag`);
    }

    if (token.type === "number") {
      return token.value > 0;
    }

    throw new Error(`scene error (line ${token.line}): expected bool flag`);
  }

  parseScene() {
    const scene = {
      name: "",
      runtimeSlice: normalizeRuntimeSlice(null),
      entities: [],
    };

    if (this.peek()?.type === "word" && this.peek().value === "scene") {
      this.next();
      scene.name = this.expectString();
    }

    let sawRuntimeSlice = false;

    while (this.peek()) {
      const token = this.peek();
      if (token.type !== "word") {
        throw new Error(
          `scene error (line ${token.line}): expected top-level 'runtime_slice' or 'entity' block`,
        );
      }

      if (token.value === "runtime_slice") {
        if (sawRuntimeSlice) {
          throw new Error(`scene error (line ${token.line}): duplicate runtime_slice block`);
        }
        this.next();
        scene.runtimeSlice = this.parseRuntimeSlice();
        sawRuntimeSlice = true;
        continue;
      }

      if (token.value === "entity") {
        scene.entities.push(this.parseEntity());
        continue;
      }

      throw new Error(
        `scene error (line ${token.line}): expected top-level 'runtime_slice' or 'entity' block`,
      );
    }

    return scene;
  }

  parseEntity() {
    this.expect("word", "entity");
    const entity = {
      name: this.expectString(),
      transform: null,
      velocity: null,
      rigidBody: null,
      sphereCollider: null,
      particleEmitter: null,
      camera: null,
      activeCamera: false,
      light: null,
      stats: null,
      parentName: null,
    };

    this.expect("brace", "{");

    while (this.peek() && !(this.peek().type === "brace" && this.peek().value === "}")) {
      const token = this.expect("word");
      switch (token.value) {
        case "transform":
          entity.transform = this.parseTransform();
          break;
        case "velocity":
          entity.velocity = this.parseVelocity();
          break;
        case "rigid_body":
          entity.rigidBody = this.parseRigidBody();
          break;
        case "sphere_collider":
          entity.sphereCollider = this.parseSphereCollider();
          break;
        case "particle_emitter":
          entity.particleEmitter = this.parseParticleEmitter();
          break;
        case "camera":
          entity.camera = this.parseCamera();
          break;
        case "active_camera":
          entity.activeCamera = true;
          break;
        case "light":
          entity.light = this.parseLight();
          break;
        case "stats":
          entity.stats = this.parseStats();
          break;
        case "parent":
          entity.parentName = this.expectString();
          break;
        default:
          throw new Error(
            `scene error (line ${token.line}): unknown entity component '${token.value}'`,
          );
      }
    }

    this.expect("brace", "}");
    return entity;
  }

  parseTransform() {
    const transform = normalizeTransform(null);
    this.expect("brace", "{");

    while (this.peek() && !(this.peek().type === "brace" && this.peek().value === "}")) {
      const token = this.expect("word");
      switch (token.value) {
        case "pos":
        case "position":
          transform.position = this.expectVec3();
          break;
        case "rot":
        case "rotation":
          transform.rotation = {
            x: this.expectNumber(),
            y: this.expectNumber(),
            z: this.expectNumber(),
            w: this.expectNumber(),
          };
          break;
        case "scale":
          transform.scale = this.expectVec3();
          break;
        case "euler":
          transform.rotation = quatFromEulerDegrees(
            this.expectNumber(),
            this.expectNumber(),
            this.expectNumber(),
          );
          break;
        default:
          throw new Error(`scene error (line ${token.line}): unknown transform field '${token.value}'`);
      }
    }

    this.expect("brace", "}");
    return transform;
  }

  parseVelocity() {
    const velocity = normalizeVelocity({ linear: ZERO_VEC3, angular: ZERO_VEC3 });
    this.expect("brace", "{");

    while (this.peek() && !(this.peek().type === "brace" && this.peek().value === "}")) {
      const token = this.expect("word");
      switch (token.value) {
        case "linear":
          velocity.linear = this.expectVec3();
          break;
        case "angular":
          velocity.angular = this.expectVec3();
          break;
        default:
          throw new Error(`scene error (line ${token.line}): unknown velocity field '${token.value}'`);
      }
    }

    this.expect("brace", "}");
    return velocity;
  }

  parseRigidBody() {
    const rigidBody = normalizeRigidBody({});
    this.expect("brace", "{");

    while (this.peek() && !(this.peek().type === "brace" && this.peek().value === "}")) {
      const token = this.expect("word");
      switch (token.value) {
        case "gravity_scale":
          rigidBody.gravityScale = this.expectNumber();
          break;
        case "linear_damping":
          rigidBody.linearDamping = this.expectNumber();
          break;
        case "angular_damping":
          rigidBody.angularDamping = this.expectNumber();
          break;
        case "restitution":
          rigidBody.restitution = this.expectNumber();
          break;
        default:
          throw new Error(`scene error (line ${token.line}): unknown rigid_body field '${token.value}'`);
      }
    }

    this.expect("brace", "}");
    return rigidBody;
  }

  parseSphereCollider() {
    const sphereCollider = normalizeSphereCollider({});
    this.expect("brace", "{");

    while (this.peek() && !(this.peek().type === "brace" && this.peek().value === "}")) {
      const token = this.expect("word");
      switch (token.value) {
        case "radius":
          sphereCollider.radius = this.expectNumber();
          break;
        default:
          throw new Error(
            `scene error (line ${token.line}): unknown sphere_collider field '${token.value}'`,
          );
      }
    }

    this.expect("brace", "}");
    return sphereCollider;
  }

  parseParticleEmitter() {
    const particleEmitter = normalizeParticleEmitter({});
    this.expect("brace", "{");

    while (this.peek() && !(this.peek().type === "brace" && this.peek().value === "}")) {
      const token = this.expect("word");
      switch (token.value) {
        case "emitting":
          particleEmitter.emitting = this.expectBoolFlag();
          break;
        case "rate":
        case "rate_per_second":
          particleEmitter.ratePerSecond = this.expectNumber();
          break;
        case "max":
        case "max_particles":
          particleEmitter.maxParticles = Math.max(0, toInteger(this.expectNumber(), 0));
          break;
        case "spawn_radius":
          particleEmitter.spawnRadius = this.expectNumber();
          break;
        case "dir":
        case "direction":
          particleEmitter.direction = this.expectVec3();
          break;
        case "spread":
          particleEmitter.spread = this.expectNumber();
          break;
        case "speed_min":
          particleEmitter.speedMin = this.expectNumber();
          break;
        case "speed_max":
          particleEmitter.speedMax = this.expectNumber();
          break;
        case "lifetime_min":
          particleEmitter.lifetimeMin = this.expectNumber();
          break;
        case "lifetime_max":
          particleEmitter.lifetimeMax = this.expectNumber();
          break;
        case "gravity_scale":
          particleEmitter.gravityScale = this.expectNumber();
          break;
        case "linear_damping":
          particleEmitter.linearDamping = this.expectNumber();
          break;
        case "start_size":
          particleEmitter.startSize = this.expectNumber();
          break;
        case "end_size":
          particleEmitter.endSize = this.expectNumber();
          break;
        case "start_color":
          particleEmitter.startColor = this.expectColor();
          break;
        case "end_color":
          particleEmitter.endColor = this.expectColor();
          break;
        case "seed":
          particleEmitter.seed = Math.max(0, toInteger(this.expectNumber(), 0));
          break;
        default:
          throw new Error(
            `scene error (line ${token.line}): unknown particle_emitter field '${token.value}'`,
          );
      }
    }

    this.expect("brace", "}");
    return particleEmitter;
  }

  parseCamera() {
    const camera = normalizeCamera({});
    this.expect("brace", "{");

    while (this.peek() && !(this.peek().type === "brace" && this.peek().value === "}")) {
      const token = this.expect("word");
      switch (token.value) {
        case "fov":
          camera.fov = this.expectNumber();
          break;
        case "near":
          camera.near = this.expectNumber();
          break;
        case "far":
          camera.far = this.expectNumber();
          break;
        default:
          throw new Error(`scene error (line ${token.line}): unknown camera field '${token.value}'`);
      }
    }

    this.expect("brace", "}");
    return camera;
  }

  parseLight() {
    const light = normalizeLight({});
    this.expect("brace", "{");

    while (this.peek() && !(this.peek().type === "brace" && this.peek().value === "}")) {
      const token = this.expect("word");
      switch (token.value) {
        case "dir":
        case "direction":
          light.direction = this.expectVec3();
          break;
        case "color":
          light.color = this.expectColor();
          break;
        case "intensity":
          light.intensity = this.expectNumber();
          break;
        case "ambient":
          light.ambient = this.expectNumber();
          break;
        default:
          throw new Error(`scene error (line ${token.line}): unknown light field '${token.value}'`);
      }
    }

    this.expect("brace", "}");
    return light;
  }

  parseStats() {
    const stats = normalizeStats({});
    this.expect("brace", "{");

    while (this.peek() && !(this.peek().type === "brace" && this.peek().value === "}")) {
      const token = this.expect("word");
      switch (token.value) {
        case "health":
          stats.health = this.expectNumber();
          break;
        case "mana":
          stats.mana = this.expectNumber();
          break;
        case "attack":
          stats.attack = this.expectNumber();
          break;
        case "defense":
          stats.defense = this.expectNumber();
          break;
        case "speed":
          stats.speed = this.expectNumber();
          break;
        default:
          throw new Error(`scene error (line ${token.line}): unknown stats field '${token.value}'`);
      }
    }

    this.expect("brace", "}");
    return stats;
  }

  parseRuntimeSlice() {
    const runtimeSlice = normalizeRuntimeSlice(null);
    this.expect("brace", "{");

    let sawTerrain = false;

    while (this.peek() && !(this.peek().type === "brace" && this.peek().value === "}")) {
      const token = this.expect("word");
      switch (token.value) {
        case "terrain":
          if (sawTerrain) {
            throw new Error(`scene error (line ${token.line}): duplicate terrain block in runtime_slice`);
          }
          runtimeSlice.terrain = this.parseRuntimeTerrain();
          sawTerrain = true;
          break;
        case "paint_layer":
          runtimeSlice.terrain.paintLayers.push(
            this.parseTerrainPaintLayer(this.expectString()),
          );
          break;
        case "vegetation_prototype":
          runtimeSlice.vegetationPrototypes.push(
            this.parseVegetationPrototype(this.expectString()),
          );
          break;
        case "vegetation_paint":
          runtimeSlice.vegetationPaints.push(
            this.parseVegetationPaint(this.expectString()),
          );
          break;
        case "static_mesh_actor":
          runtimeSlice.staticMeshActors.push(
            this.parseStaticMeshActor(this.expectString()),
          );
          break;
        case "water_volume":
          runtimeSlice.waterVolumes.push(this.parseWaterVolume(this.expectString()));
          break;
        case "spawn_point":
          runtimeSlice.spawnPoints.push(this.parseSpawnPoint(this.expectString()));
          break;
        case "patrol_route":
          runtimeSlice.patrolRoutes.push(this.parsePatrolRoute(this.expectString()));
          break;
        case "vehicle_spawner":
          runtimeSlice.vehicleSpawners.push(
            this.parseVehicleSpawner(this.expectString()),
          );
          break;
        case "breakable_prop":
          runtimeSlice.breakableProps.push(this.parseBreakableProp(this.expectString()));
          break;
        default:
          throw new Error(
            `scene error (line ${token.line}): unknown runtime_slice block entry`,
          );
      }
    }

    this.expect("brace", "}");
    return runtimeSlice;
  }

  parseRuntimeTerrain() {
    const terrain = normalizeTerrain({});
    this.expect("brace", "{");

    while (this.peek() && !(this.peek().type === "brace" && this.peek().value === "}")) {
      const token = this.expect("word");
      switch (token.value) {
        case "size":
        case "size_meters":
          terrain.sizeMeters = this.expectVec3();
          break;
        case "chunk_size":
        case "chunk_size_meters":
          terrain.chunkSizeMeters = Math.max(0, toInteger(this.expectNumber(), 0));
          break;
        case "height_scale":
        case "height_scale_meters":
          terrain.heightScaleMeters = this.expectNumber();
          break;
        default:
          throw new Error(`scene error (line ${token.line}): unknown terrain field '${token.value}'`);
      }
    }

    this.expect("brace", "}");
    return terrain;
  }

  parseTerrainPaintLayer(name) {
    const layer = normalizeTerrainPaintLayer({ name });
    this.expect("brace", "{");

    while (this.peek() && !(this.peek().type === "brace" && this.peek().value === "}")) {
      const token = this.expect("word");
      switch (token.value) {
        case "material":
          layer.material = this.expectString();
          break;
        case "tiling":
          layer.tiling = this.expectNumber();
          break;
        case "channel":
        case "weightmap_channel":
          layer.weightmapChannel = Math.max(0, toInteger(this.expectNumber(), 0));
          break;
        default:
          throw new Error(
            `scene error (line ${token.line}): unknown paint_layer field '${token.value}'`,
          );
      }
    }

    this.expect("brace", "}");
    return layer;
  }

  parseVegetationPrototype(name) {
    const prototype = normalizeVegetationPrototype({ name });
    this.expect("brace", "{");

    while (this.peek() && !(this.peek().type === "brace" && this.peek().value === "}")) {
      const token = this.expect("word");
      switch (token.value) {
        case "mesh":
          prototype.mesh = this.expectString();
          break;
        case "material":
          prototype.material = this.expectString();
          break;
        case "density":
        case "density_per_100m2":
          prototype.densityPer100m2 = this.expectNumber();
          break;
        case "random_yaw":
          prototype.randomYaw = this.expectBoolFlag();
          break;
        case "align_to_normal":
          prototype.alignToNormal = this.expectBoolFlag();
          break;
        default:
          throw new Error(
            `scene error (line ${token.line}): unknown vegetation_prototype field '${token.value}'`,
          );
      }
    }

    this.expect("brace", "}");
    return prototype;
  }

  parseVegetationPaint(prototypeName) {
    const stroke = normalizeVegetationPaint({ prototype: prototypeName });
    this.expect("brace", "{");

    while (this.peek() && !(this.peek().type === "brace" && this.peek().value === "}")) {
      const token = this.expect("word");
      switch (token.value) {
        case "center":
          stroke.center = this.expectVec3();
          break;
        case "radius":
          stroke.radius = this.expectNumber();
          break;
        case "density":
          stroke.density = this.expectNumber();
          break;
        default:
          throw new Error(
            `scene error (line ${token.line}): unknown vegetation_paint field '${token.value}'`,
          );
      }
    }

    this.expect("brace", "}");
    return stroke;
  }

  parseStaticMeshActor(name) {
    const actor = normalizeStaticMeshActor({ name });
    this.expect("brace", "{");

    while (this.peek() && !(this.peek().type === "brace" && this.peek().value === "}")) {
      const token = this.expect("word");
      switch (token.value) {
        case "mesh":
          actor.mesh = this.expectString();
          break;
        case "material":
          actor.material = this.expectString();
          break;
        case "pos":
        case "position":
          actor.position = this.expectVec3();
          break;
        case "rot":
        case "rotation":
          actor.rotationEulerDeg = this.expectVec3();
          break;
        case "scale":
          actor.scale = this.expectVec3();
          break;
        default:
          throw new Error(
            `scene error (line ${token.line}): unknown static_mesh_actor field '${token.value}'`,
          );
      }
    }

    this.expect("brace", "}");
    return actor;
  }

  parseWaterVolume(name) {
    const volume = normalizeWaterVolume({ name });
    this.expect("brace", "{");

    while (this.peek() && !(this.peek().type === "brace" && this.peek().value === "}")) {
      const token = this.expect("word");
      switch (token.value) {
        case "material":
          volume.material = this.expectString();
          break;
        case "center":
          volume.center = this.expectVec3();
          break;
        case "extents":
          volume.extents = this.expectVec3();
          break;
        case "wave_strength":
          volume.waveStrength = this.expectNumber();
          break;
        default:
          throw new Error(
            `scene error (line ${token.line}): unknown water_volume field '${token.value}'`,
          );
      }
    }

    this.expect("brace", "}");
    return volume;
  }

  parseSpawnPoint(name) {
    const spawnPoint = normalizeSpawnPoint({ name });
    this.expect("brace", "{");

    while (this.peek() && !(this.peek().type === "brace" && this.peek().value === "}")) {
      const token = this.expect("word");
      switch (token.value) {
        case "kind": {
          const kind = this.expectString();
          const normalized = String(kind).trim().toLowerCase();
          if (
            normalized !== "generic"
            && normalized !== "player"
            && normalized !== "enemy"
            && normalized !== "vehicle"
          ) {
            throw new Error(`scene error (line ${token.line}): unknown spawn_point kind`);
          }
          spawnPoint.kind = normalized;
          break;
        }
        case "pos":
        case "position":
          spawnPoint.position = this.expectVec3();
          break;
        case "yaw":
        case "facing_yaw_deg":
          spawnPoint.facingYawDeg = this.expectNumber();
          break;
        default:
          throw new Error(
            `scene error (line ${token.line}): unknown spawn_point field '${token.value}'`,
          );
      }
    }

    this.expect("brace", "}");
    return spawnPoint;
  }

  parsePatrolRoute(name) {
    const route = normalizePatrolRoute({ name });
    this.expect("brace", "{");

    while (this.peek() && !(this.peek().type === "brace" && this.peek().value === "}")) {
      const token = this.expect("word");
      switch (token.value) {
        case "looped":
          route.looped = this.expectBoolFlag();
          break;
        case "point":
          route.points.push(this.expectVec3());
          break;
        default:
          throw new Error(
            `scene error (line ${token.line}): unknown patrol_route field '${token.value}'`,
          );
      }
    }

    this.expect("brace", "}");
    return route;
  }

  parseVehicleSpawner(name) {
    const spawner = normalizeVehicleSpawner({ name });
    this.expect("brace", "{");

    while (this.peek() && !(this.peek().type === "brace" && this.peek().value === "}")) {
      const token = this.expect("word");
      switch (token.value) {
        case "mesh":
          spawner.mesh = this.expectString();
          break;
        case "material":
          spawner.material = this.expectString();
          break;
        case "pos":
        case "position":
          spawner.position = this.expectVec3();
          break;
        case "yaw":
        case "facing_yaw_deg":
          spawner.facingYawDeg = this.expectNumber();
          break;
        default:
          throw new Error(
            `scene error (line ${token.line}): unknown vehicle_spawner field '${token.value}'`,
          );
      }
    }

    this.expect("brace", "}");
    return spawner;
  }

  parseBreakableProp(name) {
    const prop = normalizeBreakableProp({ name });
    this.expect("brace", "{");

    while (this.peek() && !(this.peek().type === "brace" && this.peek().value === "}")) {
      const token = this.expect("word");
      switch (token.value) {
        case "intact_mesh":
          prop.intactMesh = this.expectString();
          break;
        case "broken_mesh":
          prop.brokenMesh = this.expectString();
          break;
        case "material":
          prop.material = this.expectString();
          break;
        case "pos":
        case "position":
          prop.position = this.expectVec3();
          break;
        case "rot":
        case "rotation":
          prop.rotationEulerDeg = this.expectVec3();
          break;
        case "scale":
          prop.scale = this.expectVec3();
          break;
        case "hit_points":
          prop.hitPoints = this.expectNumber();
          break;
        case "break_impulse":
          prop.breakImpulse = this.expectNumber();
          break;
        default:
          throw new Error(
            `scene error (line ${token.line}): unknown breakable_prop field '${token.value}'`,
          );
      }
    }

    this.expect("brace", "}");
    return prop;
  }
}

function degToRad(value) {
  return (value * Math.PI) / 180;
}

function normalizeAxis(axis) {
  const length = Math.hypot(axis.x, axis.y, axis.z);
  if (length <= 1e-10) {
    return { x: 0, y: 0, z: 0 };
  }
  return {
    x: axis.x / length,
    y: axis.y / length,
    z: axis.z / length,
  };
}

function quatFromAxisAngle(axis, angle) {
  const unitAxis = normalizeAxis(axis);
  const half = angle * 0.5;
  const sine = Math.sin(half);
  const cosine = Math.cos(half);

  return {
    x: unitAxis.x * sine,
    y: unitAxis.y * sine,
    z: unitAxis.z * sine,
    w: cosine,
  };
}

function multiplyQuat(left, right) {
  return {
    x: left.w * right.x + left.x * right.w + left.y * right.z - left.z * right.y,
    y: left.w * right.y - left.x * right.z + left.y * right.w + left.z * right.x,
    z: left.w * right.z + left.x * right.y - left.y * right.x + left.z * right.w,
    w: left.w * right.w - left.x * right.x - left.y * right.y - left.z * right.z,
  };
}

function normalizeQuatLength(quat) {
  const length = Math.hypot(quat.x, quat.y, quat.z, quat.w);
  if (length <= 1e-10) {
    return { ...IDENTITY_ROTATION };
  }
  return {
    x: quat.x / length,
    y: quat.y / length,
    z: quat.z / length,
    w: quat.w / length,
  };
}

function quatFromEulerDegrees(x, y, z) {
  return normalizeQuatLength(
    multiplyQuat(
      multiplyQuat(
        quatFromAxisAngle({ x: 0, y: 0, z: 1 }, degToRad(z)),
        quatFromAxisAngle({ x: 0, y: 1, z: 0 }, degToRad(y)),
      ),
      quatFromAxisAngle({ x: 1, y: 0, z: 0 }, degToRad(x)),
    ),
  );
}

function componentStyle(entity) {
  if (entity.activeCamera || entity.camera) {
    return {
      background: "linear-gradient(160deg, #45c2ff, #244fc4)",
    };
  }

  if (entity.light) {
    return {
      background: "linear-gradient(160deg, #ffd36b, #f08d42)",
      color: "#23170d",
    };
  }

  if (entity.stats) {
    return {
      background: "linear-gradient(160deg, #57d58c, #1d8f62)",
    };
  }

  return {
    background: "linear-gradient(160deg, #8b8df8, #3b4fc0)",
  };
}

function componentSize(entity) {
  let width = 170;
  let height = 106;

  if (entity.camera) {
    width = 188;
    height = 112;
  }

  if (entity.light) {
    width = Math.max(width, 182);
    height = Math.max(height, 112);
  }

  if (entity.stats) {
    width = Math.max(width, 196);
    height = Math.max(height, 122);
  }

  return { width, height };
}

function componentText(entity) {
  const lines = [];

  if (entity.camera) {
    const label = entity.activeCamera ? "Active camera" : "Camera";
    lines.push(`${label} | fov ${formatNumber(entity.camera.fov)} | near ${formatNumber(entity.camera.near)} | far ${formatNumber(entity.camera.far)}`);
  }

  if (entity.light) {
    lines.push(`Light | dir ${formatNumber(entity.light.direction.x)}, ${formatNumber(entity.light.direction.y)}, ${formatNumber(entity.light.direction.z)} | ambient ${formatNumber(entity.light.ambient)}`);
  }

  if (entity.stats) {
    lines.push(`Stats | hp ${formatNumber(entity.stats.health)} | mana ${formatNumber(entity.stats.mana)} | atk ${formatNumber(entity.stats.attack)} | def ${formatNumber(entity.stats.defense)}`);
  }

  if (entity.particleEmitter) {
    lines.push(`Emitter | rate ${formatNumber(entity.particleEmitter.ratePerSecond)} | max ${formatNumber(entity.particleEmitter.maxParticles)}`);
  }

  if (entity.velocity) {
    lines.push(`Velocity | lin ${formatNumber(entity.velocity.linear.x)}, ${formatNumber(entity.velocity.linear.y)}, ${formatNumber(entity.velocity.linear.z)}`);
  }

  if (!lines.length) {
    lines.push("Scene entity imported from .hsc.");
  }

  return lines.join("\n");
}

function createSyntheticRoot(sceneName, topLevelCount, runtimeSlice = null) {
  return new MorphicObject({
    name: sceneName ? `Scene: ${sceneName}` : "Imported Scene",
    position: { x: 0, y: 0 },
    size: { width: 1440, height: 960 },
    style: {
      background: "linear-gradient(180deg, rgba(18, 24, 34, 0.92), rgba(13, 16, 24, 0.78))",
      border: "1px solid rgba(255, 255, 255, 0.08)",
      borderRadius: "0",
      boxShadow: "none",
    },
    metadata: {
      scene: {
        syntheticRoot: true,
        sceneName: sceneName ?? "Imported Scene",
        runtimeSlice: normalizeRuntimeSlice(runtimeSlice),
        transform: normalizeTransform(null),
      },
    },
    text: `${topLevelCount} root entities imported from .hsc.`,
  });
}

function applyFallbackLayout(objects, parent = null) {
  for (const [index, object] of objects.entries()) {
    if (!object.metadata.scene.explicitTransform) {
      const position = parent
        ? {
            x: 34 + index * 24,
            y: 52 + index * 20,
          }
        : {
            x: 72 + (index % 4) * 196,
            y: 132 + Math.floor(index / 4) * 154,
          };

      object.position = position;
      object.metadata.scene.transform.position.x = position.x;
      object.metadata.scene.transform.position.y = position.y;
    }

    applyFallbackLayout(object.children, object);
  }
}

function createObjectFromEntity(entity, index) {
  const explicitTransform = Boolean(entity.transform);
  const transform = normalizeTransform(entity.transform, {
    x: 72 + (index % 4) * 196,
    y: 132 + Math.floor(index / 4) * 154,
    z: 0,
  });

  return new MorphicObject({
    name: entity.name,
    position: {
      x: transform.position.x,
      y: transform.position.y,
    },
    size: componentSize(entity),
    style: componentStyle(entity),
    metadata: {
      importedFrom: "hsc",
      scene: {
        sceneName: "",
        explicitTransform,
        transform,
        velocity: normalizeVelocity(entity.velocity),
        rigidBody: normalizeRigidBody(entity.rigidBody),
        sphereCollider: normalizeSphereCollider(entity.sphereCollider),
        particleEmitter: normalizeParticleEmitter(entity.particleEmitter),
        camera: normalizeCamera(entity.camera),
        activeCamera: Boolean(entity.activeCamera),
        light: normalizeLight(entity.light),
        stats: normalizeStats(entity.stats),
      },
    },
    text: componentText(entity),
  });
}

function isSyntheticRoot(object) {
  return Boolean(object?.metadata?.scene?.syntheticRoot);
}

function pushQuotedLine(lines, prefix, value) {
  const text = normalizeOptionalString(value);
  if (text !== null) {
    lines.push(`${prefix} "${sanitizeQuotedText(text, "")}"`);
  }
}

function pushVec3Line(lines, prefix, value) {
  lines.push(`${prefix} ${formatNumber(value.x)} ${formatNumber(value.y)} ${formatNumber(value.z)}`);
}

function pushColorLine(lines, prefix, value) {
  lines.push(
    `${prefix} ${formatNumber(value.r)} ${formatNumber(value.g)} ${formatNumber(value.b)} ${formatNumber(value.a)}`,
  );
}

function writeRuntimeSlice(lines, runtimeSlice) {
  const slice = normalizeRuntimeSlice(runtimeSlice);
  lines.push("runtime_slice {");

  if (!isDefaultTerrain(slice.terrain)) {
    lines.push("  terrain {");
    pushVec3Line(lines, "    size", slice.terrain.sizeMeters);
    lines.push(`    chunk_size ${formatNumber(slice.terrain.chunkSizeMeters)}`);
    lines.push(`    height_scale ${formatNumber(slice.terrain.heightScaleMeters)}`);
    lines.push("  }");
  }

  for (const layer of slice.terrain.paintLayers) {
    lines.push(`  paint_layer "${sanitizeQuotedText(layer.name, "paint_layer")}" {`);
    pushQuotedLine(lines, "    material", layer.material);
    lines.push(`    tiling ${formatNumber(layer.tiling)}`);
    lines.push(`    channel ${formatNumber(layer.weightmapChannel)}`);
    lines.push("  }");
  }

  for (const prototype of slice.vegetationPrototypes) {
    lines.push(
      `  vegetation_prototype "${sanitizeQuotedText(prototype.name, "vegetation_prototype")}" {`,
    );
    pushQuotedLine(lines, "    mesh", prototype.mesh);
    pushQuotedLine(lines, "    material", prototype.material);
    lines.push(`    density ${formatNumber(prototype.densityPer100m2)}`);
    lines.push(`    random_yaw ${formatBoolFlag(prototype.randomYaw)}`);
    lines.push(`    align_to_normal ${formatBoolFlag(prototype.alignToNormal)}`);
    lines.push("  }");
  }

  for (const stroke of slice.vegetationPaints) {
    lines.push(`  vegetation_paint "${sanitizeQuotedText(stroke.prototype, "vegetation")}" {`);
    pushVec3Line(lines, "    center", stroke.center);
    lines.push(`    radius ${formatNumber(stroke.radius)}`);
    lines.push(`    density ${formatNumber(stroke.density)}`);
    lines.push("  }");
  }

  for (const actor of slice.staticMeshActors) {
    lines.push(`  static_mesh_actor "${sanitizeQuotedText(actor.name, "static_mesh_actor")}" {`);
    pushQuotedLine(lines, "    mesh", actor.mesh);
    pushQuotedLine(lines, "    material", actor.material);
    pushVec3Line(lines, "    pos", actor.position);
    pushVec3Line(lines, "    rot", actor.rotationEulerDeg);
    pushVec3Line(lines, "    scale", actor.scale);
    lines.push("  }");
  }

  for (const volume of slice.waterVolumes) {
    lines.push(`  water_volume "${sanitizeQuotedText(volume.name, "water_volume")}" {`);
    pushQuotedLine(lines, "    material", volume.material);
    pushVec3Line(lines, "    center", volume.center);
    pushVec3Line(lines, "    extents", volume.extents);
    lines.push(`    wave_strength ${formatNumber(volume.waveStrength)}`);
    lines.push("  }");
  }

  for (const spawnPoint of slice.spawnPoints) {
    lines.push(`  spawn_point "${sanitizeQuotedText(spawnPoint.name, "spawn_point")}" {`);
    lines.push(`    kind "${sanitizeQuotedText(spawnPoint.kind, "generic")}"`);
    pushVec3Line(lines, "    pos", spawnPoint.position);
    lines.push(`    yaw ${formatNumber(spawnPoint.facingYawDeg)}`);
    lines.push("  }");
  }

  for (const route of slice.patrolRoutes) {
    lines.push(`  patrol_route "${sanitizeQuotedText(route.name, "patrol_route")}" {`);
    lines.push(`    looped ${formatBoolFlag(route.looped)}`);
    for (const point of route.points) {
      pushVec3Line(lines, "    point", point);
    }
    lines.push("  }");
  }

  for (const spawner of slice.vehicleSpawners) {
    lines.push(`  vehicle_spawner "${sanitizeQuotedText(spawner.name, "vehicle_spawner")}" {`);
    pushQuotedLine(lines, "    mesh", spawner.mesh);
    pushQuotedLine(lines, "    material", spawner.material);
    pushVec3Line(lines, "    pos", spawner.position);
    lines.push(`    yaw ${formatNumber(spawner.facingYawDeg)}`);
    lines.push("  }");
  }

  for (const prop of slice.breakableProps) {
    lines.push(`  breakable_prop "${sanitizeQuotedText(prop.name, "breakable_prop")}" {`);
    pushQuotedLine(lines, "    intact_mesh", prop.intactMesh);
    pushQuotedLine(lines, "    broken_mesh", prop.brokenMesh);
    pushQuotedLine(lines, "    material", prop.material);
    pushVec3Line(lines, "    pos", prop.position);
    pushVec3Line(lines, "    rot", prop.rotationEulerDeg);
    pushVec3Line(lines, "    scale", prop.scale);
    lines.push(`    hit_points ${formatNumber(prop.hitPoints)}`);
    lines.push(`    break_impulse ${formatNumber(prop.breakImpulse)}`);
    lines.push("  }");
  }

  lines.push("}");
}

export function getSceneName(tree) {
  const root = tree?.root;
  const treeName = tree?.sceneName;
  const sceneName = root?.metadata?.scene?.sceneName;
  return sanitizeQuotedText(treeName || sceneName || root?.name || "Morphic Scene", "Morphic Scene");
}

export function parseScene(source) {
  return new SceneParser(tokenize(source)).parseScene();
}

export function serializeScene(scene) {
  const lines = [];
  const sceneName = sanitizeQuotedText(scene?.name, "");
  const runtimeSlice = normalizeRuntimeSlice(scene?.runtimeSlice);
  const entities = Array.isArray(scene?.entities) ? scene.entities.map(normalizeSceneEntity) : [];

  if (sceneName !== "") {
    lines.push(`scene "${sceneName}"`, "");
  }

  if (!isRuntimeSliceEmpty(runtimeSlice)) {
    writeRuntimeSlice(lines, runtimeSlice);
    lines.push("");
  }

  for (const entity of entities) {
    lines.push(`entity "${sanitizeQuotedText(entity.name, "Entity")}" {`);

    if (entity.transform) {
      lines.push("  transform {");
      pushVec3Line(lines, "    pos", entity.transform.position);
      lines.push(
        `    rot ${formatNumber(entity.transform.rotation.x)} ${formatNumber(entity.transform.rotation.y)} ${formatNumber(entity.transform.rotation.z)} ${formatNumber(entity.transform.rotation.w)}`,
      );
      pushVec3Line(lines, "    scale", entity.transform.scale);
      lines.push("  }");
    }

    if (entity.velocity) {
      lines.push("  velocity {");
      pushVec3Line(lines, "    linear", entity.velocity.linear);
      pushVec3Line(lines, "    angular", entity.velocity.angular);
      lines.push("  }");
    }

    if (entity.rigidBody) {
      lines.push("  rigid_body {");
      lines.push(`    gravity_scale ${formatNumber(entity.rigidBody.gravityScale)}`);
      lines.push(`    linear_damping ${formatNumber(entity.rigidBody.linearDamping)}`);
      lines.push(`    angular_damping ${formatNumber(entity.rigidBody.angularDamping)}`);
      lines.push(`    restitution ${formatNumber(entity.rigidBody.restitution)}`);
      lines.push("  }");
    }

    if (entity.sphereCollider) {
      lines.push(`  sphere_collider { radius ${formatNumber(entity.sphereCollider.radius)} }`);
    }

    if (entity.particleEmitter) {
      lines.push("  particle_emitter {");
      lines.push(`    emitting ${formatBoolFlag(entity.particleEmitter.emitting)}`);
      lines.push(`    rate_per_second ${formatNumber(entity.particleEmitter.ratePerSecond)}`);
      lines.push(`    max_particles ${formatNumber(entity.particleEmitter.maxParticles)}`);
      lines.push(`    spawn_radius ${formatNumber(entity.particleEmitter.spawnRadius)}`);
      pushVec3Line(lines, "    direction", entity.particleEmitter.direction);
      lines.push(`    spread ${formatNumber(entity.particleEmitter.spread)}`);
      lines.push(`    speed_min ${formatNumber(entity.particleEmitter.speedMin)}`);
      lines.push(`    speed_max ${formatNumber(entity.particleEmitter.speedMax)}`);
      lines.push(`    lifetime_min ${formatNumber(entity.particleEmitter.lifetimeMin)}`);
      lines.push(`    lifetime_max ${formatNumber(entity.particleEmitter.lifetimeMax)}`);
      lines.push(`    gravity_scale ${formatNumber(entity.particleEmitter.gravityScale)}`);
      lines.push(`    linear_damping ${formatNumber(entity.particleEmitter.linearDamping)}`);
      lines.push(`    start_size ${formatNumber(entity.particleEmitter.startSize)}`);
      lines.push(`    end_size ${formatNumber(entity.particleEmitter.endSize)}`);
      pushColorLine(lines, "    start_color", entity.particleEmitter.startColor);
      pushColorLine(lines, "    end_color", entity.particleEmitter.endColor);
      lines.push(`    seed ${formatNumber(entity.particleEmitter.seed)}`);
      lines.push("  }");
    }

    if (entity.camera) {
      lines.push(
        `  camera { fov ${formatNumber(entity.camera.fov)} near ${formatNumber(entity.camera.near)} far ${formatNumber(entity.camera.far)} }`,
      );
    }

    if (entity.activeCamera) {
      lines.push("  active_camera");
    }

    if (entity.light) {
      lines.push("  light {");
      pushVec3Line(lines, "    dir", entity.light.direction);
      pushColorLine(lines, "    color", entity.light.color);
      lines.push(`    intensity ${formatNumber(entity.light.intensity)}`);
      lines.push(`    ambient ${formatNumber(entity.light.ambient)}`);
      lines.push("  }");
    }

    if (entity.stats) {
      lines.push(
        `  stats { health ${formatNumber(entity.stats.health)} mana ${formatNumber(entity.stats.mana)} attack ${formatNumber(entity.stats.attack)} defense ${formatNumber(entity.stats.defense)} speed ${formatNumber(entity.stats.speed)} }`,
      );
    }

    if (entity.parentName !== null) {
      lines.push(`  parent "${sanitizeQuotedText(entity.parentName, "Parent")}"`);
    }

    lines.push("}", "");
  }

  return `${lines.join("\n").trimEnd()}\n`;
}

export function treeToSceneData(tree, options = {}) {
  const roots = isSyntheticRoot(tree.root) ? tree.root.children : [tree.root];
  const entries = [];

  function visit(object, parent = null) {
    entries.push({ object, parent });
    for (const child of object.children) {
      visit(child, object);
    }
  }

  for (const root of roots) {
    visit(root);
  }

  const nameMap = new Map();
  const usedNames = new Set();

  for (const entry of entries) {
    const baseName = sanitizeQuotedText(entry.object.name, `Entity ${entry.object.id}`);
    let nextName = baseName;
    let suffix = 2;

    while (usedNames.has(nextName)) {
      nextName = `${baseName} ${suffix}`;
      suffix += 1;
    }

    usedNames.add(nextName);
    nameMap.set(entry.object.id, nextName);
  }

  const authoredSceneName =
    options.sceneName
    ?? tree?.sceneName
    ?? tree?.root?.metadata?.scene?.sceneName;

  return {
    name: sanitizeQuotedText(authoredSceneName ?? getSceneName(tree), "Morphic Scene"),
    runtimeSlice: sceneRuntimeSliceForTree(tree),
    entities: entries.map(({ object, parent }) => {
      const scene = sceneMetadataForObject(object);
      return {
        name: nameMap.get(object.id),
        transform: scene.transform,
        velocity: scene.velocity,
        rigidBody: scene.rigidBody,
        sphereCollider: scene.sphereCollider,
        particleEmitter: scene.particleEmitter,
        camera: scene.camera,
        activeCamera: scene.activeCamera,
        light: scene.light,
        stats: scene.stats,
        parentName: parent ? nameMap.get(parent.id) : null,
      };
    }),
  };
}

export function exportTreeToScene(tree, options = {}) {
  return serializeScene(treeToSceneData(tree, options));
}

export function sceneDataToTree(scene) {
  const normalizedScene = {
    name: normalizeOptionalString(scene?.name) ?? "",
    runtimeSlice: normalizeRuntimeSlice(scene?.runtimeSlice),
    entities: Array.isArray(scene?.entities) ? scene.entities.map(normalizeSceneEntity) : [],
  };

  const entries = normalizedScene.entities.map((entity, index) => ({
    entity,
    object: createObjectFromEntity(entity, index),
  }));

  const nameMap = new Map();
  for (const entry of entries) {
    if (entry.entity.name !== "") {
      nameMap.set(entry.entity.name, entry.object);
    }
  }

  const entryByObject = new Map(entries.map((entry) => [entry.object, entry]));
  const attachmentState = new Map();

  function attachEntry(entry) {
    const state = attachmentState.get(entry.object);
    if (state === "attached") {
      return entry.object.parent;
    }
    if (state === "visiting") {
      throw new Error(
        `scene error: cyclic parent relationship involving '${entry.entity.name}'`,
      );
    }

    attachmentState.set(entry.object, "visiting");

    if (entry.entity.parentName !== null) {
      const parent = nameMap.get(entry.entity.parentName);
      if (!parent) {
        throw new Error(`scene error: parent '${entry.entity.parentName}' not found`);
      }

      if (parent !== entry.object) {
        const parentEntry = entryByObject.get(parent);
        if (parentEntry) {
          attachEntry(parentEntry);
        }
        parent.addChild(entry.object);
      }
    }

    attachmentState.set(entry.object, "attached");
    return entry.object.parent;
  }

  const topLevel = [];
  for (const entry of entries) {
    const parent = attachEntry(entry);
    if (!parent) {
      topLevel.push(entry.object);
    }
  }

  applyFallbackLayout(topLevel);

  let root;
  if (topLevel.length === 1) {
    root = topLevel[0];
    root.metadata.scene.sceneName = sanitizeQuotedText(
      normalizedScene.name,
      root.name,
    );
    root.metadata.scene.runtimeSlice = normalizeRuntimeSlice(normalizedScene.runtimeSlice);
  } else {
    root = createSyntheticRoot(
      normalizedScene.name,
      topLevel.length,
      normalizedScene.runtimeSlice,
    );
    for (const object of topLevel) {
      root.addChild(object);
    }
  }

  const tree = new MorphicTree(root);
  tree.sceneName = sanitizeQuotedText(normalizedScene.name, root.name ?? "Imported Scene");
  return tree;
}

export function importSceneToTree(source) {
  return sceneDataToTree(parseScene(source));
}
