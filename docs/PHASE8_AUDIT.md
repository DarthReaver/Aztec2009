# Phase 8 audit: character setup (Lessons 9 to 13 and 17)

## Coverage
| Lesson topic | Status |
|---|---|
| Joint Tool, naming, joint size, X-ray joints | Joint tool and rename existed; `jointDisplayScale` and Skeleton > Joint Size added; X-ray joints already drawn on top |
| Connect Joint, Disconnect Joint, Insert Joint, Remove Joint, Reroot Skeleton | Done: `connectJoint`, `disconnectJoint`, `insertJoint`, `removeJoint`, `reroot`; joints keep their place |
| Mirror Joint: YZ / XY / XZ planes, Behavior or Orientation, search and replace names | Done: `mirrorJoint -mirrorYZ -mirrorBehavior -searchReplace l_ r_` (Behavior reverses every axis so the same rotation values move both sides symmetrically) |
| Orient Joint (primary / secondary axis, scene up), joint orient channels, display of local rotation axes | Done: `orientJoint -oj xyz -sao yup`; joints have `jointOrient`; Show > Local Rotation Axes of the selected joints |
| Move a joint without its children (Insert key) | Existing pivot edit mode |
| Freeze Transformations on a skeleton | Existing |
| Parent binding | Existing (`parent`) |
| Rigid bind, smooth bind (influences, dropoff), Edit Membership | `bindSkin -rigid / -smooth -maxInfluences -dropoff`; membership by painting weights (there is no Edit Membership tool) |
| Add / Remove Influence, Go to Bind Pose, Move Skinned Joints, preferred angle, limits | Done: `skinInfluence`, `goToBindPose`, `skinMoveJoint`, `setPreferredAngle` / `assumePreferredAngle`, `transformLimits` (limits also bind animation, IK and constraints) |
| Paint Skin Weights Tool, Smooth, Mirror, Copy, Prune, Normalize, Reset, Export / Import weight maps, max influences | Done: window and tool (replace / add / scale / smooth, value, radius, opacity, weight display in blue to red) plus `skinPaint`, `skinSmooth`, `skinMirror`, `skinCopy`, `skinPrune`, `skinNormalize`, `skinReset`, `skinMaxInfluences`, `skinExport` / `skinImport` (text format, not image maps) |
| Lattice deformer, lattice binding with local influence, driven lattice points | Lattice done (`lattice -dv`, points as `lat.pt[s][t][u]`, channels `pt_i_j_k_x/y/z` that can be keyed or driven), rigid bind of the lattice to joints; **Local Influence S/T/U is not available** (the deformation is a global Bernstein FFD) |
| Flexors | Lattice flexor added later (see docs/BOOK_WALKTHROUGH.md); sculpt and joint cluster flexors are not available |
| Blend shapes (several targets, weights, keys) | Existing (`blendShape`) |
| IK: rotate plane solver, single chain solver, spline IK, pole vector, twist, IK/FK blend | Done: `ikHandle -sol ikRPsolver / ikSCsolver / ikSplineSolver -curve`, `poleVectorConstraint`, handle channels `twist`, `ikBlend`, `offset`, `roll`. Two bone chains are solved exactly; longer rotate plane chains and the single chain solver use cyclic coordinate descent |
| Set Driven Key, constraints, Add Attribute, Connection Editor, lock channels | Existing (Phases 1 to 2) |
| Character sets (Create Character Set, current character, Set Key keys the whole set) | Done: `character`, `setCurrentCharacter`, Character menu; Set Key (S) and `setKeyframe` with no arguments key the current character |
| References: create, unload, load, remove, namespaces | Done in a simplified form: `file -reference -namespace NS`, `-unloadReference`, `-loadReference`, `-removeReference`, `referenceQuery`; referenced nodes are named `NS_name` and saved with the scene (plus a link line) |
| Reference edits (changes to referenced nodes survive unload), proxies | **Not done**: unloading discards the referenced nodes and anything changed on them, including keys made on them |
| Rename multiple objects, Distance Tool, inherits transform (Lesson 17) | Done: `renameSelection BASE`, `distanceDimension`, `inheritsTransform` channel |
| Non-linear deformers, reactive driven keys | Existing (nonLinear, setDrivenKeyframe) |

## Effects on the program
* New channels: joints get `jointOrient*`, `preferredAngle*`, `inheritsTransform`; IK handles `twist`, `ikBlend`, `offset`, `roll`; transforms `inheritsTransform`. Local matrices of joints now include the joint orient (zero in old scenes, so nothing moves).
* `set_eval` clamps to a channel's limits. Limits are saved as `attrRange` lines.
* New node types `lattice` (with a `latticeDeform` history node), `character`, `distanceDimension`; skin data saves bind options (`-skinopts`); IK handles save solver, curve and pole.
* The scene loader can now load into an existing scene (used for references); loading a file no longer relies on starting from an empty scene.
* `bindSkin` accepts a selected lattice; deleting a lattice's mesh binding is by `delete` of the history.

## Verification
* 167 tests pass, including `phase8_skeleton_tools` (orient keeps joints in place and aims x at the child, behavior mirror gives symmetric motion for equal rotations, orientation mirror keeps axes, insert / remove / reroot / disconnect, limits on manual, animated values, preferred angle, save and load), `phase8_skinning_tools` (rigid and smooth bind, bind pose, add / remove influence, painting with normalization, smooth, prune, max influences, reset, mirror, copy, export / import, move skinned joint), `phase8_ik_solvers` (exact two bone solution with pole vector, twist, reach limit, ikBlend, single chain, spline following a curve, save and load), `phase8_lattice_deformer_and_binding` and `phase8_characters_references_and_lesson17_tools`.
* Not verified: the Skeleton, Skin, Deform and Character menus, the Paint Skin Weights window and tool, and dragging lattice points in the viewport were compiled, not clicked. There is no File menu item for references yet (use the command).
