# Phase 6 audit: NURBS trim, rebuild, convert and texturing (Lessons 15 and 16)

## Coverage
| Lesson topic | Status |
|---|---|
| Project Curve on Surface, Trim Tool, Untrim, trimmed display | Done: `projectCurve [-dir x y z]`, `trim [-pt u v] [-discard] [-keep]`, `untrim`; closed curves cut holes or keep islands, open curves running edge to edge keep one side. Trimmed surfaces are tessellated with clipped boundary cells |
| Planar surface from a closed curve | Done (`planarSurface`, a flat patch trimmed to the outline) |
| Rebuild Surfaces / Curves, Open / Close Curves | Done (`rebuildSurface -su -sv -du -dv`, `rebuildCurve`, `closeCurve`, `openCurve`) |
| Convert NURBS to polygons | Done (`nurbsToPoly`) |
| NURBS UVs are the surface parameters; checker on NURBS | Done (mesh UVs are 0..1 over U and V; the checker shows stretching and pinching at poles) |
| Procedural textures: Ramp (U, V, diagonal, radial, circular, box; interpolation; editable entries), Noise, Fractal, Brownian (2D), Cloth | Done as texture nodes with attributes in the Attribute Editor and Hypershade create bar |
| Mapping a texture into another one's attribute (ramp entry color, Color Gain) | Done by connecting `tex.outColor` to `ramp.eCol1` or `ramp.colorGain`; the whole network is baked (512 x 512) for display and rendering |
| Break connection, duplicate shading network, delete unused nodes | `disconnectAttr`, `duplicateShadingNetwork`, `hyperShade -deleteUnusedNodes` / `optimizeScene` |
| Interactive Placement Tool | Placement values (center, repeat, rotation, wrap, default color outside) in a Texture Placement window and `placeTexture`; **no manipulator dragged on the surface** |
| Convert to File Texture | Done (`convertToFileTexture -res N`, bakes the color network into sourceimages/) |
| Export / import a shading network | Done (`exportShaders`, `importShaders`, files in renderData/shaders; Aztec scene format, not Maya ASCII) |
| Select Using Constraints (hard edges) | Done (`polySelectConstraint -sm hard`) |
| Color presets in the color chooser | **Not done** |
| 3D textures, place3dTexture, texture reference objects | **Not done** (Brownian is evaluated in 2D) |
| Boundary / Square / Bevel Plus, curve-on-surface editing, trim of open curves not reaching the edges | **Not done** |

## Differences from Maya
* Trim boundaries are polylines in parameter space (the projected curve is kept as a curve too), and boundary cells are clipped by straight chords; a very coarse surface shows slightly polygonal edges.
* Texture networks are baked to one image per material, so a procedural texture has the resolution of 512 x 512 over the UV square, and bump or other channels are not networked (only Color, Color Gain and ramp / cloth colors).
* Rebuild and the other structure-changing edits drop a surface's history and trims.

## Effects on the program
* New saved data: `-trims` and `-onsurface` on `nurbs` lines; texture nodes of the new kinds have more channels (old files load unchanged). Procedural textures previously named `noise` now use the new noise node (same look class, new attributes).
* Rendering: a material whose color is a network is baked and cached by a hash of every input; plain image and pattern textures take the old direct path.

## Verification
* 160 tests pass, including three new engine tests (trim area, projection, rebuild/planar), `texture_networks_bake_ramps_noise_and_cloth` and `phase6_lessons_15_16_trim_rebuild_and_texturing` (window trimmed into a cylinder and restored, rebuild, planar, shading network duplicated, converted to a file texture, optimized away, placed, exported and imported, trims saved and loaded, hard edge selection).
* A render of a trimmed wood-textured cylinder and a cloth sphere was looked at.
* Not verified: the new menu items, the ramp / noise / cloth sections of the Attribute Editor and the placement window were compiled, not clicked.
