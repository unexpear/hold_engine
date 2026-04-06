export class ContextMenu {
  menu: HTMLElement;
  constructor(items: { label: string, action: () => void }[], x: number, y: number) {
    this.menu = document.createElement('div');
    this.menu.style.position = 'fixed';
    this.menu.style.left = x + 'px';
    this.menu.style.top = y + 'px';
    this.menu.style.background = '#fff';
    this.menu.style.border = '1px solid #888';
    this.menu.style.zIndex = '10000';
    this.menu.style.padding = '4px 0';
    this.menu.style.boxShadow = '0 2px 8px rgba(0,0,0,0.15)';
    for (const item of items) {
      const btn = document.createElement('div');
      btn.textContent = item.label;
      btn.style.padding = '4px 16px';
      btn.style.cursor = 'pointer';
      btn.onmouseenter = () => btn.style.background = '#eee';
      btn.onmouseleave = () => btn.style.background = '';
      btn.onclick = () => {
        item.action();
        this.destroy();
      };
      this.menu.appendChild(btn);
    }
    document.body.appendChild(this.menu);
    document.addEventListener('mousedown', this.handleOutside, true);
  }
  handleOutside = (e: MouseEvent) => {
    if (!this.menu.contains(e.target as Node)) {
      this.destroy();
    }
  };
  destroy() {
    document.removeEventListener('mousedown', this.handleOutside, true);
    if (this.menu.parentNode) this.menu.parentNode.removeChild(this.menu);
  }
}
