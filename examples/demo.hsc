scene "Hold Demo"

runtime_slice {
  terrain {
    size 1024 256 1024
    chunk_size 64
    height_scale 80
  }
  paint_layer "sand" {
    material "materials/sand"
    tiling 6
    channel 0
  }
  vegetation_prototype "palm" {
    mesh "meshes/palm"
    material "materials/palm"
    density 2.5
    random_yaw 1
    align_to_normal 1
  }
  vegetation_paint "palm" {
    center 120 0 180
    radius 80
    density 0.9
  }
  static_mesh_actor "rock_large" {
    mesh "meshes/rock_large"
    material "materials/rock"
    pos 80 0 120
    rot 0 32 0
    scale 8 5 7
  }
  water_volume "ocean" {
    material "materials/ocean"
    center 512 0 512
    extents 1024 6 1024
    wave_strength 0.35
  }
  spawn_point "player_start" {
    kind "player"
    pos 32 4 32
    yaw 180
  }
  patrol_route "route_a" {
    looped 1
    point 96 0 96
    point 160 0 128
    point 128 0 192
  }
  vehicle_spawner "jeep" {
    mesh "meshes/jeep"
    material "materials/jeep"
    pos 56 0 56
    yaw 90
  }
  breakable_prop "crate" {
    intact_mesh "meshes/crate"
    broken_mesh "meshes/crate_broken"
    material "materials/crate"
    pos 70 0 68
    rot 0 0 0
    scale 2 2 2
    hit_points 40
    break_impulse 22
  }
}

entity "model" {
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
  light {
    dir 0.4 0.8 0.3
    color 1 1 1 1
    intensity 1
    ambient 0.15
  }
}

entity "player" {
  transform {
    pos 0 20 0
    rot 0 0 0 1
    scale 1 1 1
  }
}

entity "camera" {
  transform {
    pos 0 20 0
    rot 0 0 0 1
    scale 1 1 1
  }
  camera { fov 60 near 0.1 far 1000 }
  active_camera
}
