# Macros

A **macro** is a [Rhai](https://rhai.rs) script the engine runs. Record captures the commands you
use as Rhai; Save writes `.rhai`. You can add `if` and loops. Play is one undo step; a failure
rolls back.

Command **scripts** (`solidcraft-cli run file.json`) stay JSON. That is not the MACRO tab.

## Record / play

The toolbar's **MACRO** tab has Record, Pause, Resume and Stop. Pause keeps the recording and
skips the commands you run until Resume. A chip at the top of the window stays up for the whole
recording.

Save writes a `.rhai` file. Open loads one. Play runs it. A macro can ask for values first
(`// @param name Label | default`); Play shows one row each. `{a, b}` after the label is a
dropdown. `@if other=a,b` hides the row unless that field is one of those values (and that field's own row is shown).

```rhai
// @param min_dia Minimum diameter | 5 mm
let holes = run("inspect.find_holes", #{ min_diameter: min_dia });
run("solid.chamfer", #{ edges: holes.edges, distance: "1 mm" });
```

`run(command, params)` is `engine.execute`. `list_dir(path)` and `write_file(path, text)` are for
batch files (not in the browser). `file_stem(path)` is the name without extension.

`inspect.find_holes` finds cylindrical holes. `max_diameter` / `min_diameter` filter by size.
Empty `edges` on Chamfer is a no-op.

## Examples

Examples live in [examples/macros](../examples/macros). Open the `.rhai` file, then Play. Play
`setup_*` first when the next macro needs that design.

| Example | Play on | What it shows |
|---|---|---|
| `setup_fixtureplate.rhai` | empty | C-shaped sketch `clearance_hole` (100×60 mm, 50×30 mm notch from the right) |
| `example_fixtureplate.rhai` | after setup | 500×350×8 mm tray, six Ø8 holes chamfered 1 mm both ends, C cut patterned `nx`×`ny` at `spacing_x`/`spacing_y`, centered on the tray |
| `chamfer_holes.rhai` | a part with holes | Query holes, chamfer by size |
| `batch_export_stl.rhai` | empty | Folder of STEP → STL |
| `gear_generator.rhai` | empty | Involute gears (module or DP, 14.5/20/25°, full/stub, helical, internal) and ANSI roller-chain sprockets (layout tooth form, tooth-edge chamfer, optional hub) |

The browser plays Rhai in the same engine. `list_dir` / `write_file` are desktop only.
