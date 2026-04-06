import { MorphicRegistry } from './MorphicRegistry';

export class ScriptingConsole {
  panel: HTMLElement;
  input: HTMLTextAreaElement;
  output: HTMLElement;
  constructor() {
    this.panel = document.createElement('div');
    this.panel.style.position = 'fixed';
    this.panel.style.left = '16px';
    this.panel.style.bottom = '16px';
    this.panel.style.background = '#222';
    this.panel.style.color = '#fff';
    this.panel.style.border = '1px solid #888';
    this.panel.style.padding = '12px';
    this.panel.style.zIndex = '10001';
    this.panel.style.width = '400px';
    this.panel.style.fontFamily = 'monospace';
    this.panel.innerHTML = `<b>Scripting Console</b><br>`;
    this.input = document.createElement('textarea');
    this.input.rows = 3;
    this.input.style.width = '100%';
    this.input.style.background = '#111';
    this.input.style.color = '#fff';
    this.input.style.fontFamily = 'monospace';
    this.panel.appendChild(this.input);
    const runBtn = document.createElement('button');
    runBtn.textContent = 'Run';
    runBtn.onclick = () => this.run();
    this.panel.appendChild(runBtn);
    this.output = document.createElement('div');
    this.output.style.marginTop = '8px';
    this.panel.appendChild(this.output);
    document.body.appendChild(this.panel);
  }
  run() {
    try {
      // Expose registry as 'registry' in eval scope
      const registry = MorphicRegistry.instance;
      // eslint-disable-next-line no-eval
      const result = eval(this.input.value);
      this.output.textContent = String(result);
    } catch (e) {
      this.output.textContent = 'Error: ' + e;
    }
  }
  destroy() {
    if (this.panel.parentNode) this.panel.parentNode.removeChild(this.panel);
  }
}
