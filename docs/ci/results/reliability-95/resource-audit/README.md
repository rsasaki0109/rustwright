# BiDi resource audit

Three controlled-peer regressions retain page handles and acknowledge local close commands without emitting context-destroy events. All three fail before the fix because their worker tasks remain live; after the fix, the full 111-test BiDi suite and scoped Clippy pass. Task `is_finished()` is observed without calling abort, so dropping the retained handles cannot hide the failure.

`before/` preserves original source bytes, with separately recorded test-enabled snapshots; `after/` preserves fixed source bytes. The two added regression test files have identical before/after bytes. `production-and-tests.patch`, `report.json`, and `replay-commands.txt` document scope and limits. Session subscription ownership is unchanged. Unknown remote subscription IDs from lost acknowledgements are not claimed reclaimable.

MSRV and native checks are deferred during the parent independent memory measurement; no memory plateau or heap leak claim follows from these task-lifetime checks.

`SHA256SUMS` covers every actual evidence file except itself. Baseline executable SHA is recorded without copying the executable.
