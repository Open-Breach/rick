# vfs

`vfs` routes a `Location` to the `Provider` that owns it through the `Mounts`
table, and composes what no single provider can do alone: copying and moving
between providers from `open_read`, `open_write`, `list` and `create_dir`,
with progress and cancellation, while same-provider operations go to the
provider's own fast path. It depends on `types`, `runtime` and `ext`;
`vfs-local`, `model`, `ops` and the crates above them may depend on it.
