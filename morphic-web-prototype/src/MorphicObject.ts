export type Position = { x: number; y: number };
export type Size = { width: number; height: number };
export type Style = { [key: string]: string };
export type Metadata = { [key: string]: any };

let idCounter = 0;

export class MorphicObject {
  id: number;
  parent: MorphicObject | null;
  children: MorphicObject[];
  position: Position;
  size: Size;
  style: Style;
  metadata: Metadata;

  constructor(position: Position = { x: 0, y: 0 }, size: Size = { width: 100, height: 100 }, style: Style = {}, metadata: Metadata = {}) {
    this.id = idCounter++;
    this.parent = null;
    this.children = [];
    this.position = position;
    this.size = size;
    this.style = style;
    this.metadata = metadata;
  }

  addChild(child: MorphicObject) {
    child.parent = this;
    this.children.push(child);
  }

  removeChild(child: MorphicObject) {
    this.children = this.children.filter(c => c !== child);
    if (child.parent === this) child.parent = null;
  }

  move(newPosition: Position) {
    this.position = newPosition;
  }

  resize(newSize: Size) {
    this.size = newSize;
  }

  render(): HTMLElement {
    const el = document.createElement('div');
    el.style.position = 'absolute';
    el.style.left = this.position.x + 'px';
    el.style.top = this.position.y + 'px';
    el.style.width = this.size.width + 'px';
    el.style.height = this.size.height + 'px';
    for (const key in this.style) {
      el.style[key as any] = this.style[key];
    }
    el.setAttribute('data-morphic-id', this.id.toString());
    for (const child of this.children) {
      el.appendChild(child.render());
    }
    return el;
  }

  inspect(): object {
    return {
      id: this.id,
      parent: this.parent ? this.parent.id : null,
      children: this.children.map(c => c.id),
      position: this.position,
      size: this.size,
      style: this.style,
      metadata: this.metadata,
    };
  }

  toJSON(): object {
    return {
      id: this.id,
      position: this.position,
      size: this.size,
      style: this.style,
      metadata: this.metadata,
      children: this.children.map(child => child.toJSON()),
    };
  }

  static fromJSON(json: any): MorphicObject {
    const obj = new MorphicObject(json.position, json.size, json.style, json.metadata);
    if (Array.isArray(json.children)) {
      for (const childJson of json.children) {
        const child = MorphicObject.fromJSON(childJson);
        obj.addChild(child);
      }
    }
    return obj;
  }
}
