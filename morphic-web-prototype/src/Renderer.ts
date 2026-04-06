import { MorphicTree } from './MorphicTree';
import { MorphicObject } from './MorphicObject';

export class Renderer {
  container: HTMLElement;
  tree: MorphicTree;

  constructor(container: HTMLElement, tree: MorphicTree) {
    this.container = container;
    this.tree = tree;
  }

  render() {
    // Clear container
    this.container.innerHTML = '';
    // Render root
    const rootNode = this.renderNode(this.tree.root);
    this.container.appendChild(rootNode);
  }

  renderNode(obj: MorphicObject): HTMLElement {
    const el = obj.render();
    // Add drag, resize, right-click listeners
    el.oncontextmenu = (e) => {
      e.preventDefault();
      window.dispatchEvent(new CustomEvent('morphic-context', { detail: { obj, x: e.clientX, y: e.clientY } }));
    };
    // (Drag/resize handlers can be added here)
    for (const child of obj.children) {
      el.appendChild(this.renderNode(child));
    }
    return el;
  }
}
