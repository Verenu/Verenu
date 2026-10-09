# Demo video source

Editable source for Verenu's 60-second product demo. It has music and no
narration. See [STORYBOARD.md](STORYBOARD.md) for the shot list and timing.

The rendered MP4 is not committed. To rebuild it from this checkout:

```bash
demo-video/build.sh            # writes to ~/.cache/verenu-demo-video/<timestamp>/
```

Requirements: Node with this repo's dev dependencies, Google Chrome at
`/usr/bin/google-chrome-stable` (or set `CHROME_PATH`), and `ffmpeg` with
libx264. A full build takes about 4 minutes. Rendering is about 2.5 minutes of
that.

## Pipeline

| File | Role |
| --- | --- |
| `capture-ui.mjs` | Opens the real UI in browser preview mode with seeded sample data and screenshots the Home, Insights, Style, Contexts, and Settings pages at 2×. |
| `capture-pill.mjs` | Renders the real dictation pill (`pill.html`) through a stub Tauri event bridge and captures a timed 30 fps transparent PNG sequence. |
| `music.mjs` | Synthesizes the original music bed as a WAV. It is deterministic and uses no samples. |
| `stage.html` | The composition. `demo.render(t)` draws any moment as a pure function of time. |
| `render.mjs` | Serves the stage, steps through all 1,800 frames, and pipes them to ffmpeg with the music. `--stills t1,t2` writes preview PNGs instead. |
| `build.sh` | Runs the above with a private Vite server and a per-job `TMPDIR`. |

## Editing

- **Timing:** change the scene ranges in `stage.html` (`S`) and update
  STORYBOARD.md. Keep cuts on 2.4 s bar lines so they stay on the beat.
- **Copy:** headlines and cards live in `stage.html`. Keep claims to what
  README.md and the app actually say.
- **Screens:** edit the seed data or the page list in `capture-ui.mjs`.
  Highlight rectangles in `stage.html` use the screenshot's 1440×900 CSS
  pixels.
- **Preview a frame:** run `node demo-video/render.mjs <work-dir> <out-dir> --stills 8,20,36`.

All assets are self-generated or come from this repository: the UI, the
brand mark, and the OFL fonts bundled through `@fontsource-variable`.
