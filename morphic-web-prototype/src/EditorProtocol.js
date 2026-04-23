const AXES = new Set(["X", "Y", "Z"]);
const PLAY_STATES = new Set(["Editing", "Playing", "Paused"]);

function escapeToken(input) {
  return String(input)
    .replace(/\\/g, "\\\\")
    .replace(/\|/g, "\\p")
    .replace(/\n/g, "\\n")
    .replace(/\r/g, "\\r");
}

function unescapeToken(input) {
  let out = "";
  for (let i = 0; i < input.length; i += 1) {
    const ch = input[i];
    if (ch !== "\\") {
      out += ch;
      continue;
    }

    const next = input[i + 1];
    if (next === "\\") {
      out += "\\";
      i += 1;
    } else if (next === "p") {
      out += "|";
      i += 1;
    } else if (next === "n") {
      out += "\n";
      i += 1;
    } else if (next === "r") {
      out += "\r";
      i += 1;
    } else if (next == null) {
      out += "\\";
    } else {
      out += `\\${next}`;
      i += 1;
    }
  }
  return out;
}

function splitTokens(message) {
  return String(message).replace(/^\uFEFF/, "").split("|").map(unescapeToken);
}

function requireTokenCount(tokens, expected, kind) {
  if (tokens.length !== expected) {
    throw new Error(`${kind}: expected ${expected} tokens, got ${tokens.length}`);
  }
}

function requireMinTokenCount(tokens, expected, kind) {
  if (tokens.length < expected) {
    throw new Error(`${kind}: expected at least ${expected} tokens, got ${tokens.length}`);
  }
}

function parseAxis(token) {
  if (!AXES.has(token)) {
    throw new Error(`invalid axis '${token}'`);
  }
  return token;
}

function parseOptionalAxis(token) {
  if (!token) {
    return null;
  }
  return parseAxis(token);
}

function parsePlayState(token) {
  if (!PLAY_STATES.has(token)) {
    throw new Error(`invalid play state '${token}'`);
  }
  return token;
}

function parseOptionalToken(token) {
  return token ? token : null;
}

function encodeBool(value) {
  return value ? "true" : "false";
}

function parseBool(token) {
  if (token === "true") {
    return true;
  }
  if (token === "false") {
    return false;
  }
  throw new Error(`invalid bool '${token}'`);
}

function parseVec3(tokens, offset) {
  return [
    Number(tokens[offset]),
    Number(tokens[offset + 1]),
    Number(tokens[offset + 2]),
  ];
}

function encodeValue(value) {
  switch (value.kind) {
    case "Number":
      return ["number", String(value.value)];
    case "Text":
      return ["text", escapeToken(value.value)];
    case "Vec3":
      return [
        "vec3",
        String(value.value[0]),
        String(value.value[1]),
        String(value.value[2]),
      ];
    case "Bool":
      return ["bool", value.value ? "true" : "false"];
    default:
      throw new Error(`unsupported value kind '${value.kind}'`);
  }
}

function decodeValue(tokens) {
  const [kind, ...rest] = tokens;
  switch (kind) {
    case "number":
      requireTokenCount(tokens, 2, "number");
      return { kind: "Number", value: Number(rest[0]) };
    case "text":
      requireTokenCount(tokens, 2, "text");
      return { kind: "Text", value: rest[0] };
    case "vec3":
      requireTokenCount(tokens, 4, "vec3");
      return {
        kind: "Vec3",
        value: [Number(rest[0]), Number(rest[1]), Number(rest[2])],
      };
    case "bool":
      requireTokenCount(tokens, 2, "bool");
      if (rest[0] !== "true" && rest[0] !== "false") {
        throw new Error(`invalid bool '${rest[0]}'`);
      }
      return { kind: "Bool", value: rest[0] === "true" };
    default:
      throw new Error(`unsupported value kind '${kind}'`);
  }
}

export function encodeCommand(command) {
  switch (command.type) {
    case "SelectNextEntity":
    case "SelectPrevEntity":
    case "SelectEntity":
    case "SelectNextComponent":
    case "SelectPrevComponent":
    case "SelectComponent":
    case "SelectNextField":
    case "SelectPrevField":
    case "SelectField":
    case "EnterPlayMode":
    case "PausePlayMode":
    case "ResumePlayMode":
    case "StopPlayMode":
    case "Save":
    case "Undo":
    case "Redo":
    case "ReloadSource":
    case "ForcePatch":
    case "ForceRebuild":
      if (command.type === "SelectEntity") {
        return `cmd|SelectEntity|${command.entityId}`;
      }
      if (command.type === "SelectComponent") {
        return `cmd|SelectComponent|${command.entityId}|${escapeToken(command.componentName)}`;
      }
      if (command.type === "SelectField") {
        return `cmd|SelectField|${command.entityId}|${escapeToken(command.componentName)}|${escapeToken(command.fieldName)}`;
      }
      return `cmd|${command.type}`;
    case "SetAxis":
      return `cmd|SetAxis|${parseAxis(command.axis)}`;
    case "ApplyDelta":
      return `cmd|ApplyDelta|${command.delta}`;
    case "SetTerrainSettings":
      return `cmd|SetTerrainSettings|${command.sizeMeters[0]}|${command.sizeMeters[1]}|${command.sizeMeters[2]}|${command.chunkSizeMeters}|${command.heightScaleMeters}`;
    case "UpsertTerrainPaintLayer":
      return `cmd|UpsertTerrainPaintLayer|${escapeToken(command.name)}|${escapeToken(command.material ?? "")}|${command.tiling}|${command.weightmapChannel}`;
    case "RemoveTerrainPaintLayer":
      return `cmd|RemoveTerrainPaintLayer|${escapeToken(command.name)}`;
    case "PaintVegetation":
      return `cmd|PaintVegetation|${escapeToken(command.prototype)}|${command.center[0]}|${command.center[1]}|${command.center[2]}|${command.radius}|${command.density}`;
    case "EraseVegetation":
      return `cmd|EraseVegetation|${escapeToken(command.prototype ?? "")}|${command.center[0]}|${command.center[1]}|${command.center[2]}|${command.radius}`;
    case "UpsertPatrolRoute": {
      const tokens = [
        "cmd",
        "UpsertPatrolRoute",
        escapeToken(command.name),
        encodeBool(command.looped),
        String(command.points.length),
      ];
      for (const point of command.points) {
        tokens.push(String(point[0]), String(point[1]), String(point[2]));
      }
      return tokens.join("|");
    }
    case "RemovePatrolRoute":
      return `cmd|RemovePatrolRoute|${escapeToken(command.name)}`;
    case "UpsertVehicleSpawner":
      return `cmd|UpsertVehicleSpawner|${escapeToken(command.name)}|${escapeToken(command.mesh ?? "")}|${escapeToken(command.material ?? "")}|${command.position[0]}|${command.position[1]}|${command.position[2]}|${command.facingYawDeg}`;
    case "RemoveVehicleSpawner":
      return `cmd|RemoveVehicleSpawner|${escapeToken(command.name)}`;
    default:
      throw new Error(`unknown command '${command.type}'`);
  }
}

export function decodeCommand(message) {
  const tokens = splitTokens(message);
  if (tokens[0] !== "cmd") {
    throw new Error("command must start with 'cmd'");
  }

  switch (tokens[1]) {
    case "SelectNextEntity":
    case "SelectPrevEntity":
    case "SelectNextComponent":
    case "SelectPrevComponent":
    case "SelectNextField":
    case "SelectPrevField":
    case "EnterPlayMode":
    case "PausePlayMode":
    case "ResumePlayMode":
    case "StopPlayMode":
    case "Save":
    case "Undo":
    case "Redo":
    case "ReloadSource":
    case "ForcePatch":
    case "ForceRebuild":
      requireTokenCount(tokens, 2, tokens[1]);
      return { type: tokens[1] };
    case "SelectEntity":
      requireTokenCount(tokens, 3, "SelectEntity");
      return { type: "SelectEntity", entityId: Number(tokens[2]) };
    case "SelectComponent":
      requireTokenCount(tokens, 4, "SelectComponent");
      return {
        type: "SelectComponent",
        entityId: Number(tokens[2]),
        componentName: tokens[3],
      };
    case "SelectField":
      requireTokenCount(tokens, 5, "SelectField");
      return {
        type: "SelectField",
        entityId: Number(tokens[2]),
        componentName: tokens[3],
        fieldName: tokens[4],
      };
    case "SetAxis":
      requireTokenCount(tokens, 3, "SetAxis");
      return { type: "SetAxis", axis: parseAxis(tokens[2]) };
    case "ApplyDelta":
      requireTokenCount(tokens, 3, "ApplyDelta");
      return { type: "ApplyDelta", delta: Number(tokens[2]) };
    case "SetTerrainSettings":
      requireTokenCount(tokens, 7, "SetTerrainSettings");
      return {
        type: "SetTerrainSettings",
        sizeMeters: parseVec3(tokens, 2),
        chunkSizeMeters: Number(tokens[5]),
        heightScaleMeters: Number(tokens[6]),
      };
    case "UpsertTerrainPaintLayer":
      requireTokenCount(tokens, 6, "UpsertTerrainPaintLayer");
      return {
        type: "UpsertTerrainPaintLayer",
        name: tokens[2],
        material: parseOptionalToken(tokens[3]),
        tiling: Number(tokens[4]),
        weightmapChannel: Number(tokens[5]),
      };
    case "RemoveTerrainPaintLayer":
      requireTokenCount(tokens, 3, "RemoveTerrainPaintLayer");
      return { type: "RemoveTerrainPaintLayer", name: tokens[2] };
    case "PaintVegetation":
      requireTokenCount(tokens, 8, "PaintVegetation");
      return {
        type: "PaintVegetation",
        prototype: tokens[2],
        center: parseVec3(tokens, 3),
        radius: Number(tokens[6]),
        density: Number(tokens[7]),
      };
    case "EraseVegetation":
      requireTokenCount(tokens, 7, "EraseVegetation");
      return {
        type: "EraseVegetation",
        prototype: parseOptionalToken(tokens[2]),
        center: parseVec3(tokens, 3),
        radius: Number(tokens[6]),
      };
    case "UpsertPatrolRoute": {
      requireMinTokenCount(tokens, 5, "UpsertPatrolRoute");
      const pointCount = Number(tokens[4]);
      requireTokenCount(tokens, 5 + pointCount * 3, "UpsertPatrolRoute");
      const points = [];
      for (let i = 0; i < pointCount; i += 1) {
        points.push(parseVec3(tokens, 5 + i * 3));
      }
      return {
        type: "UpsertPatrolRoute",
        name: tokens[2],
        looped: parseBool(tokens[3]),
        points,
      };
    }
    case "RemovePatrolRoute":
      requireTokenCount(tokens, 3, "RemovePatrolRoute");
      return { type: "RemovePatrolRoute", name: tokens[2] };
    case "UpsertVehicleSpawner":
      requireTokenCount(tokens, 9, "UpsertVehicleSpawner");
      return {
        type: "UpsertVehicleSpawner",
        name: tokens[2],
        mesh: parseOptionalToken(tokens[3]),
        material: parseOptionalToken(tokens[4]),
        position: parseVec3(tokens, 5),
        facingYawDeg: Number(tokens[8]),
      };
    case "RemoveVehicleSpawner":
      requireTokenCount(tokens, 3, "RemoveVehicleSpawner");
      return { type: "RemoveVehicleSpawner", name: tokens[2] };
    default:
      throw new Error(`unknown command '${tokens[1]}'`);
  }
}

export function encodeEvent(event) {
  switch (event.type) {
    case "SelectionChanged":
      return [
        "evt",
        "SelectionChanged",
        String(event.cursor.entityId),
        escapeToken(event.cursor.componentName),
        escapeToken(event.cursor.fieldName),
        event.cursor.axis ?? "",
      ].join("|");
    case "FieldEdited":
      return [
        "evt",
        "FieldEdited",
        String(event.cursor.entityId),
        escapeToken(event.cursor.componentName),
        escapeToken(event.cursor.fieldName),
        event.cursor.axis ?? "",
        ...encodeValue(event.newValue),
      ].join("|");
    case "PlayModeChanged":
      return `evt|PlayModeChanged|${parsePlayState(event.state)}`;
    case "PatchApplied":
    case "WorldRebuilt":
    case "Saved":
    case "UndoApplied":
    case "RedoApplied":
    case "Reloaded":
      return `evt|${event.type}`;
    case "PatchRequiresRebuild":
      return `evt|PatchRequiresRebuild|${escapeToken(event.reason)}`;
    case "RuntimeSliceUpdated":
      return `evt|RuntimeSliceUpdated|${escapeToken(event.path)}`;
    default:
      throw new Error(`unknown event '${event.type}'`);
  }
}

export function decodeEvent(message) {
  const tokens = splitTokens(message);
  if (tokens[0] !== "evt") {
    throw new Error("event must start with 'evt'");
  }

  switch (tokens[1]) {
    case "SelectionChanged":
      requireTokenCount(tokens, 6, "SelectionChanged");
      return {
        type: "SelectionChanged",
        cursor: {
          entityId: Number(tokens[2]),
          componentName: tokens[3],
          fieldName: tokens[4],
          axis: parseOptionalAxis(tokens[5]),
        },
      };
    case "FieldEdited":
      if (tokens.length < 8) {
        throw new Error("FieldEdited requires at least 8 tokens");
      }
      return {
        type: "FieldEdited",
        cursor: {
          entityId: Number(tokens[2]),
          componentName: tokens[3],
          fieldName: tokens[4],
          axis: parseOptionalAxis(tokens[5]),
        },
        newValue: decodeValue(tokens.slice(6)),
      };
    case "PlayModeChanged":
      requireTokenCount(tokens, 3, "PlayModeChanged");
      return { type: "PlayModeChanged", state: parsePlayState(tokens[2]) };
    case "PatchApplied":
    case "WorldRebuilt":
    case "Saved":
    case "UndoApplied":
    case "RedoApplied":
    case "Reloaded":
      requireTokenCount(tokens, 2, tokens[1]);
      return { type: tokens[1] };
    case "PatchRequiresRebuild":
      requireTokenCount(tokens, 3, "PatchRequiresRebuild");
      return { type: "PatchRequiresRebuild", reason: tokens[2] };
    case "RuntimeSliceUpdated":
      requireTokenCount(tokens, 3, "RuntimeSliceUpdated");
      return { type: "RuntimeSliceUpdated", path: tokens[2] };
    default:
      throw new Error(`unknown event '${tokens[1]}'`);
  }
}

export const EditorProtocol = {
  encodeCommand,
  decodeCommand,
  encodeEvent,
  decodeEvent,
};
