# Following the book: honest cross-reference

Method: ~385 menu paths were extracted from *Learning Autodesk Maya 2009 Foundation* and each leaf label was matched against the menus in `src/gui/app.rs` (237 matched exactly, 149 did not; most misses are option-box fields, Toxik/mental ray, or phrasing noise). Real gaps were then checked by hand.

## Added in this audit
- Edit Curves > Curve Fillet (`filletCurve -r R -p1 -p2 -trim -join`), Lesson on curves
- Create > Polygon Primitives > Platonic Solids (`polyPlatonicSolid`; the catapult rock). It makes a plain mesh: there is no live Solid Type channel, so "set Solid Type to Dodecahedron" becomes choosing Dodecahedron in the menu.
- Create > Measure Tools > Distance Tool (menu entry for `distanceDimension`; creates locators at default positions, you move them rather than clicking two points in the view)
- Material `bump`/`bumpDepth` and a specular-color map (connect a texture) in the ray tracer. Not shown in the hardware viewport.

- Second pass: File > Create Reference... and Reference Editor (a window to create, load, unload and remove references), Edit NURBS > Reverse Surface Direction (`reverseSurface`), Create > Sets > Quick Select Set, Edit > Delete All by Type (renamed to match the book)

- Third pass (GUI wiring; compiled and smoke-started only): Panels > Layouts with Save Current Layout / Saved Layouts (in-session, not saved to disk), View > Predefined Bookmarks (Perspective, Front, Side, Top), Window > Settings/Preferences > Reset Settings, Shading > X-Ray Joints, Trax Editor "Set Range To Enabled Clips" and "Import Animation" labels, Paint Effects > Get Brush / Save Brush Preset / Paint Canvas (the first and last open the Visor; Save writes `<brush>.brush`), and a Deformer Membership and Weights window behind Edit Deformers > Edit Membership Tool / Paint Set Membership Tool / Paint Jiggle Weights Tool (buttons for the selected vertices, not painting with a brush in the view).

- Flexors (lesson on rigid skin): `flexor` / Skin > Edit Rigid Skin > Create Flexor... makes a lattice under the joint. What had to change: (1) `lattice` refused skinned meshes, now the lattice goes after the skin in the history; (2) the lattice needed to bend with the skeleton (points before the joint follow the parent bone, points after follow the joint, blended), which is new evaluation code in `src/dag/lattice.rs`; (3) lattice and flexor channel edits now re-evaluate the mesh; (4) deleting a flexor removes its deformation from the mesh (before, `delete` left the ffd node behind); (5) lattices are now drawn in the views, hidden by Show > Deformers; (6) points on the lattice boundary no longer flicker in and out because of rounding. The swelling at a bend (`flexorRounding`, `flexorLength`, `flexorCreasing` on the flexor transform) is my own approximation, not Maya's exact flexor math. Points can be keyed or driven (`setDrivenKeyframe ... flexorShape.pt_1_2_1_x`), which is what the book's Set Driven Key steps need; selecting all points and loading them in the Set Driven Key window was not checked in the GUI.

## Still differs from the book (cannot follow literally)
- Sculpt and Joint Cluster flexors (greyed out in the option window). The Skin bind option boxes and Edit Smooth Skin are absent.
- "Two Panes Stacked" layout (only single, four, persp/outliner exist). Saved layouts are lost on exit.
- Convert Selection > To UVs, Layers > Copy Layer, Interactive Shading, Custom Smoothness, IPR menu variants.
- Toxik, mental ray, Python: out of scope.

## Verification limits
Tests (209) and a GUI start-up smoke pass. No screenshot or input-automation tool exists on this machine, so no menu or window has been operated by a person or script; the book has not been followed click by click.
