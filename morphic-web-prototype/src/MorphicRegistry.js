export class MorphicRegistry {
  static _instance;

  constructor() {
    this.objects = new Map();
  }

  static get instance() {
    if (!MorphicRegistry._instance) {
      MorphicRegistry._instance = new MorphicRegistry();
    }

    return MorphicRegistry._instance;
  }

  reset() {
    this.objects.clear();
  }

  register(object) {
    this.objects.set(object.id, object);
    return object;
  }

  registerTree(object) {
    this.register(object);
    for (const child of object.children) {
      this.registerTree(child);
    }
    return object;
  }

  unregister(object) {
    this.objects.delete(object.id);
    return object;
  }

  unregisterTree(object) {
    this.unregister(object);
    for (const child of object.children) {
      this.unregisterTree(child);
    }
    return object;
  }

  getById(id) {
    return this.objects.get(id);
  }

  getAll() {
    return Array.from(this.objects.values());
  }

  find(predicate) {
    return this.getAll().filter(predicate);
  }
}
