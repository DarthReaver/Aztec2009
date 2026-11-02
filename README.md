# Aztec 2009

A Maya 2009 style 3D animation tool for Linux, written in Rust. It opens as a normal desktop window with a smooth, high resolution viewport, and it still has a terminal mode and a scripting shell.

## The book

The program follows *Learning Autodesk Maya 2009 Foundation* lesson by lesson (see `docs/FINAL_AUDIT.md`). `docs/examples/tour.mel` is a script that visits every project of the book: `./aztec2009 -c docs/examples/tour.mel`.

## Start it

- Double click **Aztec 2009.desktop** in this folder, or
- run `./install.sh` once, then open **Aztec 2009** from your application menu (a Desktop shortcut is added too), or
- `./aztec2009` from a terminal.

## Windows

- **Prebuilt:** download `aztec2009-windows-x86_64.zip` from the Actions tab (or a Release), unzip, and double click **Aztec 2009.bat**.
- **From source:** install Rust from https://rustup.rs (with the MSVC build tools), then double click **Aztec 2009.bat**. It builds on first run and starts the app. `build.bat` only builds.
- **Start menu and Desktop shortcuts:** right click `install.ps1` and choose Run with PowerShell (installs to `%LOCALAPPDATA%\Aztec2009`, no admin needed).
- Terminal modes work from `cmd` or PowerShell: `aztec2009.exe --tui`, `aztec2009.exe -c script.mel`.
- Preferences are stored in `%APPDATA%\aztec2009\prefs.txt`.

## Build from source

    ./build.sh            # cargo build --release, copies the binary to ./aztec2009

Needs a Rust toolchain and an OpenGL capable desktop (X11 or Wayland). The prebuilt tarball in `release/` is the old terminal-only build.

## The workspace

Menus (File, Edit, Modify, Create, Display, Window, Animate, Render, Help), status line, shelf with tabs, tool box (Select, Move, Rotate, Scale, layouts), viewports with their own View / Shading / Show / Panels menus, Channel Box and Layer Editor on the right, time slider, range slider, MEL command line and a help line. Window > Outliner and Window > Script Editor open the other panels.

- Viewports render at your screen's native resolution with 2x supersampling when idle, anti-aliased lines, and a lowered resolution only while you orbit or drag.
- Four view layout with top, front, side and perspective (Space toggles it).
- Move, Rotate and Scale manipulators drawn over the selection; Auto Key keys what you change.
- Marquee and click selection, undo / redo, duplicate, group, delete.
- Render > Render Current Frame opens a Render View (640x360 up to 1920x1080, optional red/cyan anaglyph) with Save PNG.


## Polygon modeling

Switch the status line menu set to **Polygons** (F3) to get the Select, Mesh and Edit Mesh menus and the modeling shelf.

- Component masks: **F8** object / component, **F9** vertex, **F10** edge, **F11** face (also the Obj / Vtx / Edge / Face buttons). Ctrl+F9/F10/F11 convert the selection.
- Click, marquee (Shift toggles, Ctrl removes), double-click an edge to select its loop. Backfacing and hidden components are not selectable.
- Move / Rotate / Scale (W / E / R) work on components with the usual manipulators.
- **Ctrl+E** Extrude (faces, or edges in edge mode; the Move manipulator comes up aligned to the face normal), **Ctrl+B** Bevel edges, **Delete** removes faces / edges / vertices, **Ctrl+Delete** deletes edges and cleans vertices, Merge Vertices, Collapse, **G** repeats the last operation.
- **Insert Edge Loop Tool** (click or drag across an edge) and **Split Polygon Tool** (click edges or vertices, Enter to cut).
- Mesh menu: Combine, Separate, Smooth (Catmull-Clark), Triangulate, Mirror Geometry, Reverse Normals.
- Display > Wireframe on Shaded, and the Poly Count heads-up display (total, object, selection).

Edited meshes are stored in `.aztec` files (`-prim custom`), so saving, loading and undo keep them. The same operations exist as commands: `selectMode -face`, `select -r obj.f[2:5]`, `polyExtrudeFacet -ltz 1`, `polyBevel -offset 0.1`, `polySplitRing -t 0.5`, `polyMergeVertex`, `polySmooth -dv 1`, `polyUnite`, `polySeparate`, `polyCount`, ... (type `help`).

## History, animation and textures

- **Construction history.** Every primitive and modeling operation is a history node (`polyCube1`, `polyExtrudeFace1`, `polyBevel1`, `polyTweak1`, ...). They are listed under INPUTS in the Channel Box and their values are editable: change `polyCube1` Width after an extrude and the whole chain re-evaluates. The status line **History** toggle stops new operations from recording history. **Edit > Delete by Type: History** (or `deleteHistory`) bakes the mesh.
- **Shape and Input nodes.** The Channel Box shows the transform, the shape node (UV Mode, UV Tiling, vertex/face counts), INPUTS (history), ANIMATION (curve nodes such as `box_translateX`, `polyTweak1_pnts_0_z`) and OUTPUTS (the material and its color map). Click a curve node to open the **Animation Curve** window to edit keys (time, value, interpolation).
- **Animating vertices.** In vertex, edge or face mode press **S** (Animate > Set Key) to key the selected vertices. Moves go through the tweak node, so a square can be keyed at frame 1, pulled out to a rectangle at frame 24 and it will play back. Key ticks for the keyed vertices appear on the time slider. In scripts: `polyMoveVertex -t x y z`, `keyVertices`.
- **Fill Hole.** Select the border edges around a hole (or nothing, to fill every hole) and choose **Mesh > Fill Hole**; a polygon closes each closed loop. Also `polyCloseBorder`.
- **Right click** over an object selects it and shows the object menu with Object Mode, Vertex, Edge, Face, Duplicate, Group, Delete, Delete History, Assign Color Map.
- **Texture maps.** **Create UVs > Assign Color Map...** (or the Browse button in the Channel Box) picks a PNG or JPEG for the object's material. Mapping modes (Automatic box, Planar X/Y/Z, Cylindrical, Spherical) are under Create UVs and as the shape's UV Mode channel; UV Tiling U/V repeat the map. Color maps show in **Textured** display (key **6**, the checker-ball button, or Display > Textured); renders always use them. `assignTexture "obj" "file.png"` and the built-in patterns `builtin:checker grid bricks stripes dots noise gradient` work in scripts. See *Texturing objects and faces* below.

## Rigging, hierarchy, rendering and editors

- **Hierarchy.** The Outliner is a tree: collapse/expand with the arrows, drag a node onto another to parent it (the world transform is kept), drop on empty space to unparent. **P** parents the selection to the last selected object, **Shift+P** unparents, and the context menu has Rename, Select Hierarchy, Duplicate, Group, Unparent, Delete and Delete History. Double-click (or the name box at the top of the Channel Box) renames. "Shapes" shows the shape nodes.
- **Time slider keys.** Keys of the selection are red markers. Click one to select it (Shift adds), drag selected markers to another frame to retime the animation. The Animation Curve window edits individual keys.
- **Right-click on history.** In the Channel Box, right-click an input node (`polyExtrudeFace1`, ...) and choose Delete This Node, or Delete All History.
- **Skeletons.** Animation menu set: **Skeleton > Joint Tool** (click to chain joints; Enter ends the chain), **IK Handle Tool** (click start joint, then end joint), **Mirror Joint**. **Skin > Bind Skin** (select the skeleton root, then the mesh) smooth binds with distance weights and four influences; **Detach Skin** removes it. Joints and the IK handle are ordinary transforms: move, rotate, key them. Scripts: `joint`, `ikHandle`, `bindSkin`, `detachSkin`, `mirrorJoint`.
- **Ray tracing.** Render View > Renderer > Raytrace (Draft to Production anti-aliasing) renders in the background with shadows, mirror reflections (Reflectivity on the material), color maps and a BVH. `rayRender -w 1280 -h 720 -q 3 -out shot.png` does it from a script.
- **Attribute Editor.** Window > Attribute Editor or **Ctrl+A**: tabs for the object, its shape, history nodes and material, with color pickers, sliders, UV mapping controls, color map, extra attributes and notes.
- **UV Texture Editor.** Window > UV Texture Editor shows the UV layout over the color map with live Mapping / Tiling / Offset / Rotate controls.
- **Images.** PNG, JPEG, BMP, TGA and GIF. Drop an image file on the window to apply it to the selection (or create an image plane when nothing is selected); **Create > Image Plane...** makes a plane with the image's aspect ratio.
- **Speed.** The rasterizer is multi-threaded, display meshes and edge counts are cached, the viewport drops to lower resolution only while you interact and is re-rendered sharp when you stop.

## Import and export (OBJ and FBX)

**File > Import**, **Export All** and **Export Selection** open a file browser that lists `.obj` and `.fbx` files; the format follows the extension. The same thing from a script:

    file -import "model.fbx";                 // or .obj
    file -export "scene.fbx";                 // everything: meshes, joints, materials, animation
    file -export "scene.fbx" -ascii;          // ASCII FBX instead of binary
    file -export "part.obj" -selected;        // only the selection (and its children)

| | OBJ | FBX |
| --- | --- | --- |
| Meshes, normals, UVs | yes | yes (binary 6.x is not read; 7.0 to 7.7, binary or ASCII) |
| Hierarchy and transforms | baked to world space (OBJ has none) | yes, including pre/post rotation, pivots, rotation orders and geometric transforms |
| Materials | `.mtl` next to the file: diffuse color and `map_Kd` texture | diffuse color and diffuse texture |
| Joints | no | yes (LimbNode), without skin weights |
| Animation | no | translate / rotate / scale curves with linear, step or spline keys, converted between time units |

Notes: Z-up files are rotated to Y-up, a material change inside one OBJ object starts a new object (a mesh carries one material), and a mesh with several FBX materials becomes one object per material. Aztec stores UVs as a generated projection (UV Mode), so imported UV layouts are not kept; exports write the generated UVs. Cameras, lights, skin weights and blend shapes are not exported.

## Added in the Maya 2009 style audit

- **Primitives:** Create > Polygon Primitives > Cone, Torus (`polyCone`, `polyTorus`), with the same live history as the other primitives.
- **Nonlinear deformers** (Animation menu set > Deform > Nonlinear): Bend, Twist, Flare, Squash, Sine. Each one is a history node (`bend1`, `twist1`, ...) whose bounds, axis and amounts are editable and keyable in the Channel Box. `nonLinear -type twist -endAngle 90`, or `bend`, `twist`, `flare`, `squash`, `sine` directly.
- **Mesh menu:** Average Vertices (`polyAverageVertex`), Extract (`polyExtract`), Duplicate Face (`polyChipOff -dup 1`).
- **Freeze Transformations** (Modify, `makeIdentity -apply true`): bakes translate / rotate / scale into the vertices as a `transformGeometry` history node, or pushes a group's transform down into its children.
- **Edit:** Ungroup, Duplicate Special with copies, per-copy translate / rotate / scale and an optional group (`duplicateSpecial`).
- **Display:** Hide Selection (Ctrl+H), Show All, Isolate Select (`hide`, `showHidden`, `isolateSelect`).
- **Animation:** Animate > Bake Simulation (`bakeResults -t "1:48"`) keys every frame and folds animation layers and connections into one base curve. Window > Playblast writes every frame to `playblast/frame.NNNN.png`. Window > Settings: Time Unit switches film / PAL / NTSC / show / 60 fps (`currentUnit -time film`).
- **Query commands:** `xform` (world or object space, translation / rotation / scale / bounding box / matrix, query or set), `listRelatives`, `nodeType`, `objExists`, `polyEvaluate`.

## Scripting

Scripts (`-c`, `source`, the Script Editor and the command line) now run structured MEL style code:

    global proc buildRow(string $prefix, int $count, float $gap) {
        for ($i = 0; $i < $count; $i++) {
            polyCube -name ($prefix + $i);
            move ($i * $gap) 0 0 ($prefix + $i);
        }
    }
    buildRow("box", 5, 2.5);
    $x = `getAttr "box3.translateX"`;
    if ($x > 5 && size("abc") == 3) print ("far: " + $x); else print "near";

- `if` / `else`, `for`, `while`, `break`, `continue`, `proc` / `global proc` with `return`, recursion, and procedures called as `myProc 1 2;` or `myProc(1, 2);`.
- Parentheses are expressions anywhere an argument is expected: `move ($i * 2) 0 0`. Operators: `+ - * / %`, comparison, `&& || !`, `?:`, `++ -- += -= *= /=`. Strings concatenate with `+`.
- Backticks run a command and use its result: `` `polyEvaluate -f cube` ``. Any other function-style name, such as `getAttr("a.tx")`, calls the command of that name.
- Functions: `abs sqrt sin cos tan asin acos atan atan2 exp log floor ceil trunc int float pow fmod min max clamp sign deg_to_rad rad_to_deg rand seed size substring tolower toupper strcmp`.
- `int` / `float` / `string` declarations, local variables inside procedures, `global` to share. A script that runs about three million statements is stopped, so a runaway loop does not hang the window.
- Differences from MEL: all numbers are floating point (`5 / 2` is 2.5), and arrays, vectors, `for ($x in $list)` and `eval` are not supported.

## Lights and shadows

Create > Lights: Point, Directional, **Spot**, **Ambient** and **Area**. Select a light and open the Attribute Editor (Ctrl+A) for its settings:

- **Light:** type (switchable), color, intensity, decay rate (none, linear, quadratic, cubic or the soft default), Emit Diffuse / Emit Specular.
- **Spot:** cone angle, penumbra angle (the fade-out past the cone edge) and dropoff. **Area:** width and height (scale the transform too); it is one sided and always gives soft shadows.
- **Shadows:** Cast Shadows, **Light Radius** (Light Angle for a directional light) and Shadow Rays, and Shadow Color. A radius of 0 gives hard shadow edges; raise it to feather them. The viewport shows light and cone gizmos; shadows appear in Render View with **Renderer: Raytrace** (the fast software renderer does not draw shadows).
- Scripts: `spotLight -ca 50 -pa 10 -ss 0.8 -sam 24`, `pointLight -ss 0.5`, `areaLight`, `ambientLight`, and `setAttr "spotLightShape1.shadowSoftness" 1`.

## Particle emitters

`nParticle` (Create > nParticles) now makes a real, selectable **emitter** object next to the solver. Click it in the viewport or the Outliner, move it to place the emission, rotate it to aim it (particles leave along its +Y axis; the cone gizmo shows the spread) and key it like any object. Its Attribute Editor tab has Rate, Speed, Spread, Lifespan, Max Particles, Start Frame and Seed. `emitter` adds one to a solver that lost its emitter.

## Time slider keys: copy and paste

Click a key in the Time Slider (Shift adds, drag moves). **Right-click** the slider for **Copy**, **Cut**, **Delete**, **Paste at Current Time** and **Paste Here** (the frame under the cursor), or press Ctrl+C / Ctrl+V while the pointer is over it. Keys go back to the channels they came from; if you select another object first, they go to the same-named channels of that object. In scripts: `copyKey -time "1:24" ball`, `pasteKey -time 60 ball2`, `cutKeys`.

## More Maya 2009 like features

- **Constrain menu** (Animation set): Point, Orient, Scale, Aim and Parent constraints with Maintain Offset and per-target weights (`pointConstraint -mo targetA targetB driven`). A constraint is a node under the driven object; edit its weights in the Attribute Editor or animate them. **Locators** (Create > Locator, `spaceLocator`) are the usual targets.
- **Set Driven Key** (Animate > Set Driven Key..., `setDrivenKeyframe -cd door.rotateY -dv 0 -v 0 knob.translateX`): drive one attribute from another through a curve.
- **Graph Editor** (Window > Graph Editor): curves of the selection, drag keys in time and value, Spline / Linear / Stepped, delete, zoom and pan.
- **Blend shapes** (Deform > Create Blend Shape, `blendShape target1 target2 base`): select same-topology copies, then the base; each target gets a keyable weight.
- **Booleans** (Mesh > Booleans, `polyCBoolOp -op 1|2|3 a b`): union, difference, intersection of closed meshes.
- **Normals** (Mesh > Normals, `polySoftEdge -a 180`): soften or harden edges per object (Smooth Angle in the Attribute Editor).
- **Skin weights:** `skinPercent -tv joint 0.5 mesh.vtx[0:10]` sets joint influences (there is no weight painting brush yet).

## Not implemented yet (compared with Maya 2009)

Toxik and mental ray (out of scope), Python, MEL arrays, fluids, cloth, hair and fur, soft bodies, subdivision surfaces, sculpt and joint cluster flexors, a Hypershade node canvas and a movie writer for playblast. `docs/FINAL_AUDIT.md` lists the status of every lesson of the book and what is missing in each.

## Display modes

Wireframe (4), **Smooth Shade** (5), **Textured** (6) and Stereo Anaglyph (7) are in the Display menu, the viewport Shading menu and the tool box. Textured is shaded display with your scene's lights and color maps, so you can judge lighting and textures while you work; it is not render quality (no shadows or reflections: use the Raytrace renderer in Render View for those). Assigning a color map switches a Shaded viewport to Textured. **Display > Border Edges (show gaps)** draws edges that have a face on one side only in red, which makes holes easy to find.

## Merging, filling and bridging

In the Polygons menu set (Edit Mesh menu, shelf and right-click menu):

- **Merge Vertices** joins selected vertices that are closer than 0.001. **Merge to Center** joins *all* selected vertices (or the ends of selected edges or the corners of selected faces) into one vertex at their center, however far apart: `polyMergeVertex -center`.
- **Merge Edges** (Edge mode): select two border edges, for example the two sides of a seam, and they become one edge; several pairs at once work too, each edge pairing with its nearest partner. `polyMergeEdge`.
- **Fill Hole**: select any edge on the rim of a gap and the whole hole is filled with a polygon, or select nothing to fill every hole. If your edges form an open chain with no complete rim, the chain is closed with one polygon. `polyCloseBorder`.
- **Bridge**: select the border edges of two holes (or the two sides of a gap) with the same number of edges and a strip of quads connects them. `polyBridgeEdge`.

## Texturing objects and faces

Every object has a base material and mapping. Faces can have their own: a group of faces that shares a material and a mapping is a *texture zone*. Zones follow the faces through modeling (extrude, move, delete, smooth...), save with the scene, and export to OBJ (`usemtl` per face) and FBX (per-polygon materials), where they come back as zones on import.

- **Select faces** (F11) and use **Create UVs**: Planar Mapping *Best Plane* fits the texture to the selected faces (the usual choice for a label or a decal), or Planar X/Y/Z, Automatic (box), Cylindrical, Spherical. With nothing selected (Object mode) they apply to the whole object.
- **Create UVs > Assign Color Map...** (also in the face right-click menu) puts an image file, or one of the **built-in patterns** (checker, grid, bricks, stripes, dots, noise, gradient), on the selected faces; each gets its own new material. **Modify > Assign Material** puts an existing material on the selected faces.
- **Create UVs > Edit Texture**: Flip U / V, rotate 90 degrees, scale the tiling, or reset; on the selected faces or the whole object.
- **UV Texture Editor** (Window menu or Create UVs): pick a zone, see the UV layout of the whole object with the zone's faces and your selected faces highlighted, drag in the layout to slide the texture, and edit the zone's mapping, tiling, offset, rotation, material and color map. *Select zone faces* selects a zone's faces; *Clear all zones* returns to the base material.
- Scripts: `assignTexture "cube.f[0:3]" "builtin:bricks"`, `assignMaterial red cube.f[4]`, `polyPlanarProjection -md b cube.f[0:3]` (x, y, z or b), `polyCylindricalProjection`, `polySphericalProjection`, `polyAutoProjection`, `polyEditUV -tu 3 -tv 3 -ou 0.25 cube.f[2]` (or relative `-u -v -su -sv -a -flipU -flipV`), `textureZones cube` to list zones and `textureZones -clear cube`.
- Limits: UV layouts come from projections, so you place and fit a texture per zone rather than moving single UV vertices, and imported UV coordinates are replaced by a projection. A mesh can have up to 255 zones.

## Workspace and transform foundations (Maya 2009 book, Phase 1)

Following *Learning Autodesk Maya Foundation*: the plan is in `docs/PHASES.md`, the audit of this phase in `docs/PHASE1_AUDIT.md`.

- **Projects.** File > Project... creates (`project -new PATH`) or selects a project folder with `scenes/`, `images/`, `sourceimages/` and more. A bare scene name opens from and saves to `scenes/`; image files are stored relative to the project; the Open and Assign Color Map dialogs start in the right folder.
- **Pivots.** Rotation and scale happen around the pivot. **Modify > Center Pivot**, **Edit Pivot** (Insert key, or the Pivot button: the Move handles then move the pivot, not the object) and Reset Pivot; the manipulators sit at the pivot; Display > Transform Display shows pivots and selection handles. `xform -cp`, `xform -piv x y z`, `xform -q -ws -rp`.
- **Snapping** (status line): **Grid Snap** snaps moves, rotation and scale to the steps in Preferences; **Point Snap** snaps a move to the vertex, joint or locator under the cursor. `snapMode -grid 1 -moveStep 0.5`.
- **Display > UI Elements** hides and shows every panel, with Hide All / Show All. The viewport **Show** menu (also in Display) turns object types on and off. `viewport -show lights 0`.
- **Hotbox**: hold Space (a quick tap still toggles the four view). **Window > Settings/Preferences > Preferences** (undo levels, autosave, start shading, time unit, default key tangent, snap steps, manipulator size) and **Hotkey Editor** (bind any key to any command, e.g. `ctrl+shift+K` runs `polyCube`). Saved to `~/.config/aztec2009/prefs.txt`.
- **Create > Polygon Primitives**: *Interactive Creation* (click two points on the grid) and *Options...* (exact sizes and subdivisions).
- **Edit > Duplicate Special** can make **instances** that follow the original's geometry (`instance`, `uninstance`). **Edit > Delete by Type** (history, non-deformer history, static channels, animation channels, constraints), **Delete Unused Nodes**, **Select All by Type**, **Display > Show Last Hidden**.
- **Maya ASCII import:** `file -import lesson.ma` or `file -open lesson.ma` reads transforms, pivots, joints, polygon geometry, lambert/blinn/phong colors with file textures and transform animation curves from a `.ma` file (not `.mb`; not exported). Anything it cannot carry over is listed in the import message.

## Keys

| Key | Action |
| --- | --- |
| Q W E R | Select, Move, Rotate, Scale |
| Alt + left drag | Orbit |
| Middle drag, right drag, wheel | Pan, zoom, zoom |
| F / A | Frame selection / frame all |
| 4 / 5 / 6 / 7 | Wireframe / smooth shaded / textured (lit, with color maps) / stereo anaglyph |
| Space | Single view <-> four view |
| S | Set key |
| Alt + V | Play / pause |
| , and . | Previous / next frame |
| Ctrl + Z / Ctrl + Y | Undo / redo |
| Ctrl + D / Ctrl + G / Del | Duplicate / group / delete |
| Ctrl + N / O / S | New / open / save scene |

## Features

- Dependency graph with a slot-map arena, plugs and connections, dirty-flag propagation and lazy evaluation.
- Node types: Transform, Mesh, Camera, StereoCamera, Material, Light, NucleusSolver.
- Primitives: polyCube, polySphere, polyCylinder, polyCone, polyTorus, polyPlane with positions, normals, UVs and polygon faces.
- Stereo camera rig with interaxial distance, zero parallax plane and a red-cyan anaglyph viewport.
- Nucleus nParticles with sub-stepped gravity, wind drag, plane bounce and particle-particle collision.
- Animation layers (base plus additive, weights 0.0 to 1.0) and a 1 to 120 frame timeline at 24 fps with linear, bezier and step keys.
- Render passes: Beauty, Depth, Normal, Wireframe, written as ASCII, ANSI or PPM text files.
- `aztecsh` command language and `.aztec` ASCII scene files.

## Dependency graph tools (Phase 2)
Window menu: Hypergraph, Hypershade, Connection Editor, Add Attribute, Channel Control, Relationship Editor.
MEL: `shadingNode -asShader blinn -name m`, `shadingNode -asTexture checker -name c`, `connectAttr c.outColor m.color`,
`hyperShade -assign m`, `listConnections`, `addAttr -ln x -at long -min 0 -max 5`, `setAttr -lock 1 "n.attr"`,
`sets -name s a b`. See docs/PHASE2_AUDIT.md.

## Polygon modeling (Phase 3)
`polyOffsetLoop`, `polySlideEdge`, `polyMirrorCut`, `polySelectBorder`, `smoothPreview -l 1|2|3` (keys 1 2 3),
`polyConvertPreview`, `polyOptions -kft 0|1 -dn 1`, `symmetricModelling -s 1 -axis x`. See docs/PHASE3_AUDIT.md.

## UVs and painting (Phase 4)
`polyProjection -type planar -md z`, `polyAutoProjection`, `uvSelect`, `polyEditUV`, `polyLayoutUV -sf`, `polyMapCut`, `polyMapSew`,
`unfold`, `paintAssign -res 512`, `paintOptions`, `paintStroke`, `paintSave`. Windows: UV Texture Editor, 3D Paint Tool.
See docs/PHASE4_AUDIT.md.

## NURBS (Phase 5)
Create > NURBS Primitives, CV/EP Curve Tool; menu set Surfaces (F4). MEL: `sphere`, `cylinder`, `torus`, `circle`,
`curve -d 3 -ep x y z ...`, `revolve`, `loft`, `extrude`, `attachCurve`, `duplicateCurve -u T`, `insertKnotSurface`,
`detachSurface`, `attachSurface`, `moveSeam`, `nurbsToPoly`, select CVs with `surf.cv[i][j]`. See docs/PHASE5_AUDIT.md.

## NURBS trimming and texture networks (Phase 6)
`projectCurve`, `trim`, `untrim`, `planarSurface`, `rebuildSurface`; `shadingNode -asTexture ramp|noise|fractal|cloth`,
connect textures into `colorGain` or ramp entries (`ramp.eCol1`), `duplicateShadingNetwork`, `convertToFileTexture`,
`placeTexture`, `exportShaders` / `importShaders`. See docs/PHASE6_AUDIT.md.

## Animation (Phase 7)
`keyTangent -itt flat -ott flat`, `setInfinity -pri cycle -poi cycleWithOffset`, `scaleKey`, `moveKey`, `setKeyframe -insert`,
`bakeResults -sac 1`, `pathAnimation -follow 1 -bank 1`, `sound -file x.wav`. Windows: Dope Sheet, Graph Editor menus
Tangents and Curves. See docs/PHASE7_AUDIT.md.

## Character setup (Phase 8)
Skeleton: `orientJoint`, `mirrorJoint -mirrorBehavior`, `insertJoint`, `reroot`, `transformLimits`. Skin: `bindSkin -rigid`,
`skinPaint`, `skinMirror`, `goToBindPose`, `lattice`. IK: `ikHandle -sol ikRPsolver|ikSCsolver|ikSplineSolver`, `poleVectorConstraint`.
Characters: `character`, `setCurrentCharacter`; references: `file -reference -namespace NS`. See docs/PHASE8_AUDIT.md.

## Deformers and Paint Effects (Phase 9)
* Deformers: `wire`, `cluster` (mesh points or curve CVs), `softMod`, `jiggle`, `pointOnCurve`, `nonLinear -type wave`; membership with `deformerMembership`, weights with `deformerWeights`, order with `reorderDeformer` / `listDeformers`. Menu: Deform > Create Deformers / Edit Deformers.
* Paint Effects: `paintBrush -preset gloppy|oak|pine|bush|astroturf|grassBermuda|daisy|vine|jetTrail|airbrush`, `makePaintable`, `pfxStroke`, `autoPaint`, `shareBrush`, `brushFromStroke`, `blendBrush`, `brushPreset`, `strokeToPoly`, `paintCanvas`. Menu: Paint Effects > Visor..., Paint Effects Tool. See docs/PHASE9_AUDIT.md.

## Lighting and rendering (Phase 10)
Render Settings: `renderSettings -renderer software|hardware|vector -w 640 -h 480 -quality production -s 1 -e 50 -prefix "<Scene>/<RenderLayer>" -mb 1 -blurType 3d`. Rendering: `renderImage [-layer L] [-pass diffuse|specular|shadow|reflection|depth|normal|mask|motion] [-region "x0 y0 x1 y1"]`, `batchRender`, `aztec2009 --render -s 1 -e 50 scene.aztec`, `fcheck`. Layers and lights: `renderLayer`, `lightLink`, `renderStats`, light channels for depth map shadow blur, glow, lens flare and fog. Menu: Render > Render Settings..., Batch Render, Render Layers..., FCheck. See docs/PHASE10_AUDIT.md.

## Dynamics (Phase 11)
Rigid bodies: `rigidBody -active|-passive`, `setRigidKey`, `rigidConstraint -type nail|pin|hinge|spring|barrier`, `rigidSolver -cache 1`, `dynInitialState`. Fields: `gravity`, `air`, `drag`, `newton`, `radial`, `turbulence`, `uniform`, `vortex`, `connectDynamic`. Particles: `particle`, `emitter -type omni|directional|volume|surface|curve`, `emitFromObject`, `makeCollide`, `particleEvent`, `particleRamp`, `particleEffect -type smoke`; blobby surfaces render in software, sprites in the hardware renderer. Menu set: Dynamics (F5). See docs/PHASE11_AUDIT.md.

## Trax, cameras and paths (Phase 12)
Clips: `clip -create walk`, `trimClip`, `splitClip`, `blendClips`, `mergeClips`, `clipTimeWarp`, `clipExport` / `clipImport` / `clipInsert`, `redirect`; Trax Editor in the Window menu. Animation layers: `animLayer -addSelected / -zeroKey / -merge / -keyWeight`. Cameras: `cameraAim [-up]`, `filmFit` channel. Paths: `makeLive`, `curve -live`, `pathMarker`, `normalConstraint`. See docs/PHASE12_AUDIT.md.

## MEL and customization (Phase 13)
`expression -n NAME -s "eyeball.sy = lookAt.blink"` (Expression Editor), scripts in `scripts/` folders load by name (`scriptPath`, `userSetup.mel`), interface from scripts (`window`, `columnLayout`, `intSliderGrp`, `button -command`, `showWindow`, ...), `shelfButton`, `markingMenu`, the new Script Editor. See docs/PHASE13_AUDIT.md.

## Command line

    aztec2009                  windowed workspace
    aztec2009 --tui            full screen terminal workspace
    aztec2009 --snapshot a.png [WxH]   render the perspective view to a PNG
    aztec2009 -c script.mel    run a script and exit
    aztec2009 -e "commands"    run commands and exit
    aztec2009 -f scene.aztec   open a scene, then start the workspace
    aztec2009 -e 'file -import "a.fbx"; file -export "b.obj"'   convert between formats
    aztec2009 --shell          plain line based aztecsh

## Terminal workspace keys (--tui)

- Esc switches between the shell and the viewport. Tab cycles panels. F1 shows help, F2 enlarges the shell.
- F10 or Alt+F/E/C/D/M/R open the menus. Ctrl+Q quits.
- Viewport: arrows orbit, Shift+arrows pan, + and - zoom, 1/2/3 shaded/wireframe/anaglyph, f frame all, k set key.
- Mouse: Alt+Left orbit, Alt+Middle pan, Alt+Right zoom, wheel zoom, click to select, click the timeline to scrub.
- Space plays and pauses. Comma and period step frames.

## Example session

    polySphere -r 2 -name myBall;
    setAttr "myBall.translateY" 3;
    setKeyframe -t 1 "myBall.translateY";
    setAttr "myBall.translateY" 0;
    setKeyframe -t 24 "myBall.translateY";
    currentTime 12;
    render -mode anaglyph -out "frame_12.txt";
    file -save "scene.aztec";

Run `examples/demo.mel` for a larger scene, and type `help` for every command.

## Layout

    src/math     Vec2/3/4, Mat4, Quat
    src/dag      scene graph, plugs, node data, .aztec serialization
    src/geo      meshes and primitives
    src/nucleus  particle solver
    src/anim     curves, layers, timeline
    src/mel      aztecsh lexer, parser, structured script engine and commands
    src/io       OBJ and FBX import / export
    src/render   rasterizer, stereo, render passes
    src/ui       terminal, panels, menus, main loop
