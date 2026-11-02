# Phase 4 audit: polygon texturing with real UVs (Lessons 3 and 8)

## Coverage
| Lesson topic | Status |
|---|---|
| UV Texture Editor showing the UVs of the selected object, with the texture behind them | Done (Window > UV Texture Editor); objects without their own UVs still get the older zone/projection editor |
| Create UVs: planar X/Y/Z, best plane, cylindrical, spherical, automatic; on faces or whole objects | Done: `polyProjection -type ... -md`, `polyAutoProjection`; menu Polygons > Texturing > Create UVs. No on-screen projection manipulator: change the mapping by re-projecting or moving the UVs |
| Select UVs, UV shells, convert face selection to UVs | Done: click, box, Select Shell, From Faces, `uvSelect` |
| Move / scale / rotate / flip UVs | Done: drag, buttons, `polyEditUV` acts on the UV selection |
| Layout UVs, separate shells at folds, no overlap | Done: `polyLayoutUV -sf` (shelf packing into 0..1) |
| Cut / Sew UV edges, Unfold | Done: `polyMapCut`, `polyMapSew` (use selected mesh edges), `unfold` (Laplacian relax; not Maya's solver) |
| Normalize UVs, delete UVs, checker display | Done |
| 3D Paint Tool: assign/edit textures at a resolution, duplicate texture into `3dpainttextures/`, paint, erase back to original, save, reload | Done: Window > 3D Paint Tool, `paintAssign/Options/Stroke/Save/Reload` |
| Paint: Extend Seam Color, Screen Projection, save on stroke | Save on stroke done; seam extension and screen-projection toggle **not done** (painting follows the UVs, so stretched UVs paint stretched) |
| Ramp texture on eye color | The `gradient` texture node maps to Color; its colors are not editable (**not done**) |

## Effects on the program
* New data: `MeshData.uv_set` (UV points + per-corner indices). Saved as `meshUvs` lines; old files load unchanged. When a set exists it replaces the projection attributes and texture zone maps for that shape.
* After modeling, UVs follow surviving faces (fingerprint, then index); new faces get a small flat patch to the right of the 0..1 square and must be mapped again.
* OBJ/FBX export writes the edited UVs; OBJ import turns file UVs into an editable set when the faces keep their order.
* `polyAutoProjection` now creates real UVs (previously a procedural box mapping).
* Instances copy the UV set of the original.

## Verification
* 145 tests pass, including `phase4_lesson_walkthrough` (project, select, move, layout inside 0..1, cut/sew, save/load), `phase4_paint_on_the_model` (ray paint, erase, save, error cases), geometry tests of automatic mapping/cut/sew/relax, and an OBJ UV round trip.
* Not verified: the UV editor window, the Create UVs menu and dragging the paint tool were compiled, not clicked.
