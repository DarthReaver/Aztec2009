# Final audit: "Learning Autodesk Maya 2009 Foundation" in Aztec 2009

Every lesson of the book was read against the program in Phases 1 to 13 (one audit file per phase, linked below). Phase 14 re-checked
the lesson objectives, closed the gaps that were cheap to close (Sculpt Geometry Tool, camera bookmarks, help for every command) and
added an end to end script, `docs/examples/tour.mel`, that touches every project of the book and is run by the test suite.

How to read the status: **done** means the lesson's steps can be followed with the same menu names or MEL commands; **simplified** means the
workflow works with a documented difference; the last column lists what is missing. Nothing is marked done that a test does not exercise, and
no test clicks through the windows: every window and menu was compiled and smoke started (the program opens and runs), not operated by a person.

| Lesson | Topic | Status | Phase | What is missing or different |
|---|---|---|---|---|
| 1 | The Maya interface | done | 1 | Tumble tool options, As Action / As Tool |
| 2 | Polygon modeling basics | done | 3 | Insert Edge Loop auto complete and multiple loops; retain component spacing |
| 3 | Shading and texturing basics, Hypershade | simplified | 2, 4 | The Hypershade is a list, not a node canvas; no shading group nodes, bump or displacement nodes, Ramp Shader |
| 4 | Keyframing, Graph Editor, preferences | done | 7 | Weighted tangents, breakdowns, ghosting |
| 5 | Workspace, selection, transforms (project 1) | done | 1 | Pivot editing per object only; no snap to curve (hold C) |
| 6 | Dependency graph, connections, Hypergraph | done | 2 | Camera bookmarks done in Phase 14 (not saved in the scene); saved window layouts not done |
| 7 | Polygon modeling of the character | done | 3 | See lesson 2 |
| 8 | UV Texture Editor, 3D Paint Tool | done | 4 | No projection manipulator; no seam extension or screen projection in 3D Paint; the gradient texture's colors are not editable |
| 9 | Skeleton and joints | done | 8 | Per-component pivots |
| 10 | Skinning, Artisan, blend shapes | simplified | 8, 14 | **Sculpt Geometry Tool added in Phase 14** (push, pull, smooth, relax, erase, flood, reference update; no tablet pressure, no brush size hotkey); lattice flexor added (`flexor`, Skin > Edit Rigid Skin > Create Flexor...; sculpt and joint cluster flexors not available); no Edit Membership tool (weights are painted) |
| 11 | Inverse kinematics, constraints | done | 8 | Chains longer than two bones and the single chain solver use cyclic coordinate descent, not Maya's solver |
| 12 | Rig organization, jiggle, character sets | done | 8, 9 | Local Influence of lattices not done |
| 13 | Animating with the rig, cycles, baking, clips | done | 8, 7, 12 | Reference edits survive an unload: not done |
| 14 | NURBS curves | done | 5 | Snap to curve |
| 15 | NURBS surfaces: extrude, attach, detach, loft | done | 5, 6 | Boundary / Square / Bevel Plus, curve-on-surface editing |
| 16 | NURBS texturing, convert to file texture | simplified | 6 | Interactive Placement Tool has no on-surface manipulator; no color presets; Brownian and other 3D textures are evaluated in 2D (no place3dTexture, no reference objects) |
| 17 | Rig extras: renaming, distance tool, reactive driven keys, inherits transform | done | 8 | none known |
| 18 | Paint Effects | simplified | 9 | A built-in set of ten brushes and one generator; no textured tubes, wind over time, pressure, fire / cloud brush types |
| 19 | Deformers | simplified | 9 | No Local Influence, no soft modification falloff curve, no membership or jiggle weight painting tools (commands exist) |
| 20 | Lights and effects, batch render, fcheck | done | 10 | Depth map shadows are ray traced (resolution and filter size blur the edge); no IFF / EXR / TIFF output |
| 21 | Rendering: render view, IPR, renderers | simplified | 10 | **mental ray out of scope**; Hardware renderer is the ray tracer without ray tracing; Vector renders bitmaps; IPR re-renders the region after every command |
| 22 | More animation, playblast | done | 7, 8 | Playblast writes frames only (no movie, no sound) |
| 23 | Non-linear animation: Trax, layers, sound, cameras | done | 12 | Per channel offsets; time warp not shown in the Graph Editor; sound has no audio output and cannot be dragged |
| 24 | Rigid bodies | done | 11 | Concave active bodies; edge to edge box contacts; continuous collision |
| 25 | Motion path, make live, normal constraint | done | 12, 7 | No path marker manipulator; live surfaces are polygon only |
| 26 | Particles | simplified | 11 | Fixed set of render types; no per-particle expressions or arbitrary per-particle attributes; no 3D textures on blobby surfaces |
| 27 | MEL scripting | done | 13 | No arrays or vectors in MEL; expressions are a subset; no Python; interface commands lack formLayout, tabLayout, menus |
| 28 | Render layers and passes | done | 10 | No 32 bit EXR motion vectors, no mental ray pass contribution maps |
| 29 | Toxik compositing | **out of scope** | - | A separate product |

## Closed in Phase 14

* Sculpt Geometry Tool (Lesson 10): `sculpt`, Mesh > Sculpt Geometry Tool with a Tool Settings window and a viewport tool.
* Camera bookmarks: `cameraBookmark`, View > Bookmarks.
* Every command has a help entry (a test keeps it that way) and `help COMMAND` works for the aliases.
* `docs/examples/tour.mel`: one script that builds a crate, NURBS, a skinned leg with IK, deformers, a motion path, Paint Effects, lights
  and render layers, dynamics, a Trax clip, an expression and a script window, renders and saves, then reloads the scene. A test runs it.

## Known limits that cut across lessons

* Rendering is CPU ray tracing; large images take time. The windows use eframe / OpenGL and need a desktop.
* Undo in the workspace restores whole-scene snapshots; very large scenes make it slow.
* Scene files are ASCII `.aztec` (not Maya `.ma` / `.mb`); OBJ and FBX import and export exist.
* The audits list per phase what was verified by tests. Windows, menus and viewport tools were not operated by a person.

## Phase audits

PHASE1_AUDIT.md workspace and transforms, PHASE2_AUDIT.md dependency graph tools, PHASE3_AUDIT.md polygon modeling, PHASE4_AUDIT.md UVs and
painting, PHASE5_AUDIT.md NURBS, PHASE6_AUDIT.md NURBS trimming and textures, PHASE7_AUDIT.md animation, PHASE8_AUDIT.md character setup,
PHASE9_AUDIT.md deformers and Paint Effects, PHASE10_AUDIT.md lighting and rendering, PHASE11_AUDIT.md dynamics, PHASE12_AUDIT.md Trax, cameras
and paths, PHASE13_AUDIT.md MEL and customization.
