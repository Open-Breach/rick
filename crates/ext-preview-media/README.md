# ext-preview-media

`ext-preview-media` previews MP3, WAV, MP4 and AVI files through GStreamer,
one code path on every platform: the system GStreamer on Linux, the runtime
the release archives bundle on macOS and Windows (QUESTIONS.md 89). It
reads the headers and tags itself (ID3, RIFF `INFO`, MP4 `ilst`), asks the
Discoverer for the streams, opens a `playbin3` whose video sink hands the
frames to the document as RGBA and whose transport the pane drives (play,
pause, seek, volume, speed, frame step, loop), and decodes audio to a
waveform shown with the cover art and the position; hardware decoders are
whatever GStreamer ranks highest. Playback starts on demand and stops when
the document is dropped. All of that sits behind the `gstreamer` feature,
which needs the GStreamer development files to build; without the feature,
without the runtime, or without the plug-in a file needs, the preview shows
the metadata and says what is missing. It is a plugin (`cdylib`, loaded
from `plugins/preview-media/`) and an `rlib` for the binary's opt-in
`preview-media` feature. It depends on `ext` and `types` only; `rick`
(behind the feature) and `tests` link it, and the plugin host loads it.
