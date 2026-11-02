# Phase 7 audit: animation fundamentals (Lessons 14, 22 and 25; Lesson 4 time slider basics)

## Coverage
| Lesson topic | Status |
|---|---|
| Tangent types: clamped, spline, linear, flat, plateau, fixed, stepped (in and out separately) | Done: `keyTangent -itt -ott -t`, Graph Editor > Tangents; default for new keys with `keyTangent -g` and Preferences > Animation (Default in / out tangent) |
| Pre / Post Infinity: constant, linear, cycle, cycle with offset, oscillate; drawn thinner outside the keys | Done: `setInfinity`, Graph Editor > Curves |
| Bake Channel (start/end, keep unbaked, sparse) | `bakeResults [-t range] [-sac 1] node.attr`: bakes any channel including infinity and driven ones (releases the connection); sparse drops keys on straight lines. "Keep unbaked keys" is not an option (baking replaces the curve) |
| Insert Keys tool, Key Selected, Auto Key with flat tangents | `setKeyframe -insert` (key lands on the curve without changing it); existing Key Selected and Auto Key use the default tangents |
| Copy / paste / merge keys, move and scale keys, timing fixes | Existing copy / cut / paste, plus `moveKey` and `scaleKey`, and the Dope Sheet |
| Dope Sheet | Done: Window > Animation Editors > Dope Sheet (rows per channel, select, drag to retime, copy, cut, paste, move by one frame, scale, delete, click the ruler to set the time) |
| Attach to Motion Path: curve, uValue keyed over the time range, follow with front / up axis, inverse, bank, twists | Done: `pathAnimation`, Animate > Motion Paths; the motion path node has editable channels (uValue, fractionMode, follow, frontAxis, upAxis, bank, bankScale, bankLimit, front/up/side twist) |
| Sound on the time slider | Import (`sound -file x.wav -offset N`) with a waveform drawn on the time slider; **no audio output** and PCM / float WAV only |
| Playblast | Existing |
| Weighted tangents, breakdowns, ghosting, Flow Path Object, Set Motion Path Key, path markers | **Not done** |
| Character sets, Trax clips (walk cycle lessons) | Phases 8 and 12 |

## Effects on the program
* Keys have separate in / out tangent types; older files and scripts that only know linear / bezier / step load unchanged (bezier = clamped, as before). Saved as `-it` / `-ot` on `setKeyframe` lines only when they differ from the default for the interpolation, and as `setInfinity` lines.
* New node types `motionPath` (child of the moving object) and `sound`; `delete motionPath1` now removes only that node (the same applies to constraints; previously deleting a constraint node removed its object).
* `bakeResults` disconnects driven channels it bakes.

## Verification
* 162 tests pass, including `phase7_tangents_infinity_and_editing` (tangent types change the curve as expected, step is out-only, cycle with offset / oscillate / linear / constant values, scale, move, insert, sparse bake, default tangents, save and load) and `phase7_motion_path_and_sound` (position and facing along a two-leg path, keyed uValue, bake and release, errors, save and load, WAV waveform).
* Not verified: the Dope Sheet, the Graph Editor Tangents / Curves menus, the motion path menu and the time slider waveform were compiled, not clicked.
