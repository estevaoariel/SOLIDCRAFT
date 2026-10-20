# Exploded views

An exploded view shows an assembly taken apart. It is **shown only**: the design keeps its
placements, its component origins and its joint definitions, and showing or hiding the view
never writes to the design.

A view holds a list of **steps**. Each step moves some occurrences; a step's moves are added to
the ones before it, so the view is a sequence that can be played back. The view also has an
**easing** (how one step blends into the next) and can draw **trail lines**: a dashed line from
every moved occurrence back to the place it is assembled in.

## The commands

```json
{"command":"explode.create","params":{"name":"Apart","trail_lines":true,"easing":"ease_in_out"}}
{"command":"explode.step_add","params":{"view":"Apart","name":"Lid off","moves":[{"occurrence":"Lid:1","translate":[0,0,25]}],"duration_ms":500}}
{"command":"explode.show","params":{"view":"Apart","step":1,"progress":0.5}}
{"command":"explode.show","params":{"view":"Apart","time":750}}
{"command":"explode.show","params":{"view":"none"}}
```

| command | what it does |
|---|---|
| `explode.create` | makes a view. `name?`, `scale?` (the automatic moves, as before), `moves?` (instead of automatic), `trail_lines?` (default true), `easing?` (`linear`, `ease_in`, `ease_out`, `ease_in_out`; default `ease_in_out`). |
| `explode.edit` | changes a view: `view`, `name?`, `trail_lines?`, `easing?`. |
| `explode.step_add` | adds a step at the end: `view`, `name?`, `moves?`, `duration_ms?` (0…600000 ms). |
| `explode.step_edit` | changes a step: `view`, `step` (index or id), `name?`, `moves?`, `duration_ms?`. |
| `explode.step_delete` | removes a step: `view`, `step` (index or id). |
| `explode.step_reorder` | moves a step: `view`, `from` (index), `to` (index; past the end appends). |
| `explode.show` | shows the view: `view` (id, name, or `none`), `step?` (index), `progress?` (0…1), `time?` (ms into the whole sequence). |
| `explode.list` | the views, with their steps. |
| `explode.delete` | removes a view: `view`. |

A step's `moves` **replace** the step's moves when they are given. A step with no moves is
legal: it is a pause in the sequence. A `step` is matched by index first and then by the step's
id; one that names no step is an error.

`explode.show` answers where it got to, so a caller can follow the sequence:

```json
{"view":3,"name":"Apart","step":1,"progress":0.5,"time_ms":750}
```

`time` and `step`/`progress` are two ways of saying the same thing; when `time` is given it wins.
The steps have lengths (`duration_ms`, 0 meaning the default 500 ms); the time is clamped to the
length of the whole sequence and lands in the step whose span holds it.

## The pose

For an occurrence `o` the offset at the end of step `k` is the sum of the moves of the steps up to
and including `k`. Inside step `k`, at progress `p`, the pose is the sum of the earlier steps plus
the eased part of step `k`'s own move:

```
offset(o) = Σ_{j < k} move_j(o) + ease(p) · move_k(o)
```

`ease` is the view's easing: `t` for linear, `t²` for ease in, `1-(1-t)²` for ease out and the
smoothstep `t²(3-2t)` for ease in and out. A view with no steps shows the assembled placement.

Showing a view writes the occurrence's placement to the session's pending moves — the document
transform with the translation added — and remembers which view is on show. Undo and redo keep
the view and the moves together. Showing `none` puts the occurrences back exactly where they
were.

## The trail lines

A trail line joins an occurrence's **assembled place** to its **exploded place** — the two
positions of the occurrence's origin. Only the occurrences the view actually moves get a line.
The lines are dashed, so they read as reference geometry rather than as edges of the model, and
they are depth-tested: the model hides a trail line that lies behind a body.

The segments are worked out every time the scene is built, from the design and the pending moves;
nothing about them is saved. They are drawn only when the view on show has trail lines set.

## In the app

The **Exploded View** dialog holds the view's name, its easing, the trail-lines switch, the list
of steps and a scrubber over the whole sequence. The steps can be added, renamed, reordered and
deleted, and each step's moves are edited as a list of `occurrence: translate [x, y, z]` rows.
Play, Pause and Loop move the scrubber in real time at the chosen speed, and the pose follows.
OK saves the view and shows it; closing the dialog without OK puts the occurrences back.

While the dialog is open, the occurrence picked in the viewport gets a translation gizmo at the
place the part is **now**, so it can be pulled along X, Y or Z. The drag changes the current
step's move for that occurrence, never the occurrence's own placement.

## The file format

A design stores each view's steps, its trail-lines switch and its easing. A design written
before the change stores a flat list of moves; it is read as a view with a single step holding
those moves, so the explosion is shown exactly as it was. The change is a pending format step
(`crates/doc/src/format/pending/explode-steps.rs`) and is folded into the numbered format on the
next release.
