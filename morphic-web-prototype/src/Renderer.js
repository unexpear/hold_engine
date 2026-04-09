export class Renderer {
  constructor(container, tree) {
    if (!(container instanceof HTMLElement)) {
      throw new Error("Renderer requires a real container element.");
    }

    this.container = container;
    this.tree = tree;
    this.selected = null;

    this.container.addEventListener("click", (event) => {
      if (event.target === this.container) {
        this.select(this.tree.root);
      }
    });
  }

  render() {
    this.container.replaceChildren(this.renderNode(this.tree.root));
    window.dispatchEvent(
      new CustomEvent("morphic-render", {
        detail: { renderer: this, tree: this.tree },
      }),
    );
  }

  select(object) {
    this.selected = object;
    this.render();
    window.dispatchEvent(
      new CustomEvent("morphic-select", {
        detail: { object, renderer: this },
      }),
    );
  }

  renderNode(object) {
    const element = document.createElement("section");
    element.dataset.morphicId = String(object.id);
    element.style.position = "absolute";
    element.style.left = `${object.position.x}px`;
    element.style.top = `${object.position.y}px`;
    element.style.width = `${object.size.width}px`;
    element.style.height = `${object.size.height}px`;
    element.style.display = "flex";
    element.style.flexDirection = "column";
    element.style.gap = "8px";
    element.style.cursor = "pointer";
    element.style.userSelect = "none";
    element.style.transition = "transform 140ms ease, box-shadow 140ms ease";

    for (const [key, value] of Object.entries(object.style)) {
      element.style[key] = value;
    }

    if (this.selected && this.selected.id === object.id) {
      element.style.outline = "3px solid rgba(243, 179, 95, 0.95)";
      element.style.outlineOffset = "2px";
      element.style.transform = "translateY(-2px)";
    }

    const title = document.createElement("div");
    title.textContent = object.name;
    title.style.fontSize = "1rem";
    title.style.fontWeight = "700";
    title.style.letterSpacing = "0.03em";

    const summary = document.createElement("div");
    summary.textContent = `${object.size.width} x ${object.size.height} | ${object.children.length} child${object.children.length === 1 ? "" : "ren"}`;
    summary.style.fontSize = "0.78rem";
    summary.style.opacity = "0.72";

    const body = document.createElement("div");
    body.textContent = object.text || "Right-click to spawn, duplicate, inspect, or prune morphs.";
    body.style.fontSize = "0.92rem";
    body.style.lineHeight = "1.35";
    body.style.maxWidth = "26ch";

    element.append(title, summary, body);

    element.addEventListener("click", (event) => {
      event.stopPropagation();
      this.select(object);
    });

    element.addEventListener("contextmenu", (event) => {
      event.preventDefault();
      event.stopPropagation();
      this.select(object);
      window.dispatchEvent(
        new CustomEvent("morphic-context", {
          detail: {
            object,
            x: event.clientX,
            y: event.clientY,
            renderer: this,
          },
        }),
      );
    });

    for (const child of object.children) {
      element.appendChild(this.renderNode(child));
    }

    return element;
  }
}
