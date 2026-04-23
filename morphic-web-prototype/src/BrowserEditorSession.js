import { EditorProtocol } from "./EditorProtocol.js";
import { getSceneName } from "./SceneCodec.js";
import { MorphicTree } from "./MorphicTree.js";

function sanitizeDownloadName(sceneName) {
  const fallback = "morphic-scene";
  const trimmed = String(sceneName ?? "").trim();
  const baseName = trimmed || fallback;
  const sanitized = baseName.replace(/[<>:"/\\|?*\u0000-\u001f]/g, "-");
  return sanitized || fallback;
}

function downloadTextFile(text, filename) {
  const blob = new Blob([text], { type: "text/plain;charset=utf-8" });
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = url;
  link.download = filename;
  link.style.display = "none";
  document.body.appendChild(link);
  link.click();
  document.body.removeChild(link);
  window.setTimeout(() => URL.revokeObjectURL(url), 1000);
}

export class BrowserEditorSession {
  constructor({ renderer, exportSceneText, replaceTree, saveSceneText = null }) {
    if (!renderer) {
      throw new TypeError("BrowserEditorSession requires a renderer.");
    }
    if (typeof exportSceneText !== "function") {
      throw new TypeError("BrowserEditorSession requires an exportSceneText callback.");
    }
    if (typeof replaceTree !== "function") {
      throw new TypeError("BrowserEditorSession requires a replaceTree callback.");
    }

    this.renderer = renderer;
    this.exportSceneText = exportSceneText;
    this.replaceTree = replaceTree;
    this.saveSceneText = typeof saveSceneText === "function" ? saveSceneText : null;
    this.pendingSourceStateProvider = () => false;
    this.undoStack = [];
    this.redoStack = [];
    this.dirty = false;
  }

  setPendingSourceStateProvider(provider) {
    this.pendingSourceStateProvider = typeof provider === "function" ? provider : () => false;
    this.dispatchState();
  }

  hasPendingSourceDraft() {
    return Boolean(this.pendingSourceStateProvider?.());
  }

  canUndo() {
    return this.undoStack.length > 0;
  }

  canRedo() {
    return this.redoStack.length > 0;
  }

  isDirty() {
    return this.dirty;
  }

  getState() {
    return {
      session: this,
      dirty: this.isDirty(),
      canUndo: this.canUndo(),
      canRedo: this.canRedo(),
      hasPendingSourceDraft: this.hasPendingSourceDraft(),
    };
  }

  captureSnapshot() {
    return {
      tree: this.renderer.tree.toJSON(),
      sceneName:
        this.renderer.tree.sceneName
        ?? this.renderer.tree.root?.metadata?.scene?.sceneName
        ?? "",
      selectedId: this.renderer.selected?.id ?? this.renderer.tree.root.id,
      dirty: this.dirty,
    };
  }

  restoreSnapshot(snapshot) {
    const nextTree = MorphicTree.fromJSON(snapshot.tree, { preserveIds: true });
    nextTree.sceneName = snapshot.sceneName;
    this.replaceTree(nextTree, snapshot.selectedId);
    this.dirty = Boolean(snapshot.dirty);
    this.dispatchState();
  }

  applyAuthoredChange(mutator) {
    if (typeof mutator !== "function") {
      throw new TypeError("Authored changes require a mutator function.");
    }

    const snapshot = this.captureSnapshot();
    const result = mutator();
    this.undoStack.push(snapshot);
    this.redoStack = [];
    this.dirty = true;
    this.dispatchState();
    return result;
  }

  async save() {
    this.ensureAuthoritativeScene("save");
    const sourceText = this.exportSceneText();
    const filename = `${sanitizeDownloadName(getSceneName(this.renderer.tree))}.hsc`;

    let result;
    if (this.saveSceneText) {
      const saved = await this.saveSceneText(sourceText, filename);
      result = {
        mode: "backend",
        filename,
        sourcePath: saved?.sourcePath ?? "",
        sourceText: saved?.sourceText ?? sourceText,
      };
    } else {
      downloadTextFile(sourceText, filename);
      result = {
        mode: "download",
        filename,
        sourcePath: "",
        sourceText,
      };
    }

    this.dirty = false;
    this.dispatchProtocolEvent({ type: "Saved" });
    this.dispatchState();
    return result;
  }

  undo() {
    this.ensureAuthoritativeScene("undo");
    const snapshot = this.undoStack.pop();
    if (!snapshot) {
      throw new Error("Undo history is empty.");
    }

    this.redoStack.push(this.captureSnapshot());
    this.restoreSnapshot(snapshot);
    this.dispatchProtocolEvent({ type: "UndoApplied" });
  }

  redo() {
    this.ensureAuthoritativeScene("redo");
    const snapshot = this.redoStack.pop();
    if (!snapshot) {
      throw new Error("Redo history is empty.");
    }

    this.undoStack.push(this.captureSnapshot());
    this.restoreSnapshot(snapshot);
    this.dispatchProtocolEvent({ type: "RedoApplied" });
  }

  ensureAuthoritativeScene(command) {
    if (this.hasPendingSourceDraft()) {
      throw new Error(
        `The .hsc buffer has unapplied changes. Load or refresh it before ${command}.`,
      );
    }
  }

  dispatchProtocolEvent(event) {
    window.dispatchEvent(
      new CustomEvent("morphic-editor-event", {
        detail: {
          session: this,
          event,
          encoded: EditorProtocol.encodeEvent(event),
        },
      }),
    );
  }

  dispatchState() {
    window.dispatchEvent(
      new CustomEvent("morphic-editor-state", {
        detail: this.getState(),
      }),
    );
  }
}
