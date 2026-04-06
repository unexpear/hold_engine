import { MorphicObject } from './MorphicObject';

export class MorphicTree {
  root: MorphicObject;

  constructor(root?: MorphicObject) {
    this.root = root || new MorphicObject();
  }

  traverse(fn: (obj: MorphicObject) => void, node: MorphicObject = this.root) {
    fn(node);
    for (const child of node.children) {
      this.traverse(fn, child);
    }
  }

  findById(id: number, node: MorphicObject = this.root): MorphicObject | null {
    if (node.id === id) return node;
    for (const child of node.children) {
      const found = this.findById(id, child);
      if (found) return found;
    }
    return null;
  }

  remove(obj: MorphicObject) {
    if (obj.parent) {
      obj.parent.removeChild(obj);
    }
  }

  toJSON(): object {
    return this.root.toJSON();
  }

  static fromJSON(json: any): MorphicTree {
    return new MorphicTree(MorphicObject.fromJSON(json));
  }
}
