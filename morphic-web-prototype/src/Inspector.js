export class Inspector {
  constructor(renderer) {
    this.renderer = renderer;
    this.panel = document.createElement("aside");
    this.panel.style.position = "fixed";
    this.panel.style.right = "18px";
    this.panel.style.top = "18px";
    this.panel.style.width = "min(340px, calc(100vw - 36px))";
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

    const exportButton = document.createElement("button");
    exportButton.type = "button";
    exportButton.textContent = "Copy Tree JSON";
    exportButton.style.marginBottom = "12px";
    exportButton.style.padding = "10px 12px";
    exportButton.style.border = "0";
    exportButton.style.borderRadius = "999px";
    exportButton.style.background = "rgba(243, 179, 95, 0.18)";
    exportButton.style.color = "#f6ecd9";
    exportButton.style.cursor = "pointer";
    exportButton.onclick = async () => {
      const json = JSON.stringify(this.renderer.tree.toJSON(), null, 2);
      if (navigator.clipboard?.writeText) {
        await navigator.clipboard.writeText(json);
      }
    };

    this.content = document.createElement("pre");
    this.content.style.margin = "0";
    this.content.style.whiteSpace = "pre-wrap";
    this.content.style.wordBreak = "break-word";
    this.content.style.fontSize = "0.82rem";
    this.content.style.lineHeight = "1.5";

    this.panel.append(title, hint, exportButton, this.content);
    document.body.appendChild(this.panel);

    window.addEventListener("morphic-select", this.handleSelect);
    this.render(renderer.selected || renderer.tree.root);
  }

  handleSelect = (event) => {
    if (event.detail.renderer === this.renderer) {
      this.render(event.detail.object);
    }
  };

  render(object) {
    this.content.textContent = JSON.stringify(object.inspect(), null, 2);
  }

  destroy() {
    window.removeEventListener("morphic-select", this.handleSelect);
    if (this.panel.parentNode) {
      this.panel.parentNode.removeChild(this.panel);
    }
  }
}
