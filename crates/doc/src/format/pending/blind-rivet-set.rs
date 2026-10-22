//! A blind rivet placed drawn set (`parts.fasten`, `drawn: "set"`) keeps the grip it closes on
//! in its `parts` entry (`set`), so it rebuilds with its shank that long and its formed tail
//! beyond. Older designs need no transformation: their parts have no `set` and are drawn unset;
//! 0.14.x asks for an update instead of opening a design that has one.
