# Plans index

The one-minute "what's in flight" view. Read this first each session instead of
re-deriving state from `git log`. Completed plans move to `done/`.

**Next free number: 0002**

## Active roster

| Plan | Title                                   | Status | Summary |
|------|-----------------------------------------|--------|---------|
| [0001](0001-core-and-standalone-mvp.md) | Core + standalone MVP, then foobar parity | draft  | Workspace → Win loopback → DSP → wgpu spectrum → scenes → C ABI → foobar plugin → mac capture. |

## Conventions

- **Numbering:** sequential, zero-padded 4 digits. Take the next free number above, then
  bump it here in the same session.
- **Phases:** ordered, each one commit, each tagged `**Owner area:**` (`core` / `standalone`
  / `plugin` / `human`). Area tags pre-name the skills this project would create if it grows
  from the lightweight harness into a full skill ecosystem.
- **Lifecycle:** `draft` → `in-progress` → `done` (then `git mv` to `done/` and drop from
  this roster). Review happens at plan end, in a fresh context — not by the session that
  wrote the code.
