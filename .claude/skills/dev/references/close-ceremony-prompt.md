# Close-ceremony prompt template

The message you send at the end of Step 4, **after the final phase is implemented and committed**.
It (a) confirms the plan landed, (b) hands a complete brief to `architect`, and (c) gets the user
to start a **fresh** session — architect needs a clean context window for the review.

## Why a fresh session

Review is qualitatively different from implementation: it compares code against plan + ADRs,
checks layering and real-time safety, reads test bodies across the whole plan's changes. That's
hard in a context already full of implementation reasoning and tool output. A fresh session forces
architect to re-read the plan and code with reviewer's eyes.

## The template

Replace `<…>` with the plan's actuals. Keep the structure — architect is tuned to receive this shape.

```
Plan implemented and committed. Ready for the architect close ceremony.

**Plan:** <plan-number> — <plan-title>  (`docs/plans/<NNNN-slug>.md`)
**Phases shipped:** <count> (<phase-1-name>, <phase-2-name>, …)
**Commits made this session:**
<paste output of: git log --oneline -n <count>>

**Done-when results (final phase):**
- [<pass|fail>] <criterion 1 verbatim from the plan>
- [<pass|fail>] <criterion 2 verbatim from the plan>

**Notes for architect** *(optional — only if relevant)*:
- <any deviation from the plan and why, with user approval noted>
- <any underspecified spot you filled with a judgment call>
- <any followup you noticed but didn't act on>

---

**Next step:** start a fresh session and invoke `/architect` with this brief. Architect will:

1. Review the whole plan against the ADRs and deliver it **in-conversation** (no review file).
2. Flip the plan's `Status:` to `done`.
3. `git mv` the plan to `docs/plans/done/<NNNN-slug>.md` and refresh the plan + ADR indexes.
4. Bump the application version for this plan (`cargo-release`, one bump per plan, no push — the
   most-forgotten close step; see `docs/releasing.md`).

After the review, push the commits and the new `vX.Y.Z` tag when you're ready.
```

## Filling it in

- **Plan identifier**: verbatim from the plan header — don't paraphrase the title.
- **Phases shipped**: count + one-line list, using the plan's phase names.
- **Commits**: `git log --oneline -n <N>` where `<N>` is the commits you just made; paste it.
- **Done-when results**: copy each criterion from the **final phase's** done-when, prefix
  `[pass]`/`[fail]`. If anything is `[fail]` you shouldn't be at Step 4 — fix it or escalate. The
  only legitimate non-pass is `[skipped — explicitly approved by user]` with a reason. Surface any
  earlier-phase failures in Notes.
- **Notes**: short. Architect re-reads the plan/ADRs/code; give the *deltas*, not a recap.

## What NOT to include

- Don't paste the full diff — architect reads files.
- Don't self-review ("looks good to me") — architect judges.
- Don't rehash reasoning from this session — the fresh session is fresh; the brief is the bridge.
- No secrets or credentials in the brief or any commit message.
