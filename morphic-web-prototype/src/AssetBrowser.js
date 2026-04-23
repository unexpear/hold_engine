function extensionOf(projectPath) {
  const value = String(projectPath ?? "");
  const lastDot = value.lastIndexOf(".");
  if (lastDot < 0) {
    return "";
  }
  return value.slice(lastDot + 1).toLowerCase();
}

export function isBitmapPreviewablePath(projectPath) {
  return ["bmp", "png", "jpg", "jpeg", "gif", "webp", "tga"].includes(extensionOf(projectPath));
}

export function isTextPreviewablePath(projectPath) {
  return ["hmat", "obj", "txt", "md", "json", "hsc", "hscene", "rs", "js"].includes(
    extensionOf(projectPath),
  );
}

export function parseMaterialAlbedoLogicalName(sourceText) {
  if (typeof sourceText !== "string") {
    return null;
  }

  for (const rawLine of sourceText.split(/\r?\n/)) {
    const line = rawLine.trim();
    if (!line || line.startsWith("#")) {
      continue;
    }

    const [key, value, extra] = line.split(/\s+/);
    if (extra !== undefined) {
      continue;
    }
    if (key !== "albedo" && key !== "albedo_logical_name") {
      continue;
    }
    if (!value || value === "-" || value.toLowerCase() === "none") {
      return null;
    }
    return value;
  }

  return null;
}

export function resolveLogicalTextureProjectPath(entries, logicalName) {
  const normalized = String(logicalName ?? "").trim().replace(/^\.?\/*/, "");
  if (!normalized) {
    return null;
  }

  const fileEntries = Array.isArray(entries)
    ? entries.filter((entry) => entry?.kind === "file")
    : [];
  const extensions = ["bmp", "png", "jpg", "jpeg", "gif", "webp", "tga"];
  for (const extension of extensions) {
    const suffix = `/${normalized}.${extension}`.toLowerCase();
    const exact = `${normalized}.${extension}`.toLowerCase();
    const match = fileEntries.find((entry) => {
      const projectPath = String(entry.projectPath ?? "").toLowerCase();
      return projectPath === exact || projectPath.endsWith(suffix);
    });
    if (match) {
      return match.projectPath;
    }
  }

  return null;
}

export function defaultAssetSelection(model) {
  const entries = Array.isArray(model?.entries)
    ? model.entries.filter((entry) => entry?.root === "assets" && entry?.kind === "file")
    : [];
  return (
    entries.find((entry) => isBitmapPreviewablePath(entry.projectPath))?.projectPath
    ?? entries.find((entry) => extensionOf(entry.projectPath) === "hmat")?.projectPath
    ?? entries[0]?.projectPath
    ?? ""
  );
}
