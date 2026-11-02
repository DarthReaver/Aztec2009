# Phase 11 audit: dynamics, rigid bodies, particles, fields (Lessons 24 and 26)

## Coverage
| Lesson topic | Status |
|---|---|
| Active and passive rigid bodies (Create Active / Passive Rigid Body) | Done: `rigidBody`, a `rigidBody` node under the object. An active body reacts to fields and collisions, a passive one follows its keys and pushes active bodies |
| Rigid body attributes: mass, bounciness, damping, static / dynamic friction, center of mass, initial velocity and spin, stand-in shape | Done as channels of the rigidBody node. The stand-in shapes are box, sphere (both from the object's bounds) and mesh (passive bodies only). **Active bodies are never concave**: an active body with the mesh shape is a box. Collisions use the *stand-in*, so a tweaked stand-in mesh works like in the lesson; deformed geometry is not followed |
| Gravity and other fields; Affect Selected Object(s) | Gravity, air, drag, newton, radial, turbulence, uniform, vortex with magnitude, attenuation, direction, max distance and volume shapes (cube, sphere, cylinder, cone sized by the field's scale); `connectDynamic` |
| Playback and simulation; rewinding | The solver runs from the start frame to the current frame. Scrubbing and rewinding are safe (it simulates again); changing any dynamics setting starts it again. Playback-speed "play every frame" is not needed |
| Set Active Key / Set Passive Key (a body that turns active at a frame) | Done: `setRigidKey`; the `active` channel keys with a step. A body that turns active keeps the velocity of its keyed motion |
| Initial State (Set for Selected / All Dynamic) | `dynInitialState`; stored with the scene |
| Simulation cache | `rigidSolver -cache 1` keeps the rigid body states of every simulated frame, attribute changes are ignored until `-deleteCache`; the cache is not saved with the scene. Particles are not cached (they replay from the start) |
| Rigid body constraints | Nail, pin, hinge (axis = the constraint's Z), spring, barrier. Simplified: point joints are solved with impulses, a hinge keeps its axis aligned, no limits |
| Contact quality | Spheres, boxes and static meshes with friction and restitution, stacking and sliding are tested. **Edge against edge contacts between two boxes are missed**, there are no continuous collisions (very fast thin objects can tunnel), no sleeping beyond a velocity clamp |
| Particles: emitters (omni, directional, volume, surface, curve), rate, speed, spread, inherit velocity | Done: `emitter -type`, `emitFromObject`. Emitters fill particle objects (`particle`), several emitters can feed one |
| Lifespan (forever, constant, random), conserve, damping, start frame | Channels of the particle shape |
| Initial state of particles (start frame -30, Set for Selected) | Done |
| Per particle ramps for scale, color and opacity (arrayMapper, Create Ramp) | `particleRamp`: stops over the normalized age plus a maximum value. There are no per particle expressions or arbitrary per particle attributes |
| Smoke effect preset | `particleEffect -type smoke|fire`: curve or surface emitter, turbulence field, lifespan, sprite render type and ramps. The sprite image of Maya is replaced by a soft puff |
| Render types: points, spheres, blobby surface, sprites, streaks | Done. Spheres and blobby surfaces are rendered by the software renderer (blobby = merging metaballs, `radius` and `threshold` like the lesson); points, sprites and streaks only render in the hardware renderer, as in the lesson |
| Particles and materials (Hypershade, 3D textures on blobbies) | `assignMaterial MATERIAL particle` colors spheres and blobby surfaces (flat color, no 3D texture lookup) |
| Make Collide, resilience and friction (geoConnector) | `makeCollide`; particles collide with the triangles of the geometry, including moving geometry |
| Particle collision events (emit, split, original dies) | `particleEvent` and the Particle Collision Event Editor window. An event triggers once per particle |
| Software and hardware rendering of particles for compositing; geometry mask, hardware motion blur | Done with the Phase 10 pipeline: `renderSettings -renderer hardware` draws sprites, points and streaks with depth from the geometry; `geometryMask` and `hardwareExposures` channels of `defaultRenderGlobals` |
| Particle shapes, Nucleus | The older `nParticle` / `nucleus` solver is unchanged |

## Not done
* No soft bodies, springs on particles, goal weights, `particleShape` expressions, or cloth / fluids. No emitters by texture. Fields have no per vertex mode. No interpenetration warnings.
* The viewports draw particles as small spheres whatever the render type.

## Effects on the program
* New nodes: `rigidBody`, `rigidSolver`, `field`, `rigidConstraint`, `particleEmitter`, `particle` (their data is saved with `-dyn` flags). `Scene::update_dynamics` runs after the old solver update in `evaluate_frame`.
* `nucleus::rigid` (bodies, contacts, joints), `nucleus::fields` (fields, particle stepping), `dag::dynamics` (scene integration).
* Particles can carry materials, ramps and a render type; the raytracer gained blobby surfaces.
* New menu set: Dynamics (F5) with Fields, Soft/Rigid Bodies, Particles, Effects and Solvers, plus the Particle Collision Event Editor.
* New commands: rigidBody, setRigidKey, rigidSolver, dynInitialState, gravity (and the other fields), connectDynamic, rigidConstraint, particle, emitFromObject, makeCollide, particleEvent, particleRamp, particleEffect, dynInfo. `emitter` with `-type` makes the new emitters.

## Tests
`nucleus::rigid` unit tests (resting, bouncing, friction on a slope, stacking, mesh floors, joints, barriers) and the `phase11_*` tests: the rigid body lesson steps, active/passive keys, cache and initial state, passive animation hitting active bodies, constraints, every field, emitters, collisions, events, ramps, curve and surface emission, smoke, blobby and hardware rendering, saving and loading. The new menus, window and gizmos are compiled and smoke started, not clicked through.
