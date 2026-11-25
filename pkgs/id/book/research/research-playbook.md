# Six apps a participant can finish, replay or resume

_Research snapshot of November 23, 2026. Code and issue observations refer to the cited revisions and collection times._

**Proposed suite.** The accompanying [app-vectors.json](evidence/app-vectors.json) contains six additional scenarios and 33 expected state transitions. Each names its parent ID in the staged 13-case [app-fixtures.json](evidence/app-fixtures.json). The two sets have different purposes. The earlier cases preserve regression and pending merge-policy decisions. These transcripts specify small levels or editorial tasks from entry through an observable result. The answers were authored by hand. JSON structure, correspondence of the six IDs, and all 33 expected records have been checked independently against the level rules. No guest, renderer or device ran them.

## Shipping path already in the tree

Start from the source and package layout of tic-tac-toe, the eight-entrypoint Roc record in Territory and the tiny RocGrid guest. Territory separates key decoding, world state, screen, published records, presence subscription and snapshot/restore. RocGrid has bounded row-major access and orthogonal neighbors. Cellular automata require diagonals as well. The existing hardware shell answers some presented forms. No proposed action label in this chapter creates a network frame or documents an existing command-line flag. [Tic-tac-toe package][ttt] · [Territory][territory] · [RocGrid guest][gridexample] · [Grid operations][grid] · [Hardware client][hardware] · [ABI][abi]

For each guest keep the level configuration small, immutable and versioned. A participant starts or reopens a world, sees their current objective, chooses an available action, reads its result, and can inspect or resume the same state. Keep initialization, logical transition, record projection and text presentation separate inside the guest. Use existing `update`, `view`, `records`, `wants`, `snapshot` and `restore`. Hosts admit authorized events through the ordinary world entry point. Replaying the same accepted journal cursor twice cannot produce two domain transitions. A new command with similar text is a separate attempt and must face the current game rules. Semantic target IDs must be resolved against the view the user saw. For each adapter compare the full records after every step, including refusals. The existing native/terminal live test and fixture gate in `id world dev` provide entry points for those checks. [Native/terminal live test][live] · [Developer loop][dev]

### Colony · haul supplies

**Goal.** Bring two ore units to a stockpile. Inspect a tile, choose Mine on a neighboring ore, carry one unit at a time, walk and deposit on the stockpile. A worker may walk onto the stockpile. A mine operation removes the adjacent unit without moving the worker. Only empty or stockpile terrain is traversable in this level. Announce progress as “1 of 2 delivered.” When the goal is reached, expose the completed record and allow a new independently initialized level. A level can add teams and jobs later without changing the units or task in this scenario.

**Transcript.** Board 4×2. Stockpile (0,1), ore (1,0) and (2,1), worker (1,1), empty pack and store. Compare the worker position plus the three locations of ore after every action. Coordinates in the JSON are zero-based.

| Input                     | Worker | Ore remaining | Pack | Store |
| ------------------------- | ------ | ------------- | ---: | ----: |
| Mine east                 | (1,1)  | (1,0)         |    1 |     0 |
| Mine north with full pack | (1,1)  | (1,0)         |    1 |     0 |
| Walk west                 | (0,1)  | (1,0)         |    1 |     0 |
| Deposit                   | (0,1)  | (1,0)         |    0 |     1 |
| Walk east                 | (1,1)  | (1,0)         |    0 |     1 |
| Mine north                | (1,1)  | empty         |    1 |     1 |
| Walk west                 | (0,1)  | empty         |    1 |     1 |
| Deposit                   | (0,1)  | empty         |    0 |     2 |

Every row conserves two units. Rejecting the second action cannot deplete the northern vein. Reuse Territory's in-bounds movement and RocGrid's cell access. Keep inventory numeric and separate from cell glyphs so placing the worker over the stockpile cannot remove it from the level.

### Orbit Arcade · clear one wave

**Goal.** Steer a cannon by at most one lane and fire at a formation before it reaches the cannon row. In this level each seat may queue a steer/fire pair for the next logical tick. A second distinct submission for an occupied seat/tick is refused in this rules version. Clamp steering at the board edges. At the tick, apply steering, spawn one shot for a requested fire, move all shots north one square and discard those outside the board. Every fourth tick the formation moves horizontally. If the move would put any remaining target out of bounds, the whole formation descends one row and reverses direction instead. Resolve collisions at the resulting positions. Resolve competing shots in stable shot-ID order and award each defeated target once. Zero remaining targets clears the wave. A surviving target on the cannon row ends it. A new wave starts explicitly from a new level seed.

**Transcript.** Start at the end of tick 3 on a 4×4 board. Cannon (2,3), one target T at (3,0), formation moving east, score 0. Queue right and fire for tick 4. A repeated delivery of that queued event leaves exactly one order. Tick 4 moves the cannon to (3,3), the shot to (3,2), and T to (3,1) facing west. Rendering 120 frames changes none of these fields. Tick 5 takes the shot to (3,1), clears it and T, leaves the cannon at (3,3) and awards 10 points. This adds wall descent and simultaneous steering/firing to the original stationary-target case. Logical ticks can be driven manually when a timer effect is refused. Only admitted ticks advance the simulation. A real-time presentation can interpolate positions locally. [Existing time-capability contract][abi]

### Burrow · dig an escape route

**Goal.** Clear three soil cells and reach the exit. Dig clears adjacent soil and enters that cell. A rock or edge blocks excavation. Walk enters an already open cell; the unlocked exit accepts entry once enough soil has been cleared. Expose rock, soil, traversable cells, exit and remaining quota as semantic descriptions. The entry, quota and exit make this a solvable puzzle without implementing enemy pathfinding. If hazards are introduced in later levels, pin their turn order and test displaced or contested cells separately.

**Transcript.** Board 4×3. Digger (0,1). Soil at (1,1), (2,1), (2,2). Rock (3,1). Exit (3,2). Dig east enters (1,1) and clears one; dig east enters (2,1) and clears two. A third east dig hits the rock and leaves the exact same board. Dig south enters (2,2) and clears the third soil. Walk east reaches (3,2), with no soil, rock still at (3,1) and phase escaped. Compare the immutable terrain, remaining soil, digger and count. Never store the current glyph as the only copy of the terrain beneath the actor.

### Cells · run and recognize a pattern

**Goal.** Seed or load a bounded board. Inspect a cell and its neighbors, edit the seed before starting, step, pause and compare generations. The level challenges the participant to predict where a glider will be after four steps. Use B3/S23, a dead exterior, eight neighbors, and a single read-only previous generation for every update. A headless Bevy observer rebuilds a projection at the checkpoint. It cannot add a simulation tick when reconnecting. Reuse RocGrid for storage and implement a bounded Moore-neighborhood read. Its built-in four-neighbor traversal alone yields a different automaton. [RocGrid][grid] · [ECS design issue](https://github.com/developing-today/code/issues/5781)

|  Generation | Live cells in row-major order     |
| ----------: | --------------------------------- |
|           0 | (1,0), (2,1), (0,2), (1,2), (2,2) |
|           1 | (0,1), (2,1), (1,2), (2,2), (1,3) |
|           2 | (2,1), (0,2), (2,2), (1,3), (2,3) |
| Reopen at 2 | (2,1), (0,2), (2,2), (1,3), (2,3) |
|           3 | (1,1), (2,2), (3,2), (1,3), (2,3) |
|           4 | (2,1), (3,2), (1,3), (2,3), (3,3) |

The board is 5×5. Generation four is the initial set translated one cell east and one cell south. Comparing only live population would miss all the movement. The checkpoint action must preserve both records and generation number.

### Sketch · edit an ink layer

**Goal.** Make an identifiable drawing, cancel an unfinished path, inspect an existing stroke, undo one's own mark and recover it as a fresh attributed object. Polyline mode has one input contract across shells: choose ordered vertices in document units, then explicitly confirm. The separate freehand tool can consume actual sampled points from a pointer host. A terminal point editor may implement polyline mode fully. It should not advertise arbitrary analog pressure or claim its vertices reproduce a stylus trace.

**Transcript.** Start on an empty 5×4 ink layer. Actor A selects (1,1), adds vertices (3,1) and (3,2). Both draft gestures leave the published stroke list and tombstones empty. Confirm creates stroke A.1 with exactly those three ordered vertices. Undo hides A.1 and adds its tombstone. Copy-own-stroke creates A.2 with the same points and provenance A.1. A.2 is visible and A.1 remains tombstoned. Delayed replay of the original undo leaves A.2 visible. This proposal gives Redo an explicit new ID and operation. In this level, A retains read access to the tombstoned original and still holds write and own-undo grants. It requires read access to the original content and current write permission; the control must be unavailable if either has gone away. A later retention or erasure policy may make copying unavailable. The original offline two-author case still determines concurrent undo behavior for the first version. [Existing sketch and merge fixtures](evidence/app-fixtures.json)

### Mind Map · reorganize an outline without breaking references

**Goal.** Add and label topics, relate two topics, move a branch and inspect the resulting outline or spatial view. Stable IDs drive tree edges and links. Two sibling labels can legitimately match. Show the path or another authorized disambiguator in pickers. Move-under, rename, layout move and add-related-link are distinct operations. An attempted cycle in a single admitted history returns an explicit refusal. The application's concurrent-crossing-move rule remains with the content-class design and its existing fixture.

**Transcript.** Root R contains North (P) and South (Q). P contains Archive (U), Q contains Archive (V). Link U to V. Reparent U beneath Q. Q now has two different children named Archive, and the link still points from U to V. Rename U to Inbox. The other Archive is still V. Attempting to move Q beneath U would create a cycle, so refuse and preserve root R, Q's parent R, both children beneath Q and the U→V link. Original node coordinates stay unchanged in every step. Confirm these values on the outline and spatial entry points. The complete name, parent, position and link record for every operation is in the vector file.

## Shell substitutions

The following are candidate affordances. The bounded hardware shell available in source folds a journal and answers supported forms. Each hardware entry here either names an optional form the world would have to provide or records why the task requires another shell. Availability in these cells is a design target. [Hardware contract][hardware] · [Native widget presenter][widget]

| App      | Scripted CLI and keyboard TUI                                                                                    | Web and Linux GUI                                                                                        | Android                                                                                                                      | Confirmation-only hardware                                                                                                                                                      |
| -------- | ---------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Colony   | CLI names action and direction; TUI selects tile, then Mine/Walk/Deposit.                                        | Select the same tile by pointer or keyboard and expose pack/store counts to accessibility APIs.          | Target list and touch grid both expose tile ID, direction and capacity.                                                      | A world-provided “Deposit one ore” token-bound form can authorize that single action at the stockpile. Full spatial loop requires a richer profile.                             |
| Orbit    | CLI queues a tick-stamped input; TUI arrow and fire controls form one tick order. Manual step remains available. | Map keyboard or labelled steer/fire buttons to the same next-tick order. Render interpolation is local.  | Persistent large lane and fire controls and a turn-based order composer allow the same action without simultaneous gestures. | Status/head available. The hardware spike has no realtime arcade input. An optional turn form could submit a prepared order once its timing and rejection behavior are defined. |
| Burrow   | CLI selects Dig or Walk and a direction; TUI offers labeled adjacent terrain.                                    | Keyboard, a target list and clickable adjacent cells reach the same adjacent target.                     | Rotate or resize while digging without changing the selected world cell. Announce “blocked rock.”                            | A short choice form may expose a single legal dig or walking direction. Paired display controls remain conditional on actual form and hardware support.                         |
| Cells    | CLI steps or lists live coordinates; TUI lets the user inspect the same cells.                                   | Step button and keyboard activation are equivalent. Show generation, population and coordinates.         | Provide explicit Step, focusable cell inspection and committed generation announcements.                                     | A token-bound Step form can advance once. Arbitrary seed editing requires an appropriate editor on another shell.                                                               |
| Sketch   | CLI gives ordered vertices; TUI places vertices with a cursor and confirms once.                                 | Polyline via keyboard or pointer, freehand via actual pointer samples; canceled gestures stay transient. | Touch vertices, stylus if supported, or accessible coordinate editor. Preserve touch cancellation and IME boundaries.        | A prepared export or other scoped confirmation may fit a form. This profile cannot author the polyline or freehand stroke.                                                      |
| Mind Map | CLI names source and destination IDs; a keyboard outline picker shows paths and labels.                          | Drag or use “Move under…”. The same two IDs are committed.                                               | Use a searchable semantic outline and Move command as an alternative to dragging.                                            | A token-bound choice may confirm two preselected nodes. Arbitrary rename or search needs free text outside this profile.                                                        |

## What needs to exist before a release can claim these loops

1. Package Colony and Burrow as independent tiny guests in the established Roc package structure. Publish full records and paired snapshot/restore. Add the existing regression cases and the new passages. Start by proving the normal Linux input-to-journal-to-record path. [Guest ABI][abi] · [Live parity test][live]
2. Add Orbit and Cells on the same platform. Define one admitted source of logical ticks per world and phase order at collisions. Exercise timer-denied/manual-step, view stall, duplicate commit and rejoin. Compare each complete intermediate record with the supplied expected result. Add ECS as a secondary observer. [ECS issue](https://github.com/developing-today/code/issues/5781)
3. Bring the games to Android through the existing native-client plan. Then verify Web, macOS and Windows in product priority order. Add shell-level assertions for focus, refusal text, cancellation and exactly one committed action per deliberate activation. Record a named device/run for any device claim. [Android implementation](https://github.com/developing-today/code/issues/7938)
4. Add Sketch's immutable stroke identity, rights, tombstone and attributed copy. Add Mind Map's stable node/link operations and single-history cycle guard. Put authorized collection and replay underneath both adapters. A release promising offline simultaneous edits also needs the independent two-replica cases and a resolved, documented tree policy. [Prior fixtures](evidence/app-fixtures.json)
5. Verify the true capability profile before advertising each loop. Use the program's current issue ownership, Fleet dispatch and release policy for any repo slice. Store dispatch/run URL, exact toolchain, module and commit with the fixture results. No build, deployment, claim or issue is implied by this playbook.

[ttt]: https://github.com/developing-today/code/blob/7310a95730791e66da98f7b0e13865930058a516/pkgs/id/apps/tic-tac-toe/README.md
[territory]: https://github.com/developing-today/code/blob/7310a95730791e66da98f7b0e13865930058a516/pkgs/id/examples/territory/main.roc
[gridexample]: https://github.com/developing-today/code/blob/7310a95730791e66da98f7b0e13865930058a516/pkgs/id/examples/roc-grid/main.roc
[grid]: https://github.com/developing-today/code/blob/7310a95730791e66da98f7b0e13865930058a516/pkgs/id/examples/roc-world/RocGrid.roc
[hardware]: https://github.com/developing-today/code/blob/7310a95730791e66da98f7b0e13865930058a516/pkgs/id/crates/id-hw-shell/src/lib.rs
[abi]: https://github.com/developing-today/code/blob/7310a95730791e66da98f7b0e13865930058a516/pkgs/id/examples/roc-world/ABI.md
[live]: https://github.com/developing-today/code/blob/7310a95730791e66da98f7b0e13865930058a516/pkgs/id/crates/id-native-shell/tests/live_session.rs
[dev]: https://github.com/developing-today/code/blob/7310a95730791e66da98f7b0e13865930058a516/pkgs/id/crates/id-worlds/src/world_dev.rs
[widget]: https://github.com/developing-today/code/blob/7310a95730791e66da98f7b0e13865930058a516/pkgs/id/crates/id-native-shell/src/widget.rs

[Independent arithmetic review of the app transcripts](evidence/app-vector-review.txt)
