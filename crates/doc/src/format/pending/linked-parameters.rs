//! A design can use the user parameters of another design (Link Parameters): `param_sources`
//! lists each linked design's path, which of its parameters come in (`scope`) and their values as
//! last read. Older designs need no transformation: they link no parameters. The last release
//! refuses a design saved with this (the newer format number).
