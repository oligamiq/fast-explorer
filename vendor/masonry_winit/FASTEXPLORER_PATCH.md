# FastExplorer patch

This is `masonry_winit 0.4.0` with one local renderer change in `src/vello_util.rs`:
- normal adapter selection is unchanged;
- if adapter selection fails, wgpu is retried with `force_fallback_adapter = true`;
- if a normal adapter is found but device creation fails, FastExplorer also retries with the fallback/software adapter;
- `FASTEXPLORER_FORCE_CPU=1` requests the fallback/software adapter directly.

The patch avoids mutating process environment variables after graphics-driver initialization and applies equally to Linux, Windows, and Android.
