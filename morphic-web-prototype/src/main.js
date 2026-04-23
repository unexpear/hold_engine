import { BackendClient } from "./BackendClient.js";
import { BrowserEditorSession } from "./BrowserEditorSession.js";
import { ContextMenu } from "./ContextMenu.js";
import { EditorProtocol } from "./EditorProtocol.js";
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

function createDefaultTree() {
  const tree = new MorphicTree(createScene());
  tree.sceneName = "Unified Branch Demo";
  return tree;
}

async function loadInitialTree() {
  const backendClient = await BackendClient.connect();
  if (!backendClient?.session?.sourceText) {
    return {
      backendClient: null,
      tree: createDefaultTree(),
    };
  }

  try {
    const tree = importSceneToTree(backendClient.session.sourceText);
    return {
      backendClient,
      tree,
    };
  } catch (error) {
    console.error("Failed to import the backend scene into the browser prototype.", error);
    return {
      backendClient: null,
      tree: createDefaultTree(),
    };
  }
}

function isEditableTarget(target) {
  return (
    target instanceof HTMLElement &&
    (target.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName))
  );
}

async function boot() {
  const container = document.getElementById("demo-root");
  const { backendClient, tree: initialTree } = await loadInitialTree();

  MorphicRegistry.instance.reset();
  MorphicRegistry.instance.registerTree(initialTree.root);

  const renderer = new Renderer(container, initialTree);
  window.morphic = {};

  function replaceTree(nextTree, selectedId = nextTree.root.id) {
    MorphicRegistry.instance.reset();
    MorphicRegistry.instance.registerTree(nextTree.root);
    renderer.tree = nextTree;
    window.morphic.tree = nextTree;
    const nextSelection = nextTree.findById(selectedId) || nextTree.root;
    renderer.select(nextSelection);
  }

  const scene = {
    exportText() {
      return exportTreeToScene(renderer.tree, {
        sceneName: renderer.tree.sceneName ?? getSceneName(renderer.tree),
      });
    },
    importText(source) {
      const nextTree = importSceneToTree(source);
      replaceTree(nextTree);
      return nextTree;
    },
  };

  const editorSession = new BrowserEditorSession({
    renderer,
    exportSceneText: () => scene.exportText(),
    replaceTree,
    saveSceneText: backendClient
      ? (sourceText) => backendClient.saveSceneText(sourceText)
      : null,
  });

  new ContextMenu(renderer, {
    applyAuthoredChange: (mutator) => editorSession.applyAuthoredChange(mutator),
  });
  const inspector = new Inspector(renderer, scene, { editorSession, backendClient });
  new ScriptingConsole({ renderer, sceneBridge: scene });

  renderer.select(initialTree.root);

  document.addEventListener("keydown", async (event) => {
    if (!(event.ctrlKey || event.metaKey) || event.altKey || isEditableTarget(event.target)) {
      return;
    }

    const key = event.key.toLowerCase();

    try {
      if (key === "s") {
        event.preventDefault();
        await inspector.saveButton.onclick();
        return;
      }

      if (key === "z" && !event.shiftKey) {
        event.preventDefault();
        inspector.undoButton.onclick();
        return;
      }

      if (key === "y" || (key === "z" && event.shiftKey)) {
        event.preventDefault();
        inspector.redoButton.onclick();
      }
    } catch (error) {
      event.preventDefault();
      inspector.setStatus(error.message, true);
    }
  });

  Object.assign(window.morphic, {
    authorEdit(mutator) {
      return editorSession.applyAuthoredChange(mutator);
    },
    backend: backendClient,
    editorProtocol: EditorProtocol,
    editorSession,
    renderer,
    saveScene() {
      return editorSession.save();
    },
    tree: renderer.tree,
    undoScene() {
      return editorSession.undo();
    },
    redoScene() {
      return editorSession.redo();
    },
    registry: MorphicRegistry.instance,
    MorphicObject,
    scene,
  });
}

boot().catch((error) => {
  console.error("Morphic browser editor failed to boot.", error);
});
