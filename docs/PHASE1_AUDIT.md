# Phase 1 audit: workspace and transform foundations

Lessons: Introduction, 1 (Primitives), 4 (Animation Basics), 5 (Working with Maya).
Tests: 134 pass (108 before the phase). `phase1_lesson_walkthrough` replays the lesson steps that exist.

## 1. Coverage

Built in this phase:

| Item | Result | Notes |
|---|---|---|
| Project (File > Project, `project -new/-set/-q`) | done | `scenes/ images/ sourceimages/ data/ sound/ scripts/ clips/ renderData/ autosave/` and `workspace.mel`; bare scene names open from and save to `scenes/`; image paths are stored relative to the project; opening a scene inside a project's `scenes/` selects the project; dialogs start in `scenes/` and `sourceimages/` |
| Preferences (Window > Settings/Preferences) | done | manipulator size, start shading, time unit, undo levels, autosave, snap steps, default key tangent; saved to `~/.config/aztec2009/prefs.txt`; `prefs -q/-set/-save/-load` |
| Hotkey Editor | partly | bind any key to any command and save; the built-in keys are listed but cannot be rebound |
| Hotbox (hold Space) | partly | a command grid (display, select, modify, tools, menu sets, UI); not Maya's five-zone marking menu layout |
| UI elements (Display > UI Elements) | done | status line, shelf, tool box, channel box, layer editor, outliner, time slider, range slider, command line, help line, Show All / Hide All |
| Show menu by type | done | polygons, lights, cameras, joints, IK handles, locators, emitters, particles, manipulators (NURBS types arrive with Phase 5); `viewport -show` |
| Pivots | done | rotate and scale pivots plus rotate-pivot translate on every transform and joint; Modify > Center Pivot, Edit Pivot (Insert), Reset Pivot; manipulators sit at the pivot; Display > Transform Display (pivots, selection handles); FBX export and import keep pivots; `xform -cp -piv -q -rp -sp` |
| Snapping | partly | grid (move step, rotate step, scale step) and point (vertices, joints, locators); curve snap waits for Phase 5 |
| Interactive creation, primitive options | done | Create > Polygon Primitives > Interactive Creation (two clicks on the grid) and Options... |
| Duplicate Special | done | copies, translate / rotate / scale per copy, group, **instances** (`instance`, `uninstance`) |
| Delete by Type | done | history, non-deformer history, static channels, animation channels, constraints; Delete Unused Nodes |
| Select All by Type | done | polygons, lights, cameras, joints, IK handles, locators, emitters, constraints; `select -hi` |
| Show Last Hidden | done | `showHidden -last` |
| Maya ASCII import (`.ma`) | partly | hierarchy, pivots, joints (orient folded into static joints), explicit and primitive polygon geometry, lambert/blinn/phong colors with file textures, transform animation curves; `file -open lesson.ma` opens it. History modifiers, NURBS, deformers, lights, cameras and face-level shading groups are not carried over and are reported as notes. **Tested only on hand-written samples in Maya's syntax, not on real files from the book's DVD** |

Lesson checklist (menu paths the lessons use):

| Lesson | Step | Aztec |
|---|---|---|
| 1 | Project > Set; Create > Polygon Primitives > Interactive Creation; Plane, Cylinder, Cone | yes |
| 1 | Display > UI Elements > Channel Box / Layer Editor; Window > Attribute Editor | yes |
| 1 | Edit > Duplicate, Duplicate Special; Display > Hide, Show > Last Hidden, Show > All; Undo | yes |
| 1 | NURBS Primitives (sphere, cylinder, plane) | no: Phase 5 |
| 4 | UI Elements > Show All; Window > Outliner; Edit > Group; Select Hierarchy | yes |
| 4 | Modify > Freeze Transformations, Center Pivot; Animate > Set Key; Graph Editor, View > Frame All | yes |
| 4 | Graph Editor Tangents > Flat | no: Phase 7 |
| 5 | Window > Settings/Preferences > Preferences; Hide All UI Elements; Show > Manipulators; grid | yes |
| 5 | Hotkey Editor | yes (custom keys) |
| 5 | Marking Menu Editor | no: Phase 13 |
| 5 | Deform > Soft Modification | no: Phase 3 |
| 5 | General Editors > Channel Control | no: Phase 2 |
| 5 | CV Curve Tool, Curve Fillet, NURBS CVs | no: Phase 5 |
| 5 | Tumble tool options, Edit > As Action / As Tool | no (tool option boxes are covered by Preferences > Snapping) |

## 2. Effect on the program

- **File format.** New attributes `rotatePivot*`, `scalePivot*`, `rotatePivotTranslate*` on transforms and joints,
  and `createNode mesh ... -instanceOf NAME`. Scenes from before the phase load unchanged (the four older test
  scenes with history, textures and texture zones opened and gave the same statistics). A scene that uses pivots or
  instances opened in an older build would lose them.
- **Transform math.** The local matrix is now Maya's `T * Rp * R * Rp^-1 * Sp * S * Sp^-1`. Every place that
  turned a matrix back into translate / rotate / scale now goes through `trs_for_local` / `set_local_matrix`
  (reparenting, ungroup, constraints, `xform` in world space, Freeze Transformations, group push-down).
  Freeze keeps pivots where they are in the world. IK solves joint rotations only and still ignores pivots on joints.
- **Instances.** An instance follows its original's geometry, zones and material; modeling an instance is refused
  with a message (run `uninstance`); deleting the original frees the instances.
- **Speed.** 1500 cubes created, moved and rotated by a script, grouped and centered in 0.27 s; the extra pivot
  plugs add nine attribute reads per transform. No measurable change in the viewport.
- **UI.** The status line gained Grid Snap, Point Snap and Pivot buttons. Space is now tap (four view) or hold
  (hotbox). New windows: Preferences, Hotkey Editor, Project, Polygon Primitive Options.
- **New files on disk.** `~/.config/aztec2009/prefs.txt` (only when you press Save Preferences); autosave files
  `name.autosave.aztec` when autosave is turned on.

## 3. Not verified or known limits

- The window features (hotbox, pivot edit drag, snapping while dragging, interactive creation clicks, the new
  windows) compile, start and are covered through their commands, but I have not clicked through them.
- Real `.ma` files from Maya 2009 may use constructs the importer skips; it lists what it skipped.
- Pivot editing works per object, not per component; no per-component pivots or pivot snapping.
- Hotkeys cannot override the built-in keys' meaning for typing in text fields, by design.
- Carried to later phases: tangent types (7), Channel Control and the Connection Editor (2), NURBS snaps (5),
  Soft Modification (3), marking menus (13).
