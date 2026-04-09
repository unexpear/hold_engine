import { MorphicObject } from "./MorphicObject.js";
import { MorphicRegistry } from "./MorphicRegistry.js";

function formatResult(result) {
  if (typeof result === "string") {
    return result;
  }

  if (typeof result === "undefined") {
    return "Executed.";
  }

  try {
    return JSON.stringify(result, null, 2);
  } catch (error) {
    return String(result);
  }
}

export class ScriptingConsole {
  constructor({ renderer, sceneBridge }) {
    this.renderer = renderer;
    this.sceneBridge = sceneBridge;
    this.panel = document.createElement("section");
    this.panel.style.position = "fixed";
    this.panel.style.left = "18px";
    this.panel.style.bottom = "18px";
    this.panel.style.width = "min(460px, calc(100vw - 36px))";
    this.panel.style.padding = "14px";
    this.panel.style.borderRadius = "18px";
    this.panel.style.border = "1px solid rgba(255, 255, 255, 0.12)";
    this.panel.style.background = "rgba(12, 15, 21, 0.88)";
    this.panel.style.color = "#f6ecd9";
    this.panel.style.boxShadow = "0 24px 60px rgba(0, 0, 0, 0.32)";
    this.panel.style.backdropFilter = "blur(16px)";
    this.panel.style.zIndex = "9998";
    this.panel.style.fontFamily = '"Cascadia Code", Consolas, monospace';

    const title = document.createElement("div");
    title.textContent = "Scripting Console";
    title.style.fontFamily = '"Aptos", "Trebuchet MS", sans-serif';
    title.style.fontWeight = "700";
    title.style.letterSpacing = "0.08em";
    title.style.textTransform = "uppercase";

    const note = document.createElement("p");
    note.textContent = "Trusted local JS only. Available helpers: registry, tree, renderer, MorphicObject, scene.";
    note.style.margin = "8px 0 10px";
    note.style.color = "rgba(246, 236, 217, 0.72)";
    note.style.fontSize = "0.88rem";
    note.style.fontFamily = '"Aptos", "Trebuchet MS", sans-serif';

    this.input = document.createElement("textarea");
    this.input.rows = 4;
    this.input.spellcheck = false;
    this.input.placeholder = "registry.getAll().map((obj) => obj.name)";
    this.input.style.width = "100%";
    this.input.style.resize = "vertical";
    this.input.style.padding = "12px";
    this.input.style.border = "1px solid rgba(255, 255, 255, 0.08)";
    this.input.style.borderRadius = "14px";
    this.input.style.background = "rgba(255, 255, 255, 0.04)";
    this.input.style.color = "#f6ecd9";
    this.input.style.font = "inherit";

    const buttonRow = document.createElement("div");
    buttonRow.style.display = "flex";
    buttonRow.style.gap = "10px";
    buttonRow.style.marginTop = "10px";

    const runButton = document.createElement("button");
    runButton.type = "button";
    runButton.textContent = "Run";
    runButton.style.padding = "10px 12px";
    runButton.style.border = "0";
    runButton.style.borderRadius = "999px";
    runButton.style.background = "rgba(243, 179, 95, 0.2)";
    runButton.style.color = "#f6ecd9";
    runButton.style.cursor = "pointer";
    runButton.onclick = () => this.run();

    const demoButton = document.createElement("button");
    demoButton.type = "button";
    demoButton.textContent = "Insert Demo";
    demoButton.style.padding = "10px 12px";
    demoButton.style.border = "0";
    demoButton.style.borderRadius = "999px";
    demoButton.style.background = "rgba(255, 255, 255, 0.08)";
    demoButton.style.color = "#f6ecd9";
    demoButton.style.cursor = "pointer";
    demoButton.onclick = () => {
      this.input.value = "const child = registry.getAll().find((obj) => obj.name.includes('Workbench'));\nchild.move({ x: child.position.x + 18, y: child.position.y + 12 });\nrenderer.render();";
    };

    buttonRow.append(runButton, demoButton);

    this.output = document.createElement("pre");
    this.output.style.margin = "12px 0 0";
    this.output.style.padding = "12px";
    this.output.style.minHeight = "56px";
    this.output.style.borderRadius = "14px";
    this.output.style.background = "rgba(255, 255, 255, 0.04)";
    this.output.style.whiteSpace = "pre-wrap";
    this.output.style.wordBreak = "break-word";
    this.output.style.fontSize = "0.82rem";

    this.panel.append(title, note, this.input, buttonRow, this.output);
    document.body.appendChild(this.panel);
  }

  run() {
    const source = this.input.value.trim();
    if (!source) {
      this.output.textContent = "Enter a JavaScript expression or statements to run.";
      return;
    }

    const registry = MorphicRegistry.instance;
    const { renderer, sceneBridge: scene } = this;
    const tree = renderer.tree;

    try {
      let result;

      try {
        result = new Function(
          "registry",
          "tree",
          "renderer",
          "MorphicObject",
          "scene",
          `"use strict"; return (${source});`,
        )(registry, tree, renderer, MorphicObject, scene);
      } catch (expressionError) {
        result = new Function(
          "registry",
          "tree",
          "renderer",
          "MorphicObject",
          "scene",
          `"use strict"; ${source}`,
        )(registry, tree, renderer, MorphicObject, scene);
      }

      this.output.textContent = formatResult(result);
      renderer.render();
    } catch (error) {
      this.output.textContent = `Error: ${error.message}`;
    }
  }

  destroy() {
    if (this.panel.parentNode) {
      this.panel.parentNode.removeChild(this.panel);
    }
  }
}
