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
- **Phases:** ordered, each one commit, each tagged `**Owner skill:**` with one value from the
  vocabulary `dev` (all code) or `human` (a task only the user can do). The `dev` skill reads
  this tag at the start of each phase; a missing tag is a Mode 4 review blocker. An optional
  `**Area:**` note (`core` / `standalone` / `plugin`) orients the reader but is not the tag.
- **Skills:** `architect` designs and owns `docs/`; `dev` implements all code. `architect`
  writes and closes plans; `dev` flips `draft → in-progress` at "go" and nothing else in the file.
- **Lifecycle:** `draft` → `in-progress` → `done` (then `git mv` to `done/` and drop from
  this roster). Review happens at plan end, in a fresh `/architect` session — not by the
  session that wrote the code.
