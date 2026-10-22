//! An extrude (or other feature) whose profile is picked by its curves also saves `holes`: for
//! each picked loop, the hole loops its profile had when it was picked. An island a later sketch
//! edit puts inside the loop (such as a face's projected outline once a rectangle is drawn round
//! it, #4558) is then filled instead of turning the pick into a ring. Older designs need no
//! transformation: a pick saved without `holes` keeps its islands as holes, as before. The last
//! release refuses a design saved with this (the newer format number).
