# Motion Semantics

Issue: #648

Use separate names for timing and spatial shape:

- `velocity_profile` controls point-to-point timing. It does not describe the
  geometric path. Examples: `natural`, `instant`, `linear`, `ease_in_out`.
- `path` controls spatial shape. It belongs to `act_stroke` and supports line,
  arc, circle, cubic Bezier, polyline, and Catmull-Rom paths.

`act_aim` and `act_drag` are point-to-point tools. Their style or
`velocity_profile` changes how quickly the pointer progresses between endpoints;
callers that need a curved, closed, or multi-waypoint spatial path should use
`act_stroke`.

Migration:

- New `act_drag` calls should send `velocity_profile`.
- The old `act_drag.curve` field remains a compatibility alias for `natural`,
  `instant`, `linear`, and `ease_in_out`.
- The old `act_drag.curve = "bezier"` value is rejected with an explicit
  parameter error. Use `act_stroke.path.kind = "cubic_bezier"` for spatial
  Bezier movement, or `velocity_profile = "ease_in_out"` for point-to-point
  timing.
