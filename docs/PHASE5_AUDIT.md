# Phase 5 audit: NURBS curves and surfaces (Lesson 15, and the NURBS parts of Lessons 1, 2, 5, 16)

## Coverage
| Lesson topic | Status |
|---|---|
| Create > NURBS Primitives: sphere, cylinder, cone, torus, plane, circle, square; spans/sections/radius as channels | Done (`sphere`, `cylinder`, `cone`, `torus`, `nurbsPlane`, `circle`, `nurbsSquare`) |
| EP Curve Tool and CV Curve Tool, degree | Done: Create menu tools (click on the grid, Enter) and `curve -d N -p ... / -ep ...` |
| Duplicate curve + Scale X -1, Attach Curves (connect) | Done (`attachCurve`; joins the closest ends, removes the originals) |
| Extrude at path (profile along path), Revolve, Loft, with live history | Done: `extrude`, `revolve`, `loft`. Moving or editing the input curves updates the surface until history is deleted |
| Isoparms: insert, duplicate surface curves, detach, attach surfaces | Done through parameters: Edit NURBS window or `insertKnotSurface`, `duplicateCurve`, `detachSurface`, `attachSurface`. **Isoparms cannot be picked by mouse** |
| Move seam | Done: `moveSeam -isoparm K` (re-indexes the periodic direction) |
| CV selection and editing, Move tool Reflection | Done: component mode on a NURBS object selects CVs (`obj.cv[i][j]`), the hull is drawn as the component mesh; reflection mirrors CV edits |
| Freeze Transformations, Center Pivot, Delete History, Duplicate, Group, materials, textures | Done for NURBS (freeze deletes the history first) |
| NURBS to polygons | Done (`nurbsToPoly`); Smooth/Bevel on the result are the polygon tools |
| Snap to curve (hold C) | **Not done** |
| Planar, boundary, trim, rebuild surface, convert polygons to NURBS, texturing NURBS (Lesson 16) | Phase 6 (rebuildCurve, closeCurve/openCurve already exist) |

## Differences from Maya worth knowing
* Splines are non-rational: a "circle" or "sphere" is a cubic periodic spline through 8 points, within about 1% of the true radius between knots. Maya's are exact (rational).
* A NURBS surface is stored as the tessellation (a mesh shape) plus its control net, so shading, picking, materials, OBJ/FBX export, instancing and framing work unchanged; polygon modeling tools refuse it with a message pointing at `nurbsToPoly`. UV editing and 3D painting need a polygon object.
* Detach / attach / insert / move seam / rebuild drop the surface's history (Maya keeps attach and detach nodes).
* Surfaces are displayed with a tessellation of 4 samples per span; wireframe mode shows those lines in addition to the isoparms.

## Effects on the program
* New node type `nurbsCurve`; mesh shapes gain an optional NURBS source. Saved as `nurbs` lines (degree, knots, CVs, generator name and input curves) right after the nodes; old files are unaffected.
* New menu set Surfaces (F4); Create menu entries; the status-line menu set list has a fourth item.
* `current_shape` and component selection accept NURBS curve shapes.
* Generated surfaces are checked in `flush_meshes` with a hash of their inputs, so scenes without NURBS pay nothing.

## Verification
* 155 tests pass: 9 engine tests (knot insertion keeps shape, split pieces meet, interpolation exact, sphere radius, attach/detach, loft passes through curves, extrude follows the path, move seam) and `phase5_lesson15_catapult_walkthrough` (base curve, mirror and attach, profile extrusion, revolved cap, cylinder with 8 spans, CV edit, detach, duplicate surface curve, loft with live update, delete history, freeze, save and load, polygon tools refused, convert).
* A ray-traced render of sphere, cylinder, torus and a lofted vase was looked at.
* Not verified: the Surfaces menus, curve tools, curve picking and the Edit NURBS window were compiled, not clicked.
