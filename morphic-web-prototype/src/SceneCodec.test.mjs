import assert from "node:assert/strict";
import fs from "node:fs";
import test from "node:test";

import { MorphicObject } from "./MorphicObject.js";
import {
  exportTreeToScene,
  importSceneToTree,
  parseScene,
} from "./SceneCodec.js";

test("browser codec imports the shipped Rust demo scene", () => {
  const source = fs.readFileSync(new URL("../../examples/demo.hsc", import.meta.url), "utf8");

  assert.doesNotThrow(() => importSceneToTree(source));
});

test("browser codec round-trips runtime_slice and richer entity components", () => {
  const source = fs.readFileSync(new URL("../../examples/demo.hsc", import.meta.url), "utf8");
  const original = parseScene(source);
  const tree = importSceneToTree(source);
  const roundtrip = parseScene(exportTreeToScene(tree));

  assert.deepEqual(roundtrip.runtimeSlice, original.runtimeSlice);

  const originalFx = original.entities.find((entity) => entity.name === "model_fx");
  const roundtripFx = roundtrip.entities.find((entity) => entity.name === "model_fx");

  assert.ok(originalFx?.particleEmitter);
  assert.ok(roundtripFx?.particleEmitter);
  assert.deepEqual(roundtripFx.particleEmitter, originalFx.particleEmitter);
});

test("single-root scenes keep runtime_slice attached to the exported scene", () => {
  const source = `scene "Single Root"

runtime_slice {
  patrol_route "route_a" {
    looped 1
    point 1 2 3
  }
}

entity "camera" {
  transform {
    pos 0 2 10
    rot 0 0 0 1
    scale 1 1 1
  }
  camera { fov 60 near 0.1 far 1000 }
  active_camera
}
`;

  const tree = importSceneToTree(source);
  const roundtrip = parseScene(exportTreeToScene(tree));

  assert.equal(roundtrip.runtimeSlice.patrolRoutes.length, 1);
  assert.equal(roundtrip.runtimeSlice.patrolRoutes[0].name, "route_a");
  assert.equal(roundtrip.entities.length, 1);
  assert.equal(roundtrip.entities[0].name, "camera");
});

test("moving imported morphs keeps authored transforms aligned for export", () => {
  const source = `scene "Duplicate"

entity "box" {
  transform {
    pos 10 20 30
    rot 0 0 0 1
    scale 1 1 1
  }
}

entity "light" {
  transform {
    pos 40 50 60
    rot 0 0 0 1
    scale 1 1 1
  }
  light {
    dir 0 -1 0
    color 1 1 1 1
    intensity 1
    ambient 0.2
  }
}
`;

  const tree = importSceneToTree(source);
  const original = tree.root.children.find((object) => object.name === "box");
  const clone = MorphicObject.fromJSON(original.toJSON(), { preserveIds: false });
  clone.name = "box Copy";
  clone.move({
    x: original.position.x + 26,
    y: original.position.y + 26,
  });
  tree.root.addChild(clone);

  const roundtrip = parseScene(exportTreeToScene(tree));
  const exportedOriginal = roundtrip.entities.find((entity) => entity.name === "box");
  const exportedCopy = roundtrip.entities.find((entity) => entity.name === "box Copy");

  assert.equal(exportedCopy.transform.position.x, exportedOriginal.transform.position.x + 26);
  assert.equal(exportedCopy.transform.position.y, exportedOriginal.transform.position.y + 26);
  assert.equal(exportedCopy.transform.position.z, exportedOriginal.transform.position.z);
});

test("morphic object JSON cloning isolates nested scene metadata", () => {
  const object = new MorphicObject({
    metadata: {
      scene: {
        transform: {
          position: { x: 1, y: 2, z: 3 },
        },
      },
    },
  });

  const clone = MorphicObject.fromJSON(object.toJSON(), { preserveIds: false });
  clone.metadata.scene.transform.position.x = 99;

  assert.equal(object.metadata.scene.transform.position.x, 1);
});

test("browser codec rejects Rust-invalid bool words", () => {
  const source = `scene "Bool Drift"

runtime_slice {
  patrol_route "route_a" {
    looped true
    point 1 2 3
  }
}
`;

  assert.throws(
    () => importSceneToTree(source),
    /expected bool flag/,
  );
});

test("browser codec rejects unknown spawn point kinds like the Rust parser", () => {
  const source = `scene "Spawn Kind"

runtime_slice {
  spawn_point "door" {
    kind "boss"
    pos 1 2 3
    yaw 45
  }
}

entity "camera" {
  transform {
    pos 0 2 10
    rot 0 0 0 1
    scale 1 1 1
  }
  camera { fov 60 near 0.1 far 1000 }
}
`;

  assert.throws(
    () => importSceneToTree(source),
    /unknown spawn_point kind/,
  );
});

test("browser codec preserves empty quoted names that the Rust parser allows", () => {
  const source = `scene ""

runtime_slice {
  spawn_point "" {
    kind "generic"
    pos 1 2 3
    yaw 0
  }
}

entity "" {
  transform {
    pos 1 2 3
    rot 0 0 0 1
    scale 1 1 1
  }
}
`;

  const roundtrip = parseScene(exportTreeToScene(importSceneToTree(source)));

  assert.equal(roundtrip.name, "");
  assert.equal(roundtrip.runtimeSlice.spawnPoints.length, 1);
  assert.equal(roundtrip.runtimeSlice.spawnPoints[0].name, "");
  assert.equal(roundtrip.entities.length, 1);
  assert.equal(roundtrip.entities[0].name, "");
});

test("browser codec rejects unresolved parent references like the Rust loader", () => {
  const source = `scene "Bad Parent"

entity "child" {
  transform {
    pos 1 2 3
    rot 0 0 0 1
    scale 1 1 1
  }
  parent "missing"
}
`;

  assert.throws(
    () => importSceneToTree(source),
    /parent 'missing' not found/,
  );
});

test("browser codec rejects cyclic parent relationships instead of dropping entities", () => {
  const source = `scene "Cycle"

entity "a" {
  transform {
    pos 0 0 0
    rot 0 0 0 1
    scale 1 1 1
  }
  parent "b"
}

entity "b" {
  transform {
    pos 1 0 0
    rot 0 0 0 1
    scale 1 1 1
  }
  parent "a"
}
`;

  assert.throws(
    () => importSceneToTree(source),
    /cyclic parent relationship/,
  );
});
