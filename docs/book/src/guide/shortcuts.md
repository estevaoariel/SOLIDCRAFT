# Keyboard shortcuts

Command shortcuts can be changed in File › Keyboard Shortcuts; these are the defaults.

| Key | Command | Key | Command |
|---|---|---|---|
| L | Line | E | Extrude |
| R | Two-Point Rectangle | Q | Press Pull |
| C | Center Diameter Circle | F | Fillet |
| D | Sketch Dimension | H | Hole |
| X | Construction | M | Move/Copy |
| O | Offset | J | Joint |
| T | Trim | I | Measure |
| P | Project | A | Appearance |
| Ctrl+Z | Undo | Ctrl+Y | Redo |
| Ctrl+N | New Design | Ctrl+O | Open |
| Ctrl+S | Save | Delete | Delete the selection |

Keys handled by the window itself:

| Key | Action |
|---|---|
| S | command search under the ribbon search field |
| V | show or hide the selected bodies, sketches and planes |
| F2 | rename the selected browser or timeline item |
| F6 | fit the view to the model |
| ← → ↑ ↓ | turn the view 15° (with Shift, a quarter turn) |
| Ctrl+← → ↑ ↓ | pan the view |
| Ctrl+8 | look straight at the selected face, plane or sketch |
| Esc | cancel the running command, or clear the selection |

While a value box has the keyboard (a dimension being added, a dialog field), keys type into
it and single-letter shortcuts don't fire. With Sketch Dimension (D), the box opens as soon as
you click a line, circle or arc (or the second point or line), so D, click, type the value,
Enter.

Mouse and touch:

| Action | Result |
|---|---|
| Click | select (Shift or Ctrl adds) |
| Click again on the same spot | select the next thing under the cursor (click cycling) |
| Hold the left button still (long-press on touch), or right-click › Select Other… | list everything under the cursor to pick from |
| Double-click a sketch curve or edge | select its chain or tangent loop |
| Double-click the middle button | go Home: the home view, framing everything shown (Preferences › Navigation › Middle double-click: Home, Fit or Off) |

Help › Keyboard Shortcuts in the app lists the current bindings, including any you changed.

## Shortcut sets

File › Keyboard Shortcuts › **Shortcut set** picks the keys commands start with: **Default**
(the tables above) or **Onshape**. Keys you bound yourself win over either set: when the set
gives one of them to another command or makes it a view key, your command keeps it and the
set's command shows no key. A command you bind back to the set's own key follows the set again.
Restore Defaults on that page goes back to the Default set with no changes of your own. Scripts
and agents choose a set with `ui.prefs {"shortcut_set": "onshape"}`.

Digit-row keys are read by their position, so Shift+7 is `Shift+7` on every keyboard layout. A
key saved under the character a layout typed (German Shift+7 as `Shift+Slash`) still runs.

### Onshape set

Keys follow the public [Onshape keyboard shortcut list](https://cad.onshape.com/help/Content/shortcut_keys.htm),
mapped to SolidCraft's own commands. Where one key starts a sketch command and a solid command,
the sketch command runs while a sketch is open.

| Key | Command | Key | Command |
|---|---|---|---|
| Shift+S | Create Sketch (in a sketch: Point) | L | Line |
| Shift+E | Extrude | R | Center Rectangle |
| Shift+W | Revolve | G | 2-Point Rectangle |
| Shift+F | Fillet (in a sketch: sketch Fillet) | C | Center Diameter Circle |
| Shift+X | Section Analysis | A | 3-Point Arc |
| [ | Measure | Shift+A | Tangent Arc |
| Shift+C | Curvature Comb Analysis | D | Sketch Dimension |
| I | Coincident | Q | Normal/Construction |
| E | Equal | O | Offset |
| H | Horizontal/Vertical | U | Project |
| B | Parallel | M | Trim |
| Shift+L | Perpendicular | X | Extend |
| Shift+O | Concentric | T | Tangent |
| Shift+Q | Symmetry | Shift+M | MidPoint |
| Shift+U | Curvature | Shift+J | Fix/UnFix |

L, C, D and O are the same keys in both sets. J stays Joint, SolidCraft's own key (Onshape's J
shows or hides mates in assemblies). Press Pull, Hole, Move/Copy and Appearance have no key in
this set (Onshape uses Q, H, M and A for sketch tools); bind them yourself if you want one.

View keys the window handles in this set (a free one can't be given to a command while the set
is on):

| Key | Action |
|---|---|
| Shift+1 … Shift+7 | front, back, left, right, top, bottom, isometric view |
| F | fit the view |
| N | look straight at the selected face, plane or sketch |
| Z / Shift+Z | zoom out / in (turned round by Preferences › Navigation's reverse zoom, as the wheel is) |

On the flat pattern, Z and Shift+Z zoom the pattern and F fits it; the view keys that turn the
view do nothing there.

Everything under "Keys handled by the window itself" keeps working as well (S, V, F2, F6, the
arrows, Ctrl+8, Esc). Onshape keys with no SolidCraft equivalent, or that would take
one of those keys, have no binding: V (vertical; H covers both directions), Shift+K (normal),
Shift+G (pierce), Shift+D (dihedral analysis), P and Shift+P (planes and construction display),
Y, Shift+Y, Shift+I and Shift+T (hide, show, isolate, transparency; V shows or hides), Shift+H
(sketch visibility), Shift+V (named views), Shift+N (rename; F2), W (zoom window), Shift+R
(high-quality view; Shift+R repeats here), Space (deselect; Space pans here), Ctrl+arrows (5°
turn; they pan here), Ctrl+Shift+arrows (move the design), the backtick (Select Other is a long
press or right-click › Select Other…), K and Ctrl+M (mate connectors), A as flip the primary axis
(A is the 3-point arc here), Shift+Enter (finish and start the command again; Shift+R repeats the last
one), holding Shift (lock or suppress inferences), the arrows on the rollback bar (the arrows
turn the view here), Shift+/ (the shortcut list; Help › Keyboard Shortcuts), Alt+C (the search
field; S), Ctrl+U (feedback), Alt+T and Ctrl+Space (browser tabs), Ctrl+Shift+F (FeatureScript
search), and the assembly, drawing and Feature Studio keys.

With the **Touchpad** navigation preset, hold **Shift** while moving the pointer to pan, or
**Alt/Option** to orbit, without pressing a mouse button. Vertical scrolling zooms. These
viewport gestures are separate from rebindable command shortcuts; mouse-button selection
and editing keep their existing modifiers.

### Sketch modifiers

While drawing or dragging a sketch point:

| Keys | Action |
|---|---|
| Ctrl/Cmd or Alt/Option (default) | suppress auto-snap for that gesture |
| Shift | prefer a line midpoint when targeting |

Change the snap-suppress chord in Preferences › Sketch. Selection still uses Shift and
Ctrl/Cmd to add or toggle (Preferences › Selection can make Shift add-only).
