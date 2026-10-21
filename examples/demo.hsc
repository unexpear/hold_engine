scene "Hold Demo"

# Authored runtime scene for the native demo host.
# `examples/demo.rs` still attaches demo-only behavior after spawn:
# - `AngularVelocity` on `model` for SpinSystem
# - `CameraMode` on `camera` for first/third-person runtime control
# Those are intentional host-side systems for now, not part of `.hsc`.

entity "model" {
  # Spawn target for the demo's spinning runtime behavior.
  transform {
    pos 0 8 0
    rot 0 0 0 1
    scale 1 1 1
  }
}

entity "sun" {
  # Main authored directional light for terrain and mesh shading.
  light {
    dir 0.4 0.8 0.3
    color 1 1 1 1
    intensity 1
    ambient 0.15
  }
}

entity "player" {
  # PlayerController uses this as the authored spawn point.
  transform {
    pos 0 20 0
    rot 0 0 0 1
    scale 1 1 1
  }
}

entity "camera" {
  # Lens settings and active-camera ownership live in scene data.
  transform {
    pos 0 20 0
    rot 0 0 0 1
    scale 1 1 1
  }
  camera { fov 60 near 0.1 far 1000 }
  active_camera
}
