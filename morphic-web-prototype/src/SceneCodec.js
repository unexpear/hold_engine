import { MorphicObject } from "./MorphicObject.js";
import { MorphicTree } from "./MorphicTree.js";

const IDENTITY_ROTATION = Object.freeze({ x: 0, y: 0, z: 0, w: 1 });
const UNIT_SCALE = Object.freeze({ x: 1, y: 1, z: 1 });

function toNumber(value, fallback = 0) {
  const number = Number(value);
  return Number.isFinite(number) ? number : fallback;
}

function cloneVec3(value = {}, fallback = {}) {
  return {
    x: toNumber(value.x, fallback.x ?? 0),
    y: toNumber(value.y, fallback.y ?? 0),
    z: toNumber(value.z, fallback.z ?? 0),
  };
}

function cloneQuat(value = {}, fallback = IDENTITY_ROTATION) {
  return {
    x: toNumber(value.x, fallback.x),
    y: toNumber(value.y, fallback.y),
    z: toNumber(value.z, fallback.z),
    w: toNumber(value.w, fallback.w),
  };
}

function normalizeTransform(transform, fallbackPosition = {}) {
  return {
    position: cloneVec3(transform?.position, {
      x: fallbackPosition.x ?? 0,
      y: fallbackPosition.y ?? 0,
      z: 0,
    }),
    rotation: cloneQuat(transform?.rotation, IDENTITY_ROTATION),
    scale: cloneVec3(transform?.scale, UNIT_SCALE),
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
    color: {
      r: toNumber(light.color?.r, 1),
      g: toNumber(light.color?.g, 1),
      b: toNumber(light.color?.b, 1),
      a: toNumber(light.color?.a, 1),
    },
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

function sceneMetadataForObject(object) {
  const scene = object.metadata?.scene ?? {};
  return {
    transform: normalizeTransform(scene.transform, {
      x: object.position?.x ?? 0,
      y: object.position?.y ?? 0,
    }),
    camera: normalizeCamera(scene.camera),
    activeCamera: Boolean(scene.activeCamera),
    light: normalizeLight(scene.light),
    stats: normalizeStats(scene.stats),
  };
}

function sanitizeQuotedText(value, fallback) {
  const text = String(value ?? fallback ?? "").trim();
  const safeText = text.replaceAll('"', "'");
  return safeText || String(fallback ?? "").replaceAll('"', "'");
}

function formatNumber(value) {
  const number = toNumber(value, 0);

  if (Math.abs(number - Math.trunc(number)) < 1e-9) {
    return String(Math.trunc(number));
  }

  return number.toFixed(6).replace(/\.?0+$/, "");
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

  parseScene() {
    const scene = {
      name: "",
      entities: [],
    };

    if (this.peek()?.type === "word" && this.peek().value === "scene") {
      this.next();
      scene.name = this.expect("string").value;
    }

    while (this.peek()) {
      scene.entities.push(this.parseEntity());
    }

    return scene;
  }

  parseEntity() {
    this.expect("word", "entity");
    const entity = {
      name: this.expect("string").value,
      transform: null,
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
          entity.parentName = this.expect("string").value;
          break;
        default:
          throw new Error(`scene error (line ${token.line}): unknown entity component '${token.value}'`);
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
          transform.position = {
            x: this.expect("number").value,
            y: this.expect("number").value,
            z: this.expect("number").value,
          };
          break;
        case "rot":
        case "rotation":
          transform.rotation = {
            x: this.expect("number").value,
            y: this.expect("number").value,
            z: this.expect("number").value,
            w: this.expect("number").value,
          };
          break;
        case "scale":
          transform.scale = {
            x: this.expect("number").value,
            y: this.expect("number").value,
            z: this.expect("number").value,
          };
          break;
        case "euler":
          transform.rotation = quatFromEulerDegrees(
            this.expect("number").value,
            this.expect("number").value,
            this.expect("number").value,
          );
          break;
        default:
          throw new Error(`scene error (line ${token.line}): unknown transform field '${token.value}'`);
      }
    }

    this.expect("brace", "}");
    return transform;
  }

  parseCamera() {
    const camera = normalizeCamera({});
    this.expect("brace", "{");

    while (this.peek() && !(this.peek().type === "brace" && this.peek().value === "}")) {
      const token = this.expect("word");
      switch (token.value) {
        case "fov":
          camera.fov = this.expect("number").value;
          break;
        case "near":
          camera.near = this.expect("number").value;
          break;
        case "far":
          camera.far = this.expect("number").value;
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
          light.direction = {
            x: this.expect("number").value,
            y: this.expect("number").value,
            z: this.expect("number").value,
          };
          break;
        case "color":
          light.color = {
            r: this.expect("number").value,
            g: this.expect("number").value,
            b: this.expect("number").value,
            a: this.expect("number").value,
          };
          break;
        case "intensity":
          light.intensity = this.expect("number").value;
          break;
        case "ambient":
          light.ambient = this.expect("number").value;
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
          stats.health = this.expect("number").value;
          break;
        case "mana":
          stats.mana = this.expect("number").value;
          break;
        case "attack":
          stats.attack = this.expect("number").value;
          break;
        case "defense":
          stats.defense = this.expect("number").value;
          break;
        case "speed":
          stats.speed = this.expect("number").value;
          break;
        default:
          throw new Error(`scene error (line ${token.line}): unknown stats field '${token.value}'`);
      }
    }

    this.expect("brace", "}");
    return stats;
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

  if (!lines.length) {
    lines.push("Scene entity imported from .hsc.");
  }

  return lines.join("\n");
}

function createSyntheticRoot(sceneName, topLevelCount) {
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
        sceneName: sceneName || "Imported Scene",
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
  const sceneName = sanitizeQuotedText(scene.name, "");

  if (sceneName) {
    lines.push(`scene "${sceneName}"`, "");
  }

  for (const entity of scene.entities) {
    lines.push(`entity "${sanitizeQuotedText(entity.name, "Entity")}" {`);

    if (entity.transform) {
      lines.push("  transform {");
      lines.push(
        `    pos ${formatNumber(entity.transform.position.x)} ${formatNumber(entity.transform.position.y)} ${formatNumber(entity.transform.position.z)}`,
      );
      lines.push(
        `    rot ${formatNumber(entity.transform.rotation.x)} ${formatNumber(entity.transform.rotation.y)} ${formatNumber(entity.transform.rotation.z)} ${formatNumber(entity.transform.rotation.w)}`,
      );
      lines.push(
        `    scale ${formatNumber(entity.transform.scale.x)} ${formatNumber(entity.transform.scale.y)} ${formatNumber(entity.transform.scale.z)}`,
      );
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
      lines.push(
        `    dir ${formatNumber(entity.light.direction.x)} ${formatNumber(entity.light.direction.y)} ${formatNumber(entity.light.direction.z)}`,
      );
      lines.push(
        `    color ${formatNumber(entity.light.color.r)} ${formatNumber(entity.light.color.g)} ${formatNumber(entity.light.color.b)} ${formatNumber(entity.light.color.a)}`,
      );
      lines.push(`    intensity ${formatNumber(entity.light.intensity)}`);
      lines.push(`    ambient ${formatNumber(entity.light.ambient)}`);
      lines.push("  }");
    }

    if (entity.stats) {
      lines.push(
        `  stats { health ${formatNumber(entity.stats.health)} mana ${formatNumber(entity.stats.mana)} attack ${formatNumber(entity.stats.attack)} defense ${formatNumber(entity.stats.defense)} speed ${formatNumber(entity.stats.speed)} }`,
      );
    }

    if (entity.parentName) {
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

  return {
    name: sanitizeQuotedText(options.sceneName || getSceneName(tree), "Morphic Scene"),
    entities: entries.map(({ object, parent }) => {
      const scene = sceneMetadataForObject(object);
      return {
        name: nameMap.get(object.id),
        transform: scene.transform,
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
  const entries = scene.entities.map((entity, index) => ({
    entity,
    object: createObjectFromEntity(entity, index),
  }));

  const nameMap = new Map();
  for (const entry of entries) {
    nameMap.set(entry.entity.name, entry.object);
  }

  const topLevel = [];
  for (const entry of entries) {
    const parent = entry.entity.parentName ? nameMap.get(entry.entity.parentName) : null;
    if (parent && parent !== entry.object) {
      parent.addChild(entry.object);
    } else {
      topLevel.push(entry.object);
    }
  }

  applyFallbackLayout(topLevel);

  let root;
  if (topLevel.length === 1) {
    root = topLevel[0];
    root.metadata.scene.sceneName = sanitizeQuotedText(scene.name || root.name, root.name);
  } else {
    root = createSyntheticRoot(scene.name, topLevel.length);
    for (const object of topLevel) {
      root.addChild(object);
    }
  }

  const tree = new MorphicTree(root);
  tree.sceneName = sanitizeQuotedText(scene.name || root.name, "Imported Scene");
  return tree;
}

export function importSceneToTree(source) {
  return sceneDataToTree(parseScene(source));
}
