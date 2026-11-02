# Phase 12 audit: non-linear animation (Trax), cameras and motion paths (Lessons 23 and 25)

## Coverage
| Lesson topic | Status |
|---|---|
| Create Animation Clip from a character set (name, time range, leave keys in timeline, time warp) | `clip -create NAME [-character C] [-t "a:b"] [-leaveKeys] [-timeWarp]`; the keys of the character's channels move into a clip source and a clip (base layer keys only; sub-characters are not supported) |
| Clip sources, Visor > Character Clips, export and import of clips, Library > Insert Clip | `clipLibrary`, `clipExport` (text `.aztclip` in the project's clips folder), `clipImport [-insert]`, `clipInsert`; the Trax Editor has Library, Import and Export Source |
| Relative and absolute clips (Channel Offsets: All Relative) | `clip -edit NAME -relative 0|1` (all channels together; **no per-channel offsets**). A relative clip starts from the value the clip before it ended with |
| Trim before / after, split, move clips | `trimClip`, `splitClip`, `clip -edit -start`, drag in the Trax Editor; cycles (with offset) and scale as clip attributes |
| Blend (initial weight curve: linear, ease in, ease out, ease in out) | `blendClips`: the first clip fades into the second across their overlap, which follows when you move a clip. Blends join clips of one track |
| Merge clips | `mergeClips -name N clips`: one clip with a key on every frame (bake), replacing the clips |
| Time warp, edit with the Graph Editor | `clipTimeWarp -create / -key "clipFrame sourceFrame" / -remove`; the warp is a curve you key by command (it is **not shown in the Graph Editor**) |
| Activate Keys | `clip -activate NAME [-keep]` puts the keys back in the timeline |
| Tracks, clip weights | Several tracks per character: an upper track overrides the lower one by its weight; before the first clip and after the last one the first / last pose holds |
| Redirect (Character > Redirect: translation only) | `redirect [-translation] [-rotation] node`: an `OffsetTranslateControl1` node adds its translation to the animation (rotation: about the position the animation started from). Animated or clip driven channels only |
| Sound in Trax | The `sound` nodes of earlier phases appear on the Trax timeline (**not draggable, no sound output**) |
| Animation layers: add selected objects, zero keys, merge layers, weight keys, tangents | `animLayer -addSelected / -zeroKey / -merge / -keyWeight / -members`, the Layer Editor has the buttons. Layer weights can be keyed. Membership is recorded, keys go to the active layer as before |
| Camera and Aim, Camera Aim and Up, key camera and aim | `cameraAim [-up]`: the camera is aim constrained to its `_aim` node, both keyable |
| Fit Resolution Gate (Fill, Horizontal, Vertical, Overscan) | `filmFit` channel (the field of view reads horizontally or vertically; default Vertical like before) and the Attribute Editor |
| Playblast with sound | Playblast writes frames only (no movie, no audio) |
| Make Live, Make Not Live, drawing a curve on a live surface | `makeLive`; `curve -live ...` drops the points onto the surface (down, else closest point), and the curve tool clicks land on it. The live surface shows as a green wireframe. **Polygon surfaces only** (no projection onto NURBS) |
| Motion path: attach, follow axes, markers, timing keys, manipulator | Done in Phase 7; **new**: path markers show their frame numbers in the views and `pathMarker -q / -f -u / -moveTo / -remove`. There is no marker manipulator: edit the U Value keys or use `pathMarker` |
| Normal constraint (aim, up, world up object) | `normalConstraint`: aim vector follows the surface normal at the closest point; world up: scene, object (the up object's direction), vector, none. The aim constraint got the same world up object option |
| Remove from character set, pivot, parent constraint with maintain offset | Existing |

## Effects on the program
* New data: `scene.trax` (sources, clips, blends), `scene.redirects`, `scene.live`; animation layers keep their member channels and a keyed weight. All are saved (`clipSource`, `clipChannel`, `clip`, `clipBlend`, `redirect`, `makeLive` lines; `-members` and `-wkeys` on `animLayer`).
* Clips are evaluated after the scene's keys in `evaluate_anim`, then redirects.
* `ConstraintKind::Normal`; constraints gained `worldUpType`. Cameras gained `filmFit`.
* New window: Trax Editor (Window menu, Character menu). New menu items: Create > Cameras > Camera and Aim, Constrain > Normal, Modify > Make Live, Character > Redirect.
* New commands: clip, trimClip, splitClip, blendClips, mergeClips, clipTimeWarp, clipExport, clipImport, clipInsert, clipLibrary, redirect, cameraAim, normalConstraint, makeLive, pathMarker; extra animLayer flags; `curve -live`.

## Tests
`anim::trax` unit tests and the `phase12_*` tests: the Trax lesson (create, trim, relative, blend, time warp, split, merge, redirect, activate keys, save and load), clip files and the library, animation layer zero keys / weight keys / merge, camera and aim, film fit, normal constraint with each world up type, live surfaces, path markers. The Trax Editor window and the new menus are compiled and smoke started, not clicked through.
