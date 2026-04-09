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

export class Inspector {
  constructor(renderer, sceneBridge) {
    this.renderer = renderer;
    this.sceneBridge = sceneBridge;
    this.sceneDirty = false;
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
      this.setStatus("Copied .hsc scene text.");
    };

    const refreshSceneButton = makeButton("Refresh .hsc", "rgba(255, 255, 255, 0.08)");
    refreshSceneButton.onclick = () => {
      this.refreshSceneSource(true);
      this.setStatus("Scene text refreshed from the current tree.");
    };

    const loadSceneButton = makeButton("Load .hsc", "rgba(87, 213, 140, 0.18)");
    loadSceneButton.onclick = () => {
      try {
        this.sceneBridge.importText(this.sceneSource.value);
        this.sceneDirty = false;
        this.refreshSceneSource(true);
        this.setStatus("Imported .hsc scene into the browser prototype.");
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
    });

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
      buttonRow,
      sceneTitle,
      sceneHint,
      this.sceneSource,
      this.status,
      this.content,
    );
    document.body.appendChild(this.panel);

    window.addEventListener("morphic-select", this.handleSelect);
    window.addEventListener("morphic-render", this.handleRender);
    this.refreshSceneSource(true);
    this.render(renderer.selected || renderer.tree.root);
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

  refreshSceneSource(force) {
    if (this.sceneDirty && !force) {
      return;
    }

    this.sceneSource.value = this.sceneBridge.exportText();
    this.sceneDirty = false;
  }

  setStatus(message, isError = false) {
    this.status.textContent = message;
    this.status.style.color = isError ? "#ffb4b4" : "rgba(246, 236, 217, 0.7)";
  }

  render(object) {
    this.content.textContent = JSON.stringify(object.inspect(), null, 2);
  }

  destroy() {
    window.removeEventListener("morphic-select", this.handleSelect);
    window.removeEventListener("morphic-render", this.handleRender);
    if (this.panel.parentNode) {
      this.panel.parentNode.removeChild(this.panel);
    }
  }
}
