# Phase 9 audit: deformers and Paint Effects (Lessons 18 and 19)

## Coverage
| Lesson topic | Status |
|---|---|
| Nonlinear deformers (bend, flare, sine, squash, twist, wave) | Done; **wave is new** (`nonLinear -type wave`: amplitude, wavelength, offset, minRadius, maxRadius, dropoff). All have an `envelope` channel |
| Wire deformer (dropoff distance, scale, rotation) | Done: `wire -dds D mesh curve`; channels `dropoffDistance`, `scale`, `rotation`, `envelope`. Points near the base curve follow the moved or reshaped curve with a smooth falloff. Several wires on one mesh are allowed |
| Point on curve | Done: `pointOnCurve -ep N` or `-p PARAM`; the locator drags that point of the curve |
| Cluster | Done on mesh vertices and on curve CVs (`cluster`); the handle moves, rotates and scales the points |
| Soft modification | Done: `softMod -radius R -falloff smooth|linear|ease`; channels `falloffRadius`, `falloffType` (**no editable falloff curve**) |
| Edit Membership, paint set membership / weights | `deformerMembership -n NODE -add|-remove|-set|-q`, `deformerWeights -n NODE -v V [-smooth]` (commands only; there is no membership tool in the views) |
| Jiggle deformer (stiffness, damping, ignore transform, weights) | Done: `jiggle`; channels `stiffness`, `damping`, `jiggleWeight`, `motionMultiplier`, `ignoreTransform`. It is simulated frame by frame: it follows steps of 1 to 2 frames forward and restarts from rest when time jumps or goes back. Weights per point by `deformerWeights` (**no Paint Jiggle Weights tool**) |
| Deformation order | `reorderDeformer NODE up|down|front|back`, `listDeformers`; Deform menu |
| Paint Effects canvas | `paintCanvas`: size, clear color, brush strokes (flat 2D rendering of tubes, leaves and flowers), save as PNG, Visor window buttons |
| Visor and brush presets | Presets by folder (trees: gloppy, oak, pine, bush; grasses: astroturf, grassBermuda; flowers: daisy, vine; fun: jetTrail, airbrush). **The preset set is a small built-in one, not Maya's full Visor** |
| Painting on geometry, Make Paintable | `makePaintable`, `pfxStroke -b BRUSH -surface MESH -pts "x y z ..."`, and the Paint Effects Tool in the views (drag over a paintable surface). `autoPaint` scatters strokes over a surface |
| Stroke attributes: display percent, draw as mesh, surface offset | Channels `displayPercent`, `drawAsMesh`, `surfaceOffset` on the stroke shape. Draw as mesh off shows edges as lines in the views; final renders always use meshes |
| Edit brush attributes (tubes per step, length, widths, gravity, brush width, global scale, segments, curl, branching, leaves, flowers, colors, wind) | All are channels of the brush node (set with `setAttr`, or the Visor window sliders); strokes regrow live |
| Share One Brush, Get Settings from Selected Stroke, Save Brush Preset, Blend Brush / Blend Shading | `shareBrush`, `brushFromStroke`, `brushPreset -save / -load` (text file), `blendBrush -a -b -amount [-shading]` |
| Paint Effects to polygons | `strokeToPoly`: a mesh with stem, leaf and flower materials (quads) |
| Test render of Paint Effects | Strokes are ordinary meshes with materials, so Render and `rayRender` show them |

## Not done / differences
* No Local Influence for lattices, no falloff curve for soft modification, no tools for membership or jiggle weight painting in the views.
* Paint Effects is not Maya's: tubes, branching, leaves and flowers are generated from a smaller set of brush channels, there is no texturing of tubes, no wind animation over time, no particle-like brush types (fire, clouds, lightning) and no pressure mapping.
* The 2D canvas is a bitmap with flat shading; it has no undo, wrap or symmetry.
* Jiggle needs playback (or stepping) forward; scrubbing backward or far restarts the simulation.

## Effects on the program
* New history node types `wave`, `wire`, `cluster`, `softMod`, `jiggle`; deformer data (handle, bind inverse, base curve, center, weights) is saved on the node line. Curve cluster and point-on-curve deformers are saved as `curveDriver` lines.
* New node type `brush`; strokes are mesh shapes with `-strokebrush/-strokeseed/-strokepath/-strokemats` and regrow after load.
* `Scene::final_render` is set while rendering final images.
* New commands: wire, cluster, softMod, jiggle, pointOnCurve, reorderDeformer, deformerMembership, deformerWeights, listDeformers, makePaintable, paintBrush, pfxStroke (the name `paintStroke` already belongs to the 3D paint tool), autoPaint, shareBrush, brushFromStroke, blendBrush, brushPreset, strokeToPoly, strokeInfo, paintCanvas.

## Tests
`phase9_deformers`, `phase9_jiggle_and_deformation_order`, `phase9_paint_effects`, `phase9_canvas` walk the lesson workflows; 171 tests pass in all. The GUI windows and the Paint Effects Tool are compiled and smoke started, not clicked through.
