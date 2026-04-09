import { MorphicObject } from "./MorphicObject.js";

export class MorphicTree {
  constructor(root = new MorphicObject()) {
    this.root = root;
  }

  traverse(visitor, node = this.root) {
    visitor(node);
    for (const child of node.children) {
      this.traverse(visitor, child);
    }
  }

  findById(id, node = this.root) {
    if (node.id === id) {
      return node;
    }

    for (const child of node.children) {
      const found = this.findById(id, child);
      if (found) {
        return found;
      }
    }

    return null;
  }

  remove(object) {
    if (object.parent) {
      object.parent.removeChild(object);
    }
  }

  toJSON() {
    return this.root.toJSON();
  }

  static fromJSON(json, options) {
    return new MorphicTree(MorphicObject.fromJSON(json, options));
  }
}
