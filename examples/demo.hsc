scene "Hold Demo"

# Authored runtime scene for the native demo host.
# `examples/demo.rs` still attaches demo-only behavior after spawn:
# - `CameraMode` on `camera` for first/third-person runtime control
# Preview/play gating for authored emitters and velocity
# Those are intentional host-side behaviors for now, not part of `.hsc`.

entity "model" {
  # Scene-authored spin data for play mode.
  transform {
    pos 0 8 0
    rot 0 0 0 1
    scale 1 1 1
  }
  velocity {
    linear 0 0 0
    angular 0.4 1.2 0
  }
}

entity "model_fx" {
  # Authored spark fountain above the model.
  transform {
    pos 0 1.6 0
    rot 0 0 0 1
    scale 1 1 1
  }
  particle_emitter {
    emitting 1
    rate_per_second 40
    max_particles 160
    spawn_radius 0.15
    direction 0 1 0
    spread 0.45
    speed_min 2.5
    speed_max 5.5
    lifetime_min 0.4
    lifetime_max 1.1
    gravity_scale 0.9
    linear_damping 1.6
    start_size 0.18
    end_size 0.02
    start_color 1 0.85 0.25 1
    end_color 1 0.2 0.05 0
    seed 1
  }
  parent "model"
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
