import { MorphicObject } from './MorphicObject';

export class MorphicRegistry {
  private static _instance: MorphicRegistry;
  private objects: Map<number, MorphicObject> = new Map();

  private constructor() {}

  static get instance(): MorphicRegistry {
    if (!MorphicRegistry._instance) {
      MorphicRegistry._instance = new MorphicRegistry();
    }
    return MorphicRegistry._instance;
  }

  register(obj: MorphicObject) {
    this.objects.set(obj.id, obj);
  }

  unregister(obj: MorphicObject) {
    this.objects.delete(obj.id);
  }

  getById(id: number): MorphicObject | undefined {
    return this.objects.get(id);
  }

  getAll(): MorphicObject[] {
    return Array.from(this.objects.values());
  }

  find(predicate: (obj: MorphicObject) => boolean): MorphicObject[] {
    return this.getAll().filter(predicate);
  }
}

// Patch MorphicObject to auto-register/unregister
import type { Constructor } from './types';
export function withRegistry<TBase extends Constructor<MorphicObject>>(Base: TBase) {
  return class extends Base {
    constructor(...args: any[]) {
      super(...args);
      MorphicRegistry.instance.register(this);
    }
    destroy() {
      MorphicRegistry.instance.unregister(this);
      if (super['destroy']) super['destroy']();
    }
  };
}
