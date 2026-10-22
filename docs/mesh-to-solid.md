# Importing a mesh as a solid body

A 3MF, STL, OBJ or glTF file imports as **mesh bodies**: they render, measure, move and export,
but the operations that need a B-rep — boolean, fillet, chamfer, shell, hole, extrude-cut,
sketch on a face — refuse them ("a boolean needs a solid body"). **Convert Mesh** rebuilds a
closed mesh as a solid body, so those operations work on it.

## How to convert

- **Right-click a mesh body → Convert to Solid** (in the browser or the viewport): it opens the
  Convert Mesh dialog with that body picked and its tolerance; or
- **Convert Mesh** from the toolbar or the command palette, a script, the CLI or the control
  channel: `mesh.convert {body, tolerance?, name?}`.
- **On import**: tick **Preferences → Design → Import meshes as solids**, or pass
  `as_solid: true` yourself (with a `tolerance`, default 0.05 mm). Each imported mesh is then
  converted where it can be; one that cannot stays a mesh body with a warning, so the import
  still succeeds.

## Workflow: before and after

**Before — a mesh body.** It is a row under **Bodies**, put there by the import feature in the
timeline. It renders, measures, moves and exports, but the solid operations refuse it ("a boolean
needs a solid body"): boolean, fillet, chamfer, shell, hole, extrude-cut, a sketch on a face.

**After — a solid body.** The conversion **replaces the mesh body in place**: the same row and
name in the browser (now a B-rep solid), and one **Convert Mesh** feature in the timeline
(`ConvertMesh1`). Nothing else is created, so scenes, appearances and downstream features keep
pointing at that body. From here the solid tools work (Combine, Fillet, Chamfer, Shell, Hole,
Extrude/Revolve, Measure, a sketch on a face, STEP/IGES export), and:

- **Undo** (`Ctrl+Z`) or **deleting the Convert Mesh feature** brings the mesh back; a saved
  design rebuilds the conversion from the timeline when it is opened.
- The feature is not edited in the timeline yet: to change the tolerance, delete it and run
  Convert Mesh again.
- To keep the mesh as well, copy the body before converting — **Move/Copy** (`M`) with **Copy**
  ticked, or `solid.move {bodies: ["Mesh1"], copy: true}` — and convert one of the two.
- The result follows the features before it (it converts whatever the upstream features last
  built).

## What the conversion does

1. Welds coincident vertices and checks the mesh: closed (two triangles per edge) and
   consistently oriented (every edge used once each way). A mesh wound inwards is turned outward.
2. Tries the exact **prismatic** reconstruction first: a closed part that is one profile extruded
   straight (holes included) is rebuilt with planar faces and, where certified within the
   tolerance, circular cylindrical walls.
3. Otherwise every triangle becomes a planar facet, and neighbouring triangles that lie on one
   plane **fuse into a single face** (with its holes): a box's six sides stay six faces, and a
   polygonal hole's wall is a ring of quads.
4. Sews the faces into one solid, checks its validity, and compares its volume with the mesh's: a
   solid that does not close, or that measures another volume, is refused.

Everything that is not a plane (a bore's wall, a fillet, a freeform surface) stays **faceted**: the
conversion never invents a surface the mesh does not describe. A constant-section part gets its
cylinders exactly, because that case is certified analytically.

## Limits and errors

- A closed, consistently oriented mesh of at most **50 000 triangles**, leaving at most
  **20 000 fused faces**, each with a boundary of at most 8 192 edges. A bigger mesh is refused
  with its size and the limit: simplify it first.
- An open or non-manifold mesh, a face boundary that branches (the mesh touches itself at a
  point), or a solid that cannot be sewn or validated is refused with a message saying which;
  the design is left unchanged, and on import the body stays a mesh.
- The work reports its progress to the import card and stops on Cancel.
- Nothing panics: the kernel guard turns an internal failure into an error.

## What works afterwards

Booleans (join, cut, intersect), fillet, chamfer, shell, hole, extrude-cut, a sketch on a face,
measure, and export to STEP, IGES, 3MF and STL. A fillet on a faceted wall selects that facet's
edge, as on any other B-rep: the conversion does not smooth the input's facets into a surface the
file never had.

The conversion cannot name its faces from history (there is none to carry over from a triangle
mesh), so a sketch placed on one of them is a **fixed** attachment: it sits on that face but does
not follow a later upstream edit that reshapes the mesh. The sketch command says so when the face
is picked.
