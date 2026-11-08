# ext-preview-3d

`ext-preview-3d` previews STL (ASCII and binary, through `stl_io`), OBJ
(through `tobj`, materials read for their colour and nothing else) and 3MF
(the zip's model part read with `roxmltree`, the declared unit and the
build transforms honoured) files. `open` reads and parses the file off the
UI thread; `render` draws the mesh through wgpu into an off-screen target of
the requested size, a shaded solid in a neutral material over a ground
grid, and reads it back as an RGBA frame the host uploads, so the plugin
never touches egui. The document claims three pointer gestures as
`PreviewControl::Pointer` controls, orbit by drag, zoom by wheel and reset
by double-click, which the preview pane forwards through
`set_control` with no 3D code of their own; the view lives in the document
and so is kept per file while it is open. The status bar shows the format,
the triangle count, the bounding box in millimetres and, for OBJ, the
number of objects; the File info section adds the 3MF unit and metadata. A
model above the triangle budget (`triangle_budget` in `preview-3d.toml`,
2 000 000 by default) is decimated for the preview by vertex clustering
and the status bar says so. Without a graphics adapter the preview shows
the facts and says that rendering is unavailable. It is a plugin
(`cdylib`, loaded from `plugins/preview-3d/`) and an `rlib` for the
binary's opt-in `preview-3d` feature. It depends on `ext` and `types`
only; `rick` (behind the feature) and `tests` link it, and the plugin host
loads it.
