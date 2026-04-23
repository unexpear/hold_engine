import {
  defaultAssetSelection,
  isBitmapPreviewablePath,
  isTextPreviewablePath,
  parseMaterialAlbedoLogicalName,
  resolveLogicalTextureProjectPath,
} from "./AssetBrowser.js";

async function copyText(text) {
  if (navigator.clipboard?.writeText) {
    await navigator.clipboard.writeText(text);
    return;
  }

  const buffer = document.createElement("textarea");
  buffer.value = text;
  buffer.setAttribute("readonly", "readonly");
  buffer.style.position = "fixed";
  buffer.style.opacity = "0";
  document.body.appendChild(buffer);
  buffer.select();
  document.execCommand("copy");
  document.body.removeChild(buffer);
}

function makeButton(label, background) {
  const button = document.createElement("button");
  button.type = "button";
  button.textContent = label;
  button.style.padding = "10px 12px";
  button.style.border = "0";
  button.style.borderRadius = "999px";
  button.style.background = background;
  button.style.color = "#f6ecd9";
  button.style.cursor = "pointer";
  return button;
}

function setButtonDisabled(button, disabled) {
  if (!button) {
    return;
  }

  button.disabled = disabled;
  button.style.opacity = disabled ? "0.46" : "1";
  button.style.cursor = disabled ? "not-allowed" : "pointer";
}

export class Inspector {
  constructor(renderer, sceneBridge, { editorSession, backendClient } = {}) {
    this.renderer = renderer;
    this.sceneBridge = sceneBridge;
    this.editorSession = editorSession ?? null;
    this.backendClient = backendClient ?? null;
    this.sceneDirty = false;
    this.assetBrowserModel = null;
    this.selectedAssetPath = "";
    this.assetPreviewToken = 0;
    this.panel = document.createElement("aside");
    this.panel.style.position = "fixed";
    this.panel.style.right = "18px";
    this.panel.style.top = "18px";
    this.panel.style.width = "min(380px, calc(100vw - 36px))";
    this.panel.style.maxHeight = "calc(100vh - 36px)";
    this.panel.style.overflow = "auto";
    this.panel.style.padding = "16px";
    this.panel.style.borderRadius = "18px";
    this.panel.style.border = "1px solid rgba(255, 255, 255, 0.12)";
    this.panel.style.background = "rgba(19, 23, 31, 0.86)";
    this.panel.style.color = "#f6ecd9";
    this.panel.style.boxShadow = "0 24px 60px rgba(0, 0, 0, 0.34)";
    this.panel.style.backdropFilter = "blur(16px)";
    this.panel.style.zIndex = "9999";
    this.panel.style.fontFamily = '"Cascadia Code", Consolas, monospace';

    const title = document.createElement("h2");
    title.textContent = "Inspector";
    title.style.margin = "0 0 10px";
    title.style.fontSize = "1rem";
    title.style.fontFamily = '"Aptos", "Trebuchet MS", sans-serif';
    title.style.letterSpacing = "0.08em";
    title.style.textTransform = "uppercase";

    const hint = document.createElement("p");
    hint.textContent = "Selection details update live.";
    hint.style.margin = "0 0 12px";
    hint.style.color = "rgba(246, 236, 217, 0.7)";
    hint.style.fontSize = "0.88rem";
    hint.style.fontFamily = '"Aptos", "Trebuchet MS", sans-serif';

    const commandRow = document.createElement("div");
    commandRow.style.display = "flex";
    commandRow.style.flexWrap = "wrap";
    commandRow.style.gap = "8px";
    commandRow.style.marginBottom = "10px";

    this.saveButton = makeButton("Save .hsc", "rgba(87, 213, 140, 0.18)");
    this.saveButton.onclick = async () => {
      if (!this.editorSession) {
        return;
      }

      try {
        const result = await this.editorSession.save();
        this.sceneDirty = false;
        this.sceneSource.value = result.sourceText;
        this.syncCommandButtons();
        if (result.mode === "backend" && result.sourcePath) {
          this.setStatus(`Saved ${result.sourcePath}.`);
        } else {
          this.setStatus(`Downloaded ${result.filename}.`);
        }
      } catch (error) {
        this.setStatus(error.message, true);
      }
    };

    this.undoButton = makeButton("Undo", "rgba(243, 179, 95, 0.18)");
    this.undoButton.onclick = () => {
      if (!this.editorSession) {
        return;
      }

      try {
        this.editorSession.undo();
        this.setStatus("Undo restored the previous authored snapshot.");
      } catch (error) {
        this.setStatus(error.message, true);
      }
    };

    this.redoButton = makeButton("Redo", "rgba(116, 208, 255, 0.16)");
    this.redoButton.onclick = () => {
      if (!this.editorSession) {
        return;
      }

      try {
        this.editorSession.redo();
        this.setStatus("Redo restored the later authored snapshot.");
      } catch (error) {
        this.setStatus(error.message, true);
      }
    };

    this.openSceneButton = makeButton("Open .hsc", "rgba(160, 132, 255, 0.18)");

    commandRow.append(this.saveButton, this.undoButton, this.redoButton, this.openSceneButton);

    const buttonRow = document.createElement("div");
    buttonRow.style.display = "flex";
    buttonRow.style.flexWrap = "wrap";
    buttonRow.style.gap = "8px";
    buttonRow.style.marginBottom = "12px";

    const exportButton = makeButton("Copy Tree JSON", "rgba(243, 179, 95, 0.18)");
    exportButton.onclick = async () => {
      await copyText(JSON.stringify(this.renderer.tree.toJSON(), null, 2));
      this.setStatus("Copied tree JSON.");
    };

    const copySceneButton = makeButton("Copy .hsc", "rgba(116, 208, 255, 0.16)");
    copySceneButton.onclick = async () => {
      await copyText(this.sceneSource.value);
      this.setStatus("Copied the current .hsc buffer.");
    };

    const refreshSceneButton = makeButton("Refresh .hsc", "rgba(255, 255, 255, 0.08)");
    refreshSceneButton.onclick = () => {
      this.refreshSceneSource(true);
      this.setStatus("Scene text refreshed from the current tree.");
    };

    const loadSceneButton = makeButton("Load .hsc", "rgba(87, 213, 140, 0.18)");
    loadSceneButton.onclick = async () => {
      try {
        const nextSource = this.backendClient
          ? await this.backendClient.applySceneText(this.sceneSource.value)
          : this.sceneSource.value;
        if (this.editorSession) {
          this.editorSession.applyAuthoredChange(() => {
            this.sceneBridge.importText(nextSource);
          });
        } else {
          this.sceneBridge.importText(nextSource);
        }
        this.sceneSource.value = nextSource;
        this.sceneDirty = false;
        this.syncCommandButtons();
        this.setStatus(
          this.backendClient
            ? "Imported .hsc scene into the browser prototype and synced the Rust backend."
            : "Imported .hsc scene into the browser prototype.",
        );
      } catch (error) {
        this.setStatus(error.message, true);
      }
    };

    buttonRow.append(exportButton, copySceneButton, refreshSceneButton, loadSceneButton);

    const sceneTitle = document.createElement("h3");
    sceneTitle.textContent = "Rust Scene Bridge (.hsc)";
    sceneTitle.style.margin = "0 0 8px";
    sceneTitle.style.fontSize = "0.96rem";
    sceneTitle.style.fontFamily = '"Aptos", "Trebuchet MS", sans-serif';

    const sceneHint = document.createElement("p");
    sceneHint.textContent = "Edit the shared scene text here, then load it back into the Morphic tree.";
    sceneHint.style.margin = "0 0 8px";
    sceneHint.style.color = "rgba(246, 236, 217, 0.7)";
    sceneHint.style.fontSize = "0.85rem";
    sceneHint.style.fontFamily = '"Aptos", "Trebuchet MS", sans-serif';

    this.sceneSource = document.createElement("textarea");
    this.sceneSource.rows = 12;
    this.sceneSource.spellcheck = false;
    this.sceneSource.style.width = "100%";
    this.sceneSource.style.resize = "vertical";
    this.sceneSource.style.marginBottom = "8px";
    this.sceneSource.style.padding = "12px";
    this.sceneSource.style.border = "1px solid rgba(255, 255, 255, 0.08)";
    this.sceneSource.style.borderRadius = "14px";
    this.sceneSource.style.background = "rgba(255, 255, 255, 0.04)";
    this.sceneSource.style.color = "#f6ecd9";
    this.sceneSource.style.font = "inherit";
    this.sceneSource.style.fontSize = "0.8rem";
    this.sceneSource.style.lineHeight = "1.45";
    this.sceneSource.addEventListener("input", () => {
      this.sceneDirty = true;
      this.setStatus("Scene text changed locally. Refresh to discard or load to apply.");
      this.syncCommandButtons();
    });

    this.sceneFileInput = document.createElement("input");
    this.sceneFileInput.type = "file";
    this.sceneFileInput.accept = ".hsc,text/plain";
    this.sceneFileInput.hidden = true;
    this.sceneFileInput.addEventListener("change", async () => {
      const [file] = Array.from(this.sceneFileInput.files ?? []);
      if (!file) {
        return;
      }

      try {
        this.sceneSource.value = await file.text();
        this.sceneDirty = true;
        this.setStatus(`Loaded ${file.name} into the .hsc buffer. Click Load .hsc to apply.`);
      } catch (error) {
        this.setStatus(`Failed to read ${file.name}: ${error.message}`, true);
      } finally {
        this.sceneFileInput.value = "";
        this.syncCommandButtons();
      }
    });

    this.openSceneButton.onclick = () => {
      this.sceneFileInput.click();
    };

    const assetTitle = document.createElement("h3");
    assetTitle.textContent = "Project Assets";
    assetTitle.style.margin = "12px 0 8px";
    assetTitle.style.fontSize = "0.96rem";
    assetTitle.style.fontFamily = '"Aptos", "Trebuchet MS", sans-serif';

    const assetHint = document.createElement("p");
    assetHint.textContent = this.backendClient
      ? "Backend-connected sessions expose a read-only asset tree with bitmap preview."
      : "Asset preview becomes available when the Rust backend is connected.";
    assetHint.style.margin = "0 0 8px";
    assetHint.style.color = "rgba(246, 236, 217, 0.7)";
    assetHint.style.fontSize = "0.85rem";
    assetHint.style.fontFamily = '"Aptos", "Trebuchet MS", sans-serif';

    this.assetList = document.createElement("div");
    this.assetList.style.display = "grid";
    this.assetList.style.gap = "4px";
    this.assetList.style.maxHeight = "220px";
    this.assetList.style.overflow = "auto";
    this.assetList.style.marginBottom = "10px";
    this.assetList.style.padding = "8px";
    this.assetList.style.border = "1px solid rgba(255, 255, 255, 0.08)";
    this.assetList.style.borderRadius = "14px";
    this.assetList.style.background = "rgba(255, 255, 255, 0.04)";

    this.assetPreview = document.createElement("div");
    this.assetPreview.style.display = "grid";
    this.assetPreview.style.gap = "8px";
    this.assetPreview.style.marginBottom = "8px";
    this.assetPreview.style.padding = "12px";
    this.assetPreview.style.border = "1px solid rgba(255, 255, 255, 0.08)";
    this.assetPreview.style.borderRadius = "14px";
    this.assetPreview.style.background = "rgba(255, 255, 255, 0.04)";

    this.assetStatus = document.createElement("p");
    this.assetStatus.style.margin = "0 0 14px";
    this.assetStatus.style.minHeight = "1.2em";
    this.assetStatus.style.fontSize = "0.8rem";
    this.assetStatus.style.fontFamily = '"Aptos", "Trebuchet MS", sans-serif';
    this.assetStatus.style.color = "rgba(246, 236, 217, 0.7)";

    this.status = document.createElement("p");
    this.status.style.margin = "0 0 14px";
    this.status.style.minHeight = "1.2em";
    this.status.style.fontSize = "0.8rem";
    this.status.style.fontFamily = '"Aptos", "Trebuchet MS", sans-serif';
    this.status.style.color = "rgba(246, 236, 217, 0.7)";

    this.content = document.createElement("pre");
    this.content.style.margin = "0";
    this.content.style.whiteSpace = "pre-wrap";
    this.content.style.wordBreak = "break-word";
    this.content.style.fontSize = "0.82rem";
    this.content.style.lineHeight = "1.5";

    this.panel.append(
      title,
      hint,
      commandRow,
      buttonRow,
      sceneTitle,
      sceneHint,
      this.sceneSource,
      this.sceneFileInput,
      assetTitle,
      assetHint,
      this.assetList,
      this.assetPreview,
      this.assetStatus,
      this.status,
      this.content,
    );
    document.body.appendChild(this.panel);

    if (this.editorSession?.setPendingSourceStateProvider) {
      this.editorSession.setPendingSourceStateProvider(() => this.hasPendingSourceDraft());
    }

    window.addEventListener("morphic-select", this.handleSelect);
    window.addEventListener("morphic-render", this.handleRender);
    window.addEventListener("morphic-editor-state", this.handleEditorState);
    this.refreshSceneSource(true);
    this.renderAssetBrowserPlaceholder();
    this.render(renderer.selected || renderer.tree.root);
    this.syncCommandButtons(this.editorSession?.getState?.());
    if (this.backendClient?.sourcePath) {
      this.setStatus(`Connected to ${this.backendClient.sourcePath}.`);
    }
    if (this.backendClient) {
      this.loadAssetBrowser();
    }
  }

  handleSelect = (event) => {
    if (event.detail.renderer === this.renderer) {
      this.render(event.detail.object);
    }
  };

  handleRender = (event) => {
    if (event.detail.renderer === this.renderer) {
      this.refreshSceneSource(false);
    }
  };

  handleEditorState = (event) => {
    if (event.detail.session === this.editorSession) {
      this.syncCommandButtons(event.detail);
    }
  };

  refreshSceneSource(force) {
    if (this.sceneDirty && !force) {
      return;
    }

    this.sceneSource.value = this.sceneBridge.exportText();
    this.sceneDirty = false;
    this.syncCommandButtons();
  }

  setStatus(message, isError = false) {
    this.status.textContent = message;
    this.status.style.color = isError ? "#ffb4b4" : "rgba(246, 236, 217, 0.7)";
  }

  setAssetStatus(message, isError = false) {
    this.assetStatus.textContent = message;
    this.assetStatus.style.color = isError ? "#ffb4b4" : "rgba(246, 236, 217, 0.7)";
  }

  render(object) {
    this.content.textContent = JSON.stringify(object.inspect(), null, 2);
  }

  renderAssetBrowserPlaceholder(message = "Select a backend asset to preview it here.") {
    this.assetPreview.replaceChildren();
    const text = document.createElement("p");
    text.textContent = message;
    text.style.margin = "0";
    text.style.color = "rgba(246, 236, 217, 0.72)";
    text.style.fontSize = "0.82rem";
    this.assetPreview.appendChild(text);
  }

  async loadAssetBrowser() {
    try {
      this.setAssetStatus("Loading project assets...");
      this.assetBrowserModel = await this.backendClient.refreshBrowserModel();
      this.renderAssetList();
      const initialSelection = defaultAssetSelection(this.assetBrowserModel);
      if (initialSelection) {
        await this.selectAsset(initialSelection);
      } else {
        this.renderAssetBrowserPlaceholder("No asset files were found under the inferred asset root.");
        this.setAssetStatus("No asset files were found.");
      }
    } catch (error) {
      this.renderAssetBrowserPlaceholder("Asset preview is unavailable.");
      this.setAssetStatus(error.message, true);
    }
  }

  renderAssetList() {
    this.assetList.replaceChildren();
    const entries = Array.isArray(this.assetBrowserModel?.entries)
      ? this.assetBrowserModel.entries.filter((entry) => entry?.root === "assets")
      : [];

    if (!entries.length) {
      const empty = document.createElement("p");
      empty.textContent = "No asset tree is available.";
      empty.style.margin = "0";
      empty.style.color = "rgba(246, 236, 217, 0.72)";
      empty.style.fontSize = "0.82rem";
      this.assetList.appendChild(empty);
      return;
    }

    for (const entry of entries) {
      const row = document.createElement(entry.kind === "file" ? "button" : "div");
      row.textContent =
        entry.depth === 0 ? `Assets · ${entry.name}` : `${" ".repeat(entry.depth)}${entry.name}`;
      row.style.margin = "0";
      row.style.padding = "8px 10px";
      row.style.paddingLeft = `${10 + entry.depth * 14}px`;
      row.style.borderRadius = "10px";
      row.style.border = "0";
      row.style.textAlign = "left";
      row.style.font = "inherit";
      row.style.fontSize = "0.8rem";
      row.style.background =
        entry.projectPath === this.selectedAssetPath
          ? "rgba(116, 208, 255, 0.16)"
          : entry.kind === "file"
            ? "rgba(255, 255, 255, 0.03)"
            : "rgba(243, 179, 95, 0.1)";
      row.style.color = "#f6ecd9";
      if (entry.kind === "file") {
        row.type = "button";
        row.style.cursor = "pointer";
        row.onclick = () => {
          this.selectAsset(entry.projectPath).catch((error) => {
            this.setAssetStatus(error.message, true);
          });
        };
      }
      this.assetList.appendChild(row);
    }
  }

  async selectAsset(projectPath) {
    this.selectedAssetPath = projectPath;
    this.renderAssetList();
    this.setAssetStatus(`Selected ${projectPath}.`);
    const token = ++this.assetPreviewToken;
    this.renderAssetBrowserPlaceholder(`Loading ${projectPath}...`);

    if (isBitmapPreviewablePath(projectPath)) {
      this.renderBitmapPreview(projectPath);
      return;
    }

    if (isTextPreviewablePath(projectPath)) {
      const sourceText = await this.backendClient.getProjectFileText(projectPath);
      if (token !== this.assetPreviewToken) {
        return;
      }
      this.renderTextPreview(projectPath, sourceText);
      return;
    }

    this.renderAssetBrowserPlaceholder(`No inline preview is available for ${projectPath}.`);
  }

  renderBitmapPreview(projectPath, heading = "Bitmap Preview") {
    this.assetPreview.replaceChildren();

    const title = document.createElement("p");
    title.textContent = heading;
    title.style.margin = "0";
    title.style.fontFamily = '"Aptos", "Trebuchet MS", sans-serif';
    title.style.fontSize = "0.84rem";
    title.style.color = "rgba(246, 236, 217, 0.72)";

    const caption = document.createElement("code");
    caption.textContent = projectPath;
    caption.style.fontSize = "0.76rem";
    caption.style.color = "rgba(246, 236, 217, 0.78)";

    const image = document.createElement("img");
    image.alt = projectPath;
    image.src = this.backendClient.projectPreviewUrl(projectPath);
    image.style.width = "100%";
    image.style.maxHeight = "220px";
    image.style.objectFit = "contain";
    image.style.borderRadius = "10px";
    image.style.background = "rgba(8, 10, 14, 0.9)";
    image.style.border = "1px solid rgba(255, 255, 255, 0.08)";

    this.assetPreview.append(title, caption, image);
  }

  renderTextPreview(projectPath, sourceText) {
    this.assetPreview.replaceChildren();

    const title = document.createElement("p");
    title.textContent = "Text Preview";
    title.style.margin = "0";
    title.style.fontFamily = '"Aptos", "Trebuchet MS", sans-serif';
    title.style.fontSize = "0.84rem";
    title.style.color = "rgba(246, 236, 217, 0.72)";

    const caption = document.createElement("code");
    caption.textContent = projectPath;
    caption.style.fontSize = "0.76rem";
    caption.style.color = "rgba(246, 236, 217, 0.78)";

    const preview = document.createElement("pre");
    preview.textContent = sourceText.slice(0, 4000);
    preview.style.margin = "0";
    preview.style.maxHeight = "220px";
    preview.style.overflow = "auto";
    preview.style.whiteSpace = "pre-wrap";
    preview.style.wordBreak = "break-word";
    preview.style.fontSize = "0.76rem";
    preview.style.lineHeight = "1.45";
    preview.style.padding = "10px";
    preview.style.borderRadius = "10px";
    preview.style.background = "rgba(8, 10, 14, 0.9)";

    this.assetPreview.append(title, caption, preview);

    if (!projectPath.endsWith(".hmat")) {
      return;
    }

    const logicalName = parseMaterialAlbedoLogicalName(sourceText);
    const bitmapPath = resolveLogicalTextureProjectPath(this.assetBrowserModel?.entries, logicalName);
    if (!bitmapPath) {
      return;
    }

    if (isBitmapPreviewablePath(bitmapPath)) {
      this.renderSecondaryBitmap(bitmapPath);
    } else {
      const note = document.createElement("p");
      note.textContent = `Material albedo resolves to ${bitmapPath}, but that format is not previewable inline.`;
      note.style.margin = "0";
      note.style.fontSize = "0.78rem";
      note.style.color = "rgba(246, 236, 217, 0.72)";
      this.assetPreview.appendChild(note);
    }
  }

  renderSecondaryBitmap(projectPath) {
    const title = document.createElement("p");
    title.textContent = "Resolved Albedo";
    title.style.margin = "0";
    title.style.fontFamily = '"Aptos", "Trebuchet MS", sans-serif';
    title.style.fontSize = "0.84rem";
    title.style.color = "rgba(246, 236, 217, 0.72)";

    const caption = document.createElement("code");
    caption.textContent = projectPath;
    caption.style.fontSize = "0.76rem";
    caption.style.color = "rgba(246, 236, 217, 0.78)";

    const image = document.createElement("img");
    image.alt = projectPath;
    image.src = this.backendClient.projectPreviewUrl(projectPath);
    image.style.width = "100%";
    image.style.maxHeight = "180px";
    image.style.objectFit = "contain";
    image.style.borderRadius = "10px";
    image.style.background = "rgba(8, 10, 14, 0.9)";
    image.style.border = "1px solid rgba(255, 255, 255, 0.08)";

    this.assetPreview.append(title, caption, image);
  }

  hasPendingSourceDraft() {
    return this.sceneDirty;
  }

  syncCommandButtons(state = this.editorSession?.getState?.()) {
    const hasSession = Boolean(this.editorSession);
    const hasPendingDraft = this.hasPendingSourceDraft();
    setButtonDisabled(this.saveButton, !hasSession || hasPendingDraft);
    setButtonDisabled(this.undoButton, !hasSession || hasPendingDraft || !state?.canUndo);
    setButtonDisabled(this.redoButton, !hasSession || hasPendingDraft || !state?.canRedo);
  }

  destroy() {
    window.removeEventListener("morphic-select", this.handleSelect);
    window.removeEventListener("morphic-render", this.handleRender);
    window.removeEventListener("morphic-editor-state", this.handleEditorState);
    if (this.panel.parentNode) {
      this.panel.parentNode.removeChild(this.panel);
    }
  }
}
