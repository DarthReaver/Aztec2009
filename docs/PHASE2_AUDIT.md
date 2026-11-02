# Phase 2 audit: dependency graph tools (Lessons 3 and 6)

## Coverage
| Lesson topic | Status |
|---|---|
| Hypergraph: scene hierarchy, shape-node toggle, input/output connections, history / material / anim-curve nodes, connection label on hover, delete connection (button or Delete/Backspace), vertical/horizontal layout | Done (Window > Hypergraph) |
| Connection Editor: Reload Left/Right, Clear All, pick output then input, connect/disconnect | Done; compound attributes connect channel-by-channel via `connectAttr tex.outColor mat.color` (no expandable tree) |
| Hypershade: Materials/Textures/All tabs, create Lambert/Blinn/Phong/Surface Shader and File/Checker/Grid/Bricks/Cloth/Dots/Noise/Ramp, assign, select users, delete unused | Done (simplified list, not a node canvas; use Hypergraph to see the graph) |
| Material attributes: shading model, color, incandescence, transparency, specular color; texture alpha drives transparency | Done, in both renderers |
| Texture placement (repeat, offset, rotate, gain) | Done, **embedded in the texture node** rather than a separate place2dTexture |
| Add Attribute (float/int/bool, min/max/default, keyable), delete, Channel Control (keyable/locked), `setAttr -lock/-keyable` | Done; Channel Box now lists only keyable/animated/connected channels |
| Sets: Quick Select Set, Relationship Editor, select members, outliner glyph | Done |
| Camera bookmarks, saved window layouts | **Not done** |
| Shading group nodes, bump/displacement, Ramp Shader, real place2dTexture | **Not done** (assignment is drawn as a synthetic "shading group" edge) |

## Effects on the program
* File format: new node types `texture`, `objectSet`; `addAttr` gains `-min -max -k -at`; `setAttr -lock/-keyable` lines. Old `assignTexture` lines still load (converted to a texture node). New files with texture nodes are not readable by older builds.
* Behavior change: materials now have `shaderModel` (default Blinn keeps old look; `lambert` command makes no highlight), so scenes using `lambert` get no specular.
* Channel Box hides non-keyable plugs (e.g. pivots) by default.
* OBJ/FBX export still uses the color-map path through `color_map()`.

## Verification
* 138 unit tests pass, including `phase2_lesson_walkthrough` (shader network, hyperShade, attributes, locks, sets, save/reload, renderer receives model and placement).
* Not verified: GUI windows were compiled but not clicked through; run `./aztec2009` and open them from the Window menu.
