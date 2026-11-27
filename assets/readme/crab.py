#!/usr/bin/env python3
"""Writes crab.stl, the sample 3D model of the README's screenshot.

An original low-poly crab made for this repository (specs/readme.md,
"Screenshot"), the animal of release 0.6.1: convex blocks put together, in
millimetres, Z up, standing across the 3D previewer's default view, which
looks along Y: a crab is wider than long.
Run it to regenerate the file beside it; the output is the same bytes
every time.

    python3 assets/readme/crab.py
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


def turn(points, pivot, degrees):
    """Turns `points` around the vertical axis through `pivot`."""
    sin, cos = math.sin(math.radians(degrees)), math.cos(math.radians(degrees))
    out = []
    for x, y, z in points:
        dx, dy = x - pivot[0], y - pivot[1]
        out.append((pivot[0] + dx * cos - dy * sin, pivot[1] + dx * sin + dy * cos, z))
    return out


def crab():
    parts = [
        # The shell: wide, thick and chamfered, with a lower plate under it.
        box((0, 2, 21), (15, 11, 8), 4.5),
        box((0, 2, 13), (11.5, 8, 2.5), 1.5),
        # A ridge along the back of the shell.
        box((0, 5, 28), (8.5, 4.5, 2.4), 1.6),
    ]
    for side in (-1, 1):
        # The eyes on their stalks, at the front edge.
        parts.append(frustum((side * 4.5, -7.5, 27), (1.6, 1.6), (side * 5, -9.5, 34), (1.1, 1.1)))
        parts.append(ball((side * 5.1, -9.8, 35.8), 3.0))
        # Four walking legs, fanned out a little: a thigh rising from the
        # shell to the knee, and a sturdy shin coming down to the foot.
        for n, angle in enumerate((-24, -8, 8, 24)):
            hip = (side * 12, 2 + (n - 1.5) * 4.6, 18)
            knee = (side * 20.5, hip[1], 25)
            thigh = frustum((side * 12, hip[1], 17), (3.0, 2.0), knee, (2.6, 1.8))
            shin = frustum((side * 24, hip[1], 0), (1.5, 1.3), knee, (2.6, 1.8))
            for part in (thigh, shin):
                parts.append(turn(part, hip, side * angle))
        # The claw, raised: an arm reaching up and forward, a thick hand
        # and two fingers pointing at the sky.
        parts.append(frustum((side * 12, -6, 19), (3.0, 3.0), (side * 17, -13, 31), (2.6, 2.6)))
        parts.append(box((side * 18, -14, 36), (4.8, 4.4, 5.6), 1.9))
        parts.append(frustum((side * 15.5, -14, 40), (2.1, 2.3), (side * 15, -14.5, 49), (0.7, 0.7)))
        parts.append(frustum((side * 20.7, -14, 40), (1.9, 2.1), (side * 21.6, -14.5, 47), (0.7, 0.7)))
    # A crab is wider than it is long, and it walks sideways: it stands
    # across the view, its wide side along Y, and centred on the origin,
    # which the default view looks at.
    turned = [[(-y, x, z) for x, y, z in part] for part in parts]
    xs = [p[0] for part in turned for p in part]
    ys = [p[1] for part in turned for p in part]
    shift = ((min(xs) + max(xs)) / 2, (min(ys) + max(ys)) / 2)
    triangles = []
    for part in turned:
        triangles.extend(hull([(x - shift[0], y - shift[1], z) for x, y, z in part]))
    return triangles


def main():
    triangles = crab()
    header = b"crab: an original low-poly model made for Rick (Apache-2.0)"
    data = bytearray(header.ljust(80, b" "))
    data += struct.pack("<I", len(triangles))
    for normal, a, b, c in triangles:
        data += struct.pack("<12fH", *normal, *a, *b, *c, 0)
    path = pathlib.Path(__file__).with_name("crab.stl")
    path.write_bytes(data)
    print(f"{path}: {len(triangles)} triangles, {len(data)} bytes")


if __name__ == "__main__":
    main()
