#!/usr/bin/env python3
"""Writes pig.stl, the sample 3D model of the README's screenshot.

An original low-poly pig made for this repository (specs/readme.md,
"Screenshot"), the animal of release 0.6.0: convex blocks put together, in
millimetres, Z up, facing -Y as the 3D previewer's default view expects.
Run it to regenerate the file beside it; the output is the same bytes
every time.

    python3 assets/readme/pig.py
"""

import itertools
import math
import pathlib
import struct

EPS = 1e-6


def sub(a, b):
    return (a[0] - b[0], a[1] - b[1], a[2] - b[2])


def cross(a, b):
    return (
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    )


def dot(a, b):
    return a[0] * b[0] + a[1] * b[1] + a[2] * b[2]


def unit(a):
    length = math.sqrt(dot(a, a))
    return (a[0] / length, a[1] / length, a[2] / length)


def hull(points):
    """The triangles of the convex hull of `points`, facing outward."""
    faces = {}
    for i, j, k in itertools.combinations(range(len(points)), 3):
        normal = cross(sub(points[j], points[i]), sub(points[k], points[i]))
        if dot(normal, normal) < EPS:
            continue
        normal = unit(normal)
        sides = [dot(normal, sub(p, points[i])) for p in points]
        if max(sides) > EPS and min(sides) < -EPS:
            continue
        if max(sides) > EPS:
            normal = (-normal[0], -normal[1], -normal[2])
        on_plane = tuple(n for n, side in enumerate(sides) if abs(side) <= EPS)
        faces[on_plane] = normal
    triangles = []
    for on_plane, normal in sorted(faces.items()):
        corners = [points[n] for n in on_plane]
        centre = tuple(sum(c[axis] for c in corners) / len(corners) for axis in range(3))
        u = unit(sub(corners[0], centre))
        v = cross(normal, u)
        corners.sort(key=lambda c: math.atan2(dot(sub(c, centre), v), dot(sub(c, centre), u)))
        for n in range(1, len(corners) - 1):
            triangles.append((normal, corners[0], corners[n], corners[n + 1]))
    return triangles


def box(centre, half, chamfer=0.0):
    """A box, its twelve edges cut at 45 degrees when `chamfer` is set."""
    points = set()
    for sx, sy, sz in itertools.product((-1, 1), repeat=3):
        if chamfer == 0.0:
            points.add((sx * half[0], sy * half[1], sz * half[2]))
            continue
        for axis in range(3):
            corner = [sx * (half[0] - chamfer), sy * (half[1] - chamfer), sz * (half[2] - chamfer)]
            corner[axis] = (sx, sy, sz)[axis] * half[axis]
            points.add(tuple(corner))
    return [(x + centre[0], y + centre[1], z + centre[2]) for x, y, z in sorted(points)]


def frustum(base, base_half, top, top_half):
    """A tapered block between two rectangles parallel to the ground."""
    points = []
    for centre, half in ((base, base_half), (top, top_half)):
        for sx, sy in itertools.product((-1, 1), repeat=2):
            points.append((centre[0] + sx * half[0], centre[1] + sy * half[1], centre[2]))
    return points


def prism(centre, radius, half_depth, sides, axis):
    """A regular prism whose axis is X (0) or Y (1)."""
    points = []
    for n in range(sides):
        angle = math.tau * (n + 0.5) / sides
        a, b = radius * math.cos(angle), radius * math.sin(angle)
        for depth in (-half_depth, half_depth):
            offset = (depth, a, b) if axis == 0 else (a, depth, b)
            points.append((centre[0] + offset[0], centre[1] + offset[1], centre[2] + offset[2]))
    return points


def ball(centre, radius):
    """An icosahedron."""
    phi = (1 + math.sqrt(5)) / 2
    scale = radius / math.sqrt(1 + phi * phi)
    points = []
    for a, b in itertools.product((-1, 1), (-phi, phi)):
        for x, y, z in ((0, a, b), (a, b, 0), (b, 0, a)):
            points.append((centre[0] + x * scale, centre[1] + y * scale, centre[2] + z * scale))
    return points


def lean(points, pivot, degrees):
    """Tips `points` backwards (towards +Y) around the X axis through `pivot`."""
    sin, cos = math.sin(math.radians(degrees)), math.cos(math.radians(degrees))
    out = []
    for x, y, z in points:
        dy, dz = y - pivot[1], z - pivot[2]
        out.append((x, pivot[1] + dy * cos + dz * sin, pivot[2] - dy * sin + dz * cos))
    return out


def pig():
    parts = [
        # The barrel of a body, and the rump rounding it off.
        box((0, 6, 25), (13, 17, 11.5), 5.5),
        ball((0, 20, 26), 13.5),
        # The belly hangs a little.
        box((0, 5, 16.5), (9.5, 12, 4), 2.5),
        # The tail: a curl of three small knots.
        ball((0, 33, 31), 2.4),
        ball((1.8, 35.5, 33.5), 2.0),
        ball((-0.6, 36.5, 36.5), 1.7),
    ]
    for side in (-1, 1):
        for y in (-5, 17):
            # A short leg and its trotter.
            parts.append(frustum((side * 7.5, y, 14), (3.6, 3.6), (side * 7.5, y, 3), (2.7, 2.7)))
            parts.append(box((side * 7.5, y - 0.4, 1.6), (3.0, 3.3, 1.6), 0.6))
    # The head, built level and nodding forward as one.
    pivot = (0, -10, 28)
    level = [
        # Head and cheeks.
        box((0, -18, 29), (9.5, 8.5, 9), 3.5),
        # The snout, a flat disc with two nostrils sunk beside its middle.
        prism((0, -29.5, 26.5), 5.2, 3.2, 10, 1),
        prism((0, -33.2, 26.5), 4.4, 0.6, 10, 1),
    ]
    for side in (-1, 1):
        # Ears standing and tipping forward, the eyes, the nostrils.
        level.append(frustum((side * 6.2, -15, 36.5), (3.4, 2.2), (side * 8.2, -20.5, 44.5), (0.9, 0.7)))
        level.append(prism((side * 9.4, -22, 32.5), 1.7, 0.5, 8, 0))
        level.append(prism((side * 1.8, -34.0, 26.8), 0.9, 0.4, 6, 1))
    for part in level:
        parts.append(lean(part, pivot, -8))
    triangles = []
    for part in parts:
        triangles.extend(hull(part))
    return triangles


def main():
    triangles = pig()
    header = b"pig: an original low-poly model made for Rick (Apache-2.0)"
    data = bytearray(header.ljust(80, b" "))
    data += struct.pack("<I", len(triangles))
    for normal, a, b, c in triangles:
        data += struct.pack("<12fH", *normal, *a, *b, *c, 0)
    path = pathlib.Path(__file__).with_name("pig.stl")
    path.write_bytes(data)
    print(f"{path}: {len(triangles)} triangles, {len(data)} bytes")


if __name__ == "__main__":
    main()
