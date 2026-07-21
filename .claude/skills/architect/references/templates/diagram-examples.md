# Diagram examples — light-music-visualizer

Project-specific mermaid patterns. Copy one and adapt. Keep diagrams small (<~12 nodes) and use
`subgraph` to mark boundaries — what's inside `core/`, what's a shell, what's external.

## Component / data-flow map

The canonical picture: audio in (from either source) → ring → DSP → scenes → wgpu → screen.

```mermaid
flowchart LR
  subgraph ext[External sources]
    wasapi[WASAPI loopback]
    sck[ScreenCaptureKit / BlackHole]
    fb[foobar visualisation_stream]
  end
  subgraph shells[Shells]
    sa[standalone: winit + capture]
    pl[foobar plugin: C++ shim]
  end
  subgraph core[core/ - Rust]
    ring[[SPSC ring buffer]]
    dsp[DSP: FFT + onset/beat]
    scenes[Scene graph]
    wgpu[wgpu layer]
  end
  wasapi --> sa
  sck --> sa
  fb --> pl
  sa --> ring
  pl -->|C ABI| ring
  ring --> dsp --> scenes --> wgpu --> screen([screen])
```

## Cross-boundary sequence (plugin path)

Use a `sequenceDiagram` when the interesting thing is the order of calls across the C ABI.

```mermaid
sequenceDiagram
  participant FB as foobar2000 (C++ host)
  participant Shim as plugin-foobar (C++)
  participant Core as core (Rust, via C ABI)
  FB->>Shim: visualisation_stream chunk
  Shim->>Core: lmv_push_samples(handle, ptr, len, rate, ch)
  Note over Core: copy into ring buffer, return immediately
  FB->>Shim: paint(hwnd, dt)
  Shim->>Core: lmv_render(handle, target)
  Core-->>Shim: ok / error code
```

## Scene lifecycle

Use `stateDiagram-v2` for things with explicit states.

```mermaid
stateDiagram-v2
  [*] --> Active
  Active --> Active: update(AnalysisFrame) / render
  Active --> Switching: user cycles scene
  Switching --> Active: next scene init done
  Active --> [*]: shutdown
```
