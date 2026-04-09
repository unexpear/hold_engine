import { ContextMenu } from "./ContextMenu.js";
import { Inspector } from "./Inspector.js";
import { MorphicObject } from "./MorphicObject.js";
import { MorphicRegistry } from "./MorphicRegistry.js";
import { MorphicTree } from "./MorphicTree.js";
import { Renderer } from "./Renderer.js";
import { exportTreeToScene, getSceneName, importSceneToTree } from "./SceneCodec.js";
import { ScriptingConsole } from "./ScriptingConsole.js";

function createScene() {
  const root = new MorphicObject({
    name: "Command Deck",
    position: { x: 430, y: 160 },
    size: { width: 360, height: 240 },
    style: {
      background: "linear-gradient(145deg, #5e6bf2, #253570)",
    },
    metadata: {
      role: "root",
      source: "unified-branch-demo",
    },
    text: "A high-level morph that represents the combined branch workspace.",
  });

  root.addChild(
    new MorphicObject({
      name: "Workbench",
      position: { x: 34, y: 58 },
      size: { width: 150, height: 108 },
      style: {
        background: "linear-gradient(145deg, #f6b73c, #f39c5f)",
      },
      metadata: {
        purpose: "editing",
      },
      text: "Prototype tools live here: inspect, duplicate, and script.",
    }),
  );

  root.addChild(
    new MorphicObject({
      name: "Runtime View",
      position: { x: 192, y: 96 },
      size: { width: 132, height: 92 },
      style: {
        background: "linear-gradient(145deg, #28c7fa, #2558b6)",
      },
      metadata: {
        purpose: "rendering",
      },
      text: "A lightweight view standing in for the Rust renderer side.",
    }),
  );

  return root;
}

const container = document.getElementById("demo-root");
const root = createScene();
const tree = new MorphicTree(root);
tree.sceneName = "Unified Branch Demo";

MorphicRegistry.instance.reset();
MorphicRegistry.instance.registerTree(root);

const renderer = new Renderer(container, tree);
window.morphic = {};

function replaceTree(nextTree) {
  MorphicRegistry.instance.reset();
  MorphicRegistry.instance.registerTree(nextTree.root);
  renderer.tree = nextTree;
  window.morphic.tree = nextTree;
  renderer.select(nextTree.root);
}

const scene = {
  exportText() {
    return exportTreeToScene(renderer.tree, {
      sceneName: renderer.tree.sceneName || getSceneName(renderer.tree),
    });
  },
  importText(source) {
    const nextTree = importSceneToTree(source);
    replaceTree(nextTree);
    return nextTree;
  },
};

new ContextMenu(renderer);
new Inspector(renderer, scene);
new ScriptingConsole({ renderer, sceneBridge: scene });

renderer.select(root);

Object.assign(window.morphic, {
  renderer,
  tree,
  registry: MorphicRegistry.instance,
  MorphicObject,
  scene,
});
