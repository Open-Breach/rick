#!/usr/bin/env python3
"""Writes giraffe.stl, the sample 3D model of the README's screenshot.

An original low-poly giraffe made for this repository (specs/readme.md,
"Screenshot"), the animal of release 0.4.1: convex blocks put together, in
millimetres, Z up, facing -Y as the 3D previewer's default view expects.
Run it to regenerate the file beside it; the output is the same bytes
every time.

    python3 assets/readme/giraffe.py
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


def giraffe():
    parts = [
        # Body, on long legs.
        box((0, 4, 44), (9, 15, 8), 3),
    ]
    for side in (-1, 1):
        for y in (-7, 15):
            # A leg and its hoof.
            parts.append(box((side * 6.2, y, 19), (2.3, 2.3, 19)))
            parts.append(box((side * 6.2, y - 0.6, 2.2), (3.2, 3.8, 2.2), 0.8))
        # Spots on the flanks.
        for y, z, radius in ((-4, 46.5, 3.2), (5, 42, 2.6), (12, 47, 3.0)):
            parts.append(prism((side * 9.0, y, z), radius, 0.9, 6, 0))
    # The neck and what it carries, built upright and tipped forward as one.
    pivot = (0, -8, 48)
    upright = [
        # Neck and the mane down its back.
        frustum((0, -8, 46), (5, 5.5), (0, -8, 90), (3.2, 3.4)),
        box((0, -3.4, 70), (0.9, 1.5, 17)),
        # Head and muzzle.
        box((0, -13, 94.5), (5.5, 9, 5.5), 2),
        box((0, -24, 92.5), (4, 3.2, 3.6), 1.2),
        # The two horns with their knobs.
        box((-2.7, -8.5, 102.5), (0.8, 0.8, 3)),
        box((2.7, -8.5, 102.5), (0.8, 0.8, 3)),
        ball((-2.7, -8.5, 106.6), 1.9),
        ball((2.7, -8.5, 106.6), 1.9),
    ]
    for side in (-1, 1):
        # Ears out to the sides, eyes, and spots on the neck.
        upright.append(frustum((side * 6, -6.5, 96.5), (2.4, 1.6), (side * 11, -6.5, 100), (1.0, 0.8)))
        upright.append(prism((side * 5.4, -16, 96.5), 1.6, 0.5, 8, 0))
        for z, half_width, radius in ((58, 4.5, 2.4), (72, 4.0, 2.0), (83, 3.5, 1.6)):
            upright.append(prism((side * half_width, -8, z), radius, 0.9, 6, 0))
    for part in upright:
        parts.append(lean(part, pivot, -16))
    # The tail, down and back, with its tuft.
    root = (0, 19, 46)
    parts.append(lean(frustum((0, 19, 30), (0.8, 0.8), (0, 19, 46), (1.2, 1.2)), root, -22))
    parts.append(lean(ball((0, 19, 28), 2.2), root, -22))
    triangles = []
    for part in parts:
        triangles.extend(hull(part))
    return triangles


def main():
    triangles = giraffe()
    header = b"giraffe: an original low-poly model made for Rick (Apache-2.0)"
    data = bytearray(header.ljust(80, b" "))
    data += struct.pack("<I", len(triangles))
    for normal, a, b, c in triangles:
        data += struct.pack("<12fH", *normal, *a, *b, *c, 0)
    path = pathlib.Path(__file__).with_name("giraffe.stl")
    path.write_bytes(data)
    print(f"{path}: {len(triangles)} triangles, {len(data)} bytes")


if __name__ == "__main__":
    main()
