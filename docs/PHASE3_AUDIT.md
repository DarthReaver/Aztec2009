# Phase 3 audit: polygon modeling (Lessons 2 and 7)

## Coverage
| Lesson topic | Status |
|---|---|
| Extrude (faces, edges), repeat last (G), Keep Faces Together on/off | Done; `polyOptions -kft`, Edit Mesh checkbox, recorded as `keepFacesTogether` on the extrude node |
| Split Polygon Tool, Insert Edge Loop Tool (weight) | Already present (GUI tools; `polySplitRing -t`) |
| Offset Edge Loop | New: `polyOffsetLoop -offset`, Edit Mesh menu |
| Slide Edge | New: `polySlideEdge -t -1..1` (menu offers fixed steps, no MMB drag) |
| Delete Edge/Vertex, Merge Edge, Merge Vertices, Bridge, Collapse, Bevel | Already present |
| Mirror Geometry, Duplicate Special instance mirror | Already present |
| Mirror Cut | New: `polyMirrorCut -axis x [-offset N] [-negative]`; the cutting plane is implicit (no mirrorCutPlane object to move) |
| Smooth Mesh Preview (keys 1/2/3), Smooth Mesh Preview to Polygons | New: `smoothPreview -l 1\|2\|3 -dv N`, `polyConvertPreview`, hotkeys 1 2 3 |
| Display > Polygons > Face Normals / Border Edges | Face Normals new (`polyOptions -dn`, `viewport -normals`); Border Edges existed |
| Select Border Edge Tool | New: `polySelectBorder` |
| Move Tool Reflection (symmetric editing) | New: `symmetricModelling -s 1 -axis x`; mirrors vertex edits about the local axis plane; Edit Mesh > Reflection |
| Insert Edge Loop: Auto Complete off / multiple loops | **Not done** |
| Wireframe on Shaded | Already present |
| Marking menu editor, retain component spacing option | **Not done** |

## Effects on the program
* New history node kinds `offsetLoop`, `slideEdge`, `mirrorCut`; extrude nodes gain a `keepFacesTogether` plug (old files default to 1 = old behaviour). Mesh shapes gain `displaySmoothness` and `smoothLevel` plugs. Preference `keep_faces_together` is saved in prefs.txt.
* `selectMode` now takes the shape from the object selection, fixing the known quirk where a stale component node from the previous mode was reused.
* Smooth preview is not applied to meshes with per-face texture zones (they render as the cage).
* Rendering cost: one extra Catmull-Clark pass per preview-enabled mesh, cached.

## Verification
* 139 tests pass including `phase3_lesson_walkthrough` (extrude options, offset loop, slide, border select, mirror cut symmetry, smooth preview render and bake, reflection).
* Not verified: the new Edit Mesh menu items and 1/2/3 hotkeys were compiled, not clicked.
