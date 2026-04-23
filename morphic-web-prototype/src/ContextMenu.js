import { MorphicObject } from "./MorphicObject.js";
import { MorphicRegistry } from "./MorphicRegistry.js";

const palette = [
  ["#ee7752", "#e73c7e"],
  ["#f6b73c", "#ff7f50"],
  ["#32c5ff", "#2d6cdf"],
  ["#6be585", "#2bb673"],
];

let paletteIndex = 0;

function createChildMorph(parent) {
  const [start, end] = palette[paletteIndex % palette.length];
  paletteIndex += 1;

  return new MorphicObject({
    name: `Morph ${parent.children.length + 1}`,
    position: {
      x: 28 + parent.children.length * 20,
      y: 46 + parent.children.length * 18,
    },
    size: {
      width: Math.max(140, parent.size.width * 0.48),
      height: Math.max(90, parent.size.height * 0.36),
    },
    style: {
      background: `linear-gradient(160deg, ${start}, ${end})`,
    },
    metadata: {
      createdFrom: parent.id,
      variant: "context-menu-child",
    },
    text: "Fresh morph created from the context menu.",
  });
}

export class ContextMenu {
  constructor(renderer, { applyAuthoredChange } = {}) {
    this.renderer = renderer;
    this.applyAuthoredChange = applyAuthoredChange ?? ((mutator) => mutator());
    this.menu = null;

    window.addEventListener("morphic-context", this.handleContext);
  }

  handleContext = (event) => {
    const { renderer, object, x, y } = event.detail;
    if (renderer !== this.renderer) {
      return;
    }

    this.show(object, x, y);
  };

  buildItems(object) {
    return [
      {
        label: "Inspect",
        action: () => {
          this.renderer.select(object);
        },
      },
      {
        label: "Add child",
        action: () => {
          this.applyAuthoredChange(() => {
            const child = MorphicRegistry.instance.register(createChildMorph(object));
            object.addChild(child);
            this.renderer.select(child);
          });
        },
      },
      {
        label: "Duplicate",
        action: () => {
          this.applyAuthoredChange(() => {
            const clone = MorphicObject.fromJSON(object.toJSON(), { preserveIds: false });
            clone.name = `${object.name} Copy`;
            clone.move({
              x: object.position.x + 26,
              y: object.position.y + 26,
            });
            const host = object.parent || object;
            host.addChild(clone);
            MorphicRegistry.instance.registerTree(clone);
            this.renderer.select(clone);
          });
        },
      },
      ...(object.parent
        ? [
            {
              label: "Delete",
              action: () => {
                this.applyAuthoredChange(() => {
                  const parent = object.parent;
                  MorphicRegistry.instance.unregisterTree(object);
                  parent.removeChild(object);
                  this.renderer.select(parent);
                });
              },
            },
          ]
        : []),
    ];
  }

  show(object, x, y) {
    this.destroyMenu();

    const menu = document.createElement("div");
    menu.style.position = "fixed";
    menu.style.left = `${x}px`;
    menu.style.top = `${y}px`;
    menu.style.minWidth = "170px";
    menu.style.padding = "8px";
    menu.style.borderRadius = "16px";
    menu.style.border = "1px solid rgba(255, 255, 255, 0.12)";
    menu.style.background = "rgba(15, 18, 24, 0.95)";
    menu.style.boxShadow = "0 20px 40px rgba(0, 0, 0, 0.35)";
    menu.style.backdropFilter = "blur(16px)";
    menu.style.zIndex = "10000";

    for (const item of this.buildItems(object)) {
      const button = document.createElement("button");
      button.type = "button";
      button.textContent = item.label;
      button.style.display = "block";
      button.style.width = "100%";
      button.style.margin = "0";
      button.style.padding = "10px 12px";
      button.style.border = "0";
      button.style.borderRadius = "10px";
      button.style.background = "transparent";
      button.style.color = "#f6ecd9";
      button.style.font = "inherit";
      button.style.textAlign = "left";
      button.style.cursor = "pointer";
      button.onmouseenter = () => {
        button.style.background = "rgba(243, 179, 95, 0.18)";
      };
      button.onmouseleave = () => {
        button.style.background = "transparent";
      };
      button.onclick = () => {
        item.action();
        this.destroyMenu();
      };
      menu.appendChild(button);
    }

    this.menu = menu;
    document.body.appendChild(menu);
    document.addEventListener("mousedown", this.handleOutside, true);
  }

  handleOutside = (event) => {
    if (this.menu && !this.menu.contains(event.target)) {
      this.destroyMenu();
    }
  };

  destroyMenu() {
    document.removeEventListener("mousedown", this.handleOutside, true);
    if (this.menu && this.menu.parentNode) {
      this.menu.parentNode.removeChild(this.menu);
    }
    this.menu = null;
  }

  destroy() {
    window.removeEventListener("morphic-context", this.handleContext);
    this.destroyMenu();
  }
}
