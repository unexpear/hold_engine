let idCounter = 1;

function cloneMetadata(value) {
  if (value == null || typeof value !== "object") {
    return value;
  }

  if (typeof globalThis.structuredClone === "function") {
    return globalThis.structuredClone(value);
  }

  if (Array.isArray(value)) {
    return value.map((entry) => cloneMetadata(entry));
  }

  const clone = {};
  for (const [key, entry] of Object.entries(value)) {
    clone[key] = cloneMetadata(entry);
  }
  return clone;
}

export class MorphicObject {
  constructor({
    name,
    position = {},
    size = {},
    style = {},
    metadata = {},
    text = "",
  } = {}) {
    const id = idCounter++;

    this.id = id;
    this.name = name ?? `Morph ${id}`;
    this.parent = null;
    this.children = [];
    this.position = { x: 0, y: 0, ...position };
    this.size = { width: 160, height: 120, ...size };
    this.style = {
      background: "linear-gradient(160deg, #5d7cf6, #2946af)",
      color: "#fff8ef",
      border: "1px solid rgba(255, 255, 255, 0.18)",
      borderRadius: "18px",
      boxShadow: "0 18px 40px rgba(0, 0, 0, 0.28)",
      padding: "14px",
      overflow: "hidden",
      ...style,
    };
    this.metadata = cloneMetadata(metadata) ?? {};
    this.text = text;
  }

  addChild(child) {
    if (!(child instanceof MorphicObject)) {
      throw new TypeError("Child must be a MorphicObject.");
    }

    if (child.parent) {
      child.parent.removeChild(child);
    }

    child.parent = this;
    this.children.push(child);
    return child;
  }

  removeChild(child) {
    this.children = this.children.filter((candidate) => candidate !== child);
    if (child.parent === this) {
      child.parent = null;
    }
    return child;
  }

  move(position) {
    this.position = { ...this.position, ...position };
    if (this.metadata?.scene?.transform?.position) {
      this.metadata.scene.transform.position = {
        ...this.metadata.scene.transform.position,
        x: this.position.x,
        y: this.position.y,
      };
    }
    return this;
  }

  resize(size) {
    this.size = { ...this.size, ...size };
    return this;
  }

  inspect() {
    return {
      id: this.id,
      name: this.name,
      parent: this.parent ? this.parent.id : null,
      children: this.children.map((child) => child.id),
      position: { ...this.position },
      size: { ...this.size },
      style: { ...this.style },
      metadata: cloneMetadata(this.metadata) ?? {},
      text: this.text,
    };
  }

  toJSON() {
    return {
      id: this.id,
      name: this.name,
      position: { ...this.position },
      size: { ...this.size },
      style: { ...this.style },
      metadata: cloneMetadata(this.metadata) ?? {},
      text: this.text,
      children: this.children.map((child) => child.toJSON()),
    };
  }

  static fromJSON(json, { preserveIds = true } = {}) {
    const object = new MorphicObject({
      name: json.name,
      position: json.position,
      size: json.size,
      style: json.style,
      metadata: json.metadata,
      text: json.text,
    });

    if (preserveIds && typeof json.id === "number") {
      object.id = json.id;
      idCounter = Math.max(idCounter, object.id + 1);
    }

    if (Array.isArray(json.children)) {
      for (const childJson of json.children) {
        object.addChild(MorphicObject.fromJSON(childJson, { preserveIds }));
      }
    }

    return object;
  }
}
