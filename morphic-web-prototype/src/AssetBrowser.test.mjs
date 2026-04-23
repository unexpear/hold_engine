import test from "node:test";
import assert from "node:assert/strict";

import {
  defaultAssetSelection,
  isBitmapPreviewablePath,
  parseMaterialAlbedoLogicalName,
  resolveLogicalTextureProjectPath,
} from "./AssetBrowser.js";

test("bitmap preview detection includes bmp assets", () => {
  assert.equal(isBitmapPreviewablePath("examples/source/textures/crate.bmp"), true);
  assert.equal(isBitmapPreviewablePath("examples/source/textures/crate.tga"), true);
});

test("material albedo parsing ignores comments and none markers", () => {
  assert.equal(
    parseMaterialAlbedoLogicalName("# comment\nalbedo textures/sand"),
    "textures/sand",
  );
  assert.equal(parseMaterialAlbedoLogicalName("albedo none"), null);
});

test("logical texture resolution finds matching browser entries", () => {
  const entries = [
    { kind: "file", projectPath: "examples/source/textures/crate.bmp" },
    { kind: "file", projectPath: "examples/source/materials/crate.hmat" },
  ];

  assert.equal(
    resolveLogicalTextureProjectPath(entries, "textures/crate"),
    "examples/source/textures/crate.bmp",
  );
});

test("default asset selection prefers previewable bitmap files", () => {
  const model = {
    entries: [
      { root: "assets", kind: "file", projectPath: "examples/source/materials/crate.hmat" },
      { root: "assets", kind: "file", projectPath: "examples/source/textures/crate.bmp" },
    ],
  };

  assert.equal(defaultAssetSelection(model), "examples/source/textures/crate.bmp");
});
