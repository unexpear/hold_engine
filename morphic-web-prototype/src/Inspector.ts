import { MorphicObject } from './MorphicObject';

export class Inspector {
  panel: HTMLElement;
  constructor(obj: MorphicObject) {
    this.panel = document.createElement('div');
    this.panel.style.position = 'fixed';
    this.panel.style.right = '16px';
    this.panel.style.top = '16px';
    this.panel.style.background = '#f9f9f9';
    this.panel.style.border = '1px solid #888';
    this.panel.style.padding = '12px';
    this.panel.style.zIndex = '10001';
    this.panel.style.maxWidth = '320px';
    this.panel.style.fontFamily = 'monospace';
    this.panel.innerHTML = `<b>Inspector</b><br><pre>${JSON.stringify(obj.inspect(), null, 2)}</pre>`;
    const closeBtn = document.createElement('button');
    closeBtn.textContent = 'Close';
    closeBtn.onclick = () => this.destroy();
    this.panel.appendChild(closeBtn);
    document.body.appendChild(this.panel);
  }
  destroy() {
    if (this.panel.parentNode) this.panel.parentNode.removeChild(this.panel);
  }
}
