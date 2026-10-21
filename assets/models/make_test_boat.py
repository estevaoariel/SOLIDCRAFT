#!/usr/bin/env python3
"""Builds assets/models/test_boat.bin: a reduced copy of a public-domain boat model, for the 3D mouse
test pane in Preferences. Needs numpy and fast-simplification (pip install numpy fast-simplification).

    python3 assets/models/make_test_boat.py SOURCE.stl assets/models/test_boat.bin [TRIANGLES]

SOURCE.stl is the binary STL of the full model (225,706 triangles). Vertices are welded, the mesh
is reduced by quadric edge collapse to about TRIANGLES (default 6000), centred, scaled into a unit
box and quantised. File layout, little-endian: u32 vertex count, u32 triangle count, f32 centre
(x, y, z), f32 half size of the longest side, then the vertices as i16 x 3 (32767 is the half size)
and the triangles as u16 x 3 indices. The model sits with z up.
"""
import struct
import sys

import numpy as np
import fast_simplification


def main() -> None:
    source, target = sys.argv[1], sys.argv[2]
    triangles = int(sys.argv[3]) if len(sys.argv) > 3 else 6000
    data = open(source, "rb").read()
    count = struct.unpack_from("<I", data, 80)[0]
    record = np.dtype([("n", "<f4", 3), ("v", "<f4", (3, 3)), ("attr", "<u2")])
    tris = np.frombuffer(data, dtype=record, count=count, offset=84)["v"].reshape(-1, 3)
    points, inverse = np.unique(np.round(tris, 1), axis=0, return_inverse=True)
    faces = inverse.reshape(-1, 3).astype(np.int32)
    keep = (faces[:, 0] != faces[:, 1]) & (faces[:, 1] != faces[:, 2]) & (faces[:, 0] != faces[:, 2])
    points, faces = points.astype(np.float32), faces[keep]
    # The collapse stops short on a mesh of this kind, so go round again on its own result.
    for _ in range(60):
        if len(faces) <= triangles * 1.05:
            break
        points, faces = fast_simplification.simplify(points, faces, target_reduction=1.0 - triangles / len(faces), agg=10)
    low, high = points.min(axis=0), points.max(axis=0)
    centre, half = (low + high) / 2.0, float((high - low).max() / 2.0)
    quantised = np.clip(np.round((points - centre) / half * 32767.0), -32767, 32767).astype("<i2")
    with open(target, "wb") as out:
        out.write(struct.pack("<II3ff", len(points), len(faces), *centre.tolist(), half))
        out.write(quantised.tobytes())
        out.write(faces.astype("<u2").tobytes())
    print(f"{len(points)} vertices, {len(faces)} triangles, centre {centre}, half size {half}")


if __name__ == "__main__":
    main()
