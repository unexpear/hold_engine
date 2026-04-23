async function readTextResponse(response) {
  const text = await response.text();
  if (!response.ok) {
    throw new Error(text || `Request failed with status ${response.status}.`);
  }
  return text;
}

async function postPlainText(url, body = "") {
  const response = await fetch(url, {
    method: "POST",
    headers: {
      "Content-Type": "text/plain;charset=utf-8",
    },
    body,
  });
  return readTextResponse(response);
}

export class BackendClient {
  constructor(session) {
    this.session = session;
    this.browserModel = null;
  }

  static async connect() {
    try {
      const response = await fetch("./api/session");
      const text = await readTextResponse(response);
      return new BackendClient(JSON.parse(text));
    } catch (_error) {
      return null;
    }
  }

  get sourcePath() {
    return this.session?.sourcePath ?? "";
  }

  async refreshSession() {
    const response = await fetch("./api/session");
    const text = await readTextResponse(response);
    this.session = JSON.parse(text);
    return this.session;
  }

  async refreshBrowserModel() {
    const response = await fetch("./api/browser");
    const text = await readTextResponse(response);
    this.browserModel = JSON.parse(text);
    return this.browserModel;
  }

  async getSceneText() {
    const response = await fetch("./api/scene");
    const text = await readTextResponse(response);
    this.session.sourceText = text;
    return text;
  }

  async applySceneText(sourceText) {
    const text = await postPlainText("./api/scene", sourceText);
    this.session.sourceText = text;
    this.session.dirty = true;
    return text;
  }

  async saveSceneText(sourceText) {
    const text = await postPlainText("./api/save", sourceText);
    this.session.sourceText = text;
    this.session.dirty = false;
    return {
      sourcePath: this.sourcePath,
      sourceText: text,
    };
  }

  async reloadSceneText() {
    const text = await postPlainText("./api/reload");
    this.session.sourceText = text;
    this.session.dirty = false;
    return text;
  }

  projectFileUrl(projectPath) {
    return `./api/project-file?path=${encodeURIComponent(projectPath)}`;
  }

  projectPreviewUrl(projectPath) {
    return `./api/project-preview?path=${encodeURIComponent(projectPath)}`;
  }

  async getProjectFileText(projectPath) {
    const response = await fetch(this.projectFileUrl(projectPath));
    return readTextResponse(response);
  }

  async executeProtocolCommand(message) {
    const text = await postPlainText("./api/command", message);
    return text
      .split(/\r?\n/)
      .map((line) => line.trim())
      .filter(Boolean);
  }
}
