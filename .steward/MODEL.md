# MODEL

> Stewardry's grounded understanding of this project. Regenerate with `stew understand`.

- **Updated:** 2026-06-14T21:36:40Z

## Understanding

Noter is, on disk, a planning-stage Rust project whose ONLY source file is a 30-line `src/main.rs` that prints a 'planning skeleton' marker — there is no editor yet. The repo's real substance is its documentation (README/REQUIREMENTS/DESIGN/ROADMAP/RIGOROUS_REVIEW plus docs/), which `main.rs` explicitly declares to be 'part of the product.' Critically, the supplied `.steward/MODEL.md` structure and the symbol repo-map describe a large, fully-implemented and heavily-tested codebase (src/core/document.rs, atomic_io.rs, recovery.rs, search.rs, src/app.rs, src/ui/md_syntax.rs, fuzz/, golden/, mutants.out/) that does NOT exist in this checkout — those are stale/aspirational. The intended design is an egui/eframe GUI with an egui-agnostic reliability core (atomic saves, recovery, line-ending/BOM fidelity), but none of it is built.

## Key components

- `src/main.rs` — the sole compiled artifact: a deliberate Phase-0 marker that prints version/status and documents the planned (not-yet-existing) module layout (app.rs, core/*, ui/*, platform/*). This is the entire current program. - _src/main.rs_
- `Cargo.toml` — defines product identity, release profile (lto, codegen-units=1, strip, panic=abort, opt-level=3), forbids unsafe, and enables clippy pedantic/nursery/cargo groups; but `[dependencies]` is EMPTY and every real dep (egui, ropey, rfd, serde, etc.) is only commented as 'planned by phase.' - _Cargo.toml_
- Planning documents at repo root + docs/ (DESIGN.md, REQUIREMENTS.md, ROADMAP.md, README.md, RIGOROUS_REVIEW.md, docs/README.md, docs/manual-test-matrix.md) — these ARE the deliverable at this stage, per the main.rs philosophy comment, not just supporting material. - _src/main.rs; DESIGN.md; ROADMAP.md; docs/manual-test-matrix.md_

## Load-bearing invariants

- The core editing/I-O logic must stay completely independent of egui (UI-framework-agnostic core). This is stated as a hard maintainer rule and underpins the intended testability of the reliability layer. - _Cargo.toml_
- `unsafe_code = "forbid"` and `panic = "abort"` in release are load-bearing safety/size commitments baked into the manifest; code that needs unsafe or unwinding would break the build's lint contract and the documented reliability stance. - _Cargo.toml_
- Dependencies are introduced gradually and only with written justification (DESIGN.md + `cargo tree -i` + binary-size measurement at phase gates); the empty dependency set and lockfile are an intentional baseline, not an oversight. - _Cargo.toml; Cargo.lock_

## Footguns

- The biggest trap: `.steward/MODEL.md`, the symbol repo-map, and the listed file tree describe an extensive implemented+tested codebase (core modules, fuzz targets, golden files, mutants.out, proptest-regressions) that is absent from the actual working tree — `find` shows `src/main.rs` is the only .rs file. Any agent or newcomer that trusts that map will hallucinate functions (e.g. `Document::save_atomic`, `md_syntax_layouter`, `find_all`) and 'edit' code that does not exist. - _src/main.rs; Cargo.lock; .steward/MODEL.md_
- This checkout is a git worktree (`.git` is a gitdir pointer to `C:/GitHub/noter/.git/worktrees/...`), so it shares history/branches with the parent repo; destructive git operations or branch assumptions here can affect the main working copy and are not as isolated as a normal clone. - _.git_
- Adding the first real feature is high-risk precisely because everything (atomic-save behavior, line-ending/BOM fidelity, recovery) is specified in prose only — there is no test harness, no dependencies, and no reference implementation to diff against, so the documented invariants are currently unenforced by any code or CI in this tree. - _Cargo.toml; REQUIREMENTS.md; src/main.rs_

## Structure

```
.gitignore
docs/
docs\DEV_STANDARDS.md
docs\README.md
docs\SECURITY.md
docs\adr/
docs\adr\001-panic-abort.md
docs\manual-test-matrix.md
docs\options-minimalism-design.md
docs\testing-strategy.md
fonts/
fonts\README.txt
fuzz/
fuzz\Cargo.toml
fuzz\fuzz_targets/
fuzz\fuzz_targets\README.md
fuzz\fuzz_targets\document_bytes.rs
golden/
golden\sample_bom_crlf.txt
golden\sample_crlf.txt
golden\sample_lf.txt
launch_crash.log
launch_crash.log.err
mutants.out/
mutants.out\caught.txt
mutants.out\debug.log
mutants.out\diff/
mutants.out\diff\src__core__search.rs_line_24_col_25.diff
mutants.out\diff\src__core__search.rs_line_24_col_30.diff
mutants.out\diff\src__core__search.rs_line_24_col_5.diff
mutants.out\diff\src__core__search.rs_line_24_col_5_001.diff
mutants.out\diff\src__core__search.rs_line_29_col_5.diff
mutants.out\lock.json
mutants.out\log/
mutants.out\log\baseline.log
mutants.out\log\src__core__search.rs_line_24_col_25.log
mutants.out\log\src__core__search.rs_line_24_col_30.log
mutants.out\log\src__core__search.rs_line_24_col_5.log
mutants.out\log\src__core__search.rs_line_24_col_5_001.log
mutants.out\log\src__core__search.rs_line_29_col_5.log
mutants.out\missed.txt
mutants.out\mutants.json
mutants.out\outcomes.json
mutants.out\timeout.txt
mutants.out\unviable.txt
mutants.out.old/
mutants.out.old\caught.txt
mutants.out.old\debug.log
mutants.out.old\diff/
mutants.out.old\diff\src__core__atomic_io.rs_line_125_col_5.diff
mutants.out.old\diff\src__core__atomic_io.rs_line_129_col_12.diff
mutants.out.old\diff\src__core__atomic_io.rs_line_129_col_43.diff
mutants.out.old\diff\src__core__atomic_io.rs_line_129_col_46.diff
mutants.out.old\diff\src__core__atomic_io.rs_line_222_col_5.diff
mutants.out.old\diff\src__core__atomic_io.rs_line_242_col_5.diff
mutants.out.old\diff\src__core__atomic_io.rs_line_242_col_5_001.diff
mutants.out.old\diff\src__core__atomic_io.rs_line_242_col_5_002.diff
mutants.out.old\diff\src__core__atomic_io.rs_line_243_col_19.diff
mutants.out.old\diff\src__core__atomic_io.rs_line_243_col_19_001.diff
mutants.out.old\diff\src__core__atomic_io.rs_line_243_col_19_002.diff
mutants.out.old\diff\src__core__atomic_io.rs_line_243_col_25.diff
mutants.out.old\diff\src__core__atomic_io.rs_line_243_col_25_001.diff
mutants.out.old\diff\src__core__atomic_io.rs_line_243_col_32.diff
mutants.out.old\diff\src__core__atomic_io.rs_line_243_col_32_001.diff
mutants.out.old\diff\src__core__document.rs_line_100_col_5.diff
mutants.out.old\diff\src__core__document.rs_line_100_col_5_001.diff
mutants.out.old\diff\src__core__document.rs_line_124_col_9.diff
mutants.out.old\diff\src__core__document.rs_line_141_col_9.diff
mutants.out.old\diff\src__core__document.rs_line_172_col_9.diff
mutants.out.old\diff\src__core__document.rs_line_172_col_9_001.diff
mutants.out.old\diff\src__core__document.rs_line_172_col_9_002.diff
mutants.out.old\diff\src__core__document.rs_line_174_col_29.diff
mutants.out.old\diff\src__core__document.rs_line_192_col_9.diff
mutants.out.old\diff\src__core__document.rs_line_207_col_9.diff
mutants.out.old\diff\src__core__document.rs_line_216_col_9.diff
mutants.out.old\diff\src__core__document.rs_line_223_col_9.diff
mutants.out.old\diff\src__core__document.rs_line_223_col_9_001.diff
mutants.out.old\diff\src__core__document.rs_line_239_col_9.diff
mutants.out.old\diff\src__core__document.rs_line_239_col_9_001.diff
mutants.out.old\diff\src__core__document.rs_line_250_col_9.diff
mutants.out.old\diff\src__core__document.rs_line_253_col_18.diff
mutants.out.old\diff\src__core__document.rs_line_263_col_9.diff
mutants.out.old\diff\src__core__document.rs_line_263_col_9_001.diff
mutants.out.old\diff\src__core__document.rs_line_272_col_9.diff
mutants.out.old\diff\src__core__document.rs_line_272_col_9_001.diff
mutants.out.old\diff\src__core__document.rs_line_272_col_9_002.diff
mutants.out.old\diff\src__core__document.rs_line_272_col_9_003.diff
mutants.out.old\diff\src__core__document.rs_line_275_col_20.diff
mutants.out.old\diff\src__core__document.rs_line_275_col_20_001.diff
mutants.out.old\diff\src__core__document.rs_line_37_col_9.diff
mutants.out.old\diff\src__core__document.rs_line_37_col_9_001.diff
mutants.out.old\diff\src__core__document.rs_line_37_col_9_002.diff
mutants.out.old\diff\src__core__document.rs_line_45_col_9.diff
mutants.out.old\diff\src__core__document.rs_line_45_col_9_001.diff
mutants.out.old\diff\src__core__document.rs_line_55_col_9.diff
mutants.out.old\diff\src__core__document.rs_line_57_col_18.diff
mutants.out.old\diff\src__core__document.rs_line_63_col_18.diff
mutants.out.old\diff\src__core__document.rs_line_85_col_9.diff
mutants.out.old\diff\src__core__document.rs_line_85_col_9_001.diff
mutants.out.old\diff\src__core__recovery.rs_line_114_col_5.diff
mutants.out.old\diff\src__core__recovery.rs_line_114_col_5_001.diff
mutants.out.old\diff\src__core__recovery.rs_line_120_col_46.diff
mutants.out.old\diff\src__core__recovery.rs_line_120_col_51.diff
mutants.out.old\diff\src__core__recovery.rs_line_120_col_58.diff
mutants.out.old\diff\src__core__recovery.rs_line_120_col_63.diff
mutants.out.old\diff\src__core__recovery.rs_line_120_col_70.diff
mutants.out.old\diff\src__core__recovery.rs_line_120_col_75.diff
mutants.out.old\diff\src__core__recovery.rs_line_136_col_5.diff
mutants.out.old\diff\src__core__recovery.rs_line_136_col_5_001.diff
mutants.out.old\diff\src__core__recovery.rs_line_146_col_5.diff
mutants.out.old\diff\src__core__recovery.rs_line_182_col_5.diff
mutants.out.old\diff\src__core__recovery.rs_line_189_col_21.diff
mutants.out.old\diff\src__core__recovery.rs_line_192_col_21.diff
mutants.out.old\diff\src__core__recovery.rs_line_203_col_22.diff
mutants.out.old\diff\src__core__recovery.rs_line_216_col_5.diff
mutants.out.old\diff\src__core__recovery.rs_line_235_col_13.diff
mutants.out.old\diff\src__core__recovery.rs_line_235_col_5.diff
mutants.out.old\diff\src__core__recovery.rs_line_249_col_13.diff
mutants.out.old\diff\src__core__recovery.rs_line_259_col_5.diff
mutants.out.old\diff\src__core__recovery.rs_line_276_col_5.diff
mutants.out.old\diff\src__core__recovery.rs_line_276_col_5_001.diff
mutants.out.old\diff\src__core__recovery.rs_line_284_col_52.diff
mutants.out.old\diff\src__core__recovery.rs_line_287_col_61.diff
mutants.out.old\diff\src__core__recovery.rs_line_287_col_61_001.diff
mutants.out.old\diff\src__core__recovery.rs_line_287_col_61_002.diff
mutants.out.old\diff\src__core__recovery.rs_line_310_col_5.diff
mutants.out.old\diff\src__core__recovery.rs_line_310_col_5_001.diff
mutants.out.old\diff\src__core__recovery.rs_line_310_col_5_002.diff
mutants.out.old\diff\src__core__recovery.rs_line_325_col_5.diff
mutants.out.old\diff\src__core__recovery.rs_line_335_col_5.diff
mutants.out.old\diff\src__core__recovery.rs_line_335_col_5_001.diff
mutants.out.old\diff\src__core__recovery.rs_line_335_col_5_002.diff
mutants.out.old\diff\src__core__recovery.rs_line_336_col_8.diff
mutants.out.old\diff\src__core__recovery.rs_line_348_col_21.diff
mutants.out.old\diff\src__core__recovery.rs_line_386_col_5.diff
mutants.out.old\diff\src__core__recovery.rs_line_396_col_17.diff
mutants.out.old\diff\src__core__recovery.rs_line_60_col_5.diff
mutants.out.old\diff\src__core__recovery.rs_line_64_col_8.diff
mutants.out.old\diff\src__core__recovery.rs_line_77_col_25.diff
mutants.out.old\diff\src__core__recovery.rs_line_78_col_25.diff
mutants.out.old\diff\src__core__recovery.rs_line_91_col_5.diff
mutants.out.old\lock.json
mutants.out.old\log/
mutants.out.old\log\baseline.log
mutants.out.old\log\src__core__atomic_io.rs_line_125_col_5.log
mutants.out.old\log\src__core__atomic_io.rs_line_129_col_12.log
mutants.out.old\log\src__core__atomic_io.rs_line_129_col_43.log
mutants.out.old\log\src__core__atomic_io.rs_line_129_col_46.log
mutants.out.old\log\src__core__atomic_io.rs_line_222_col_5.log
mutants.out.old\log\src__core__atomic_io.rs_line_242_col_5.log
mutants.out.old\log\src__core__atomic_io.rs_line_242_col_5_001.log
mutants.out.old\log\src__core__atomic_io.rs_line_242_col_5_002.log
mutants.out.old\log\src__core__atomic_io.rs_line_243_col_19.log
mutants.out.old\log\src__core__atomic_io.rs_line_243_col_19_001.log
mutants.out.old\log\src__core__atomic_io.rs_line_243_col_19_002.log
mutants.out.old\log\src__core__atomic_io.rs_line_243_col_25.log
mutants.out.old\log\src__core__atomic_io.rs_line_243_col_25_001.log
mutants.out.old\log\src__core__atomic_io.rs_line_243_col_32.log
mutants.out.old\log\src__core__atomic_io.rs_line_243_col_32_001.log
mutants.out.old\log\src__core__document.rs_line_100_col_5.log
mutants.out.old\log\src__core__document.rs_line_100_col_5_001.log
mutants.out.old\log\src__core__document.rs_line_124_col_9.log
mutants.out.old\log\src__core__document.rs_line_141_col_9.log
mutants.out.old\log\src__core__document.rs_line_172_col_9.log
mutants.out.old\log\src__core__document.rs_line_172_col_9_001.log
mutants.out.old\log\src__core__document.rs_line_172_col_9_002.log
mutants.out.old\log\src__core__document.rs_line_174_col_29.log
mutants.out.old\log\src__core__document.rs_line_192_col_9.log
mutants.out.old\log\src__core__document.rs_line_207_col_9.log
mutants.out.old\log\src__core__document.rs_line_216_col_9.log
mutants.out.old\log\src__core__document.rs_line_223_col_9.log
mutants.out.old\log\src__core__document.rs_line_223_col_9_001.log
mutants.out.old\log\src__core__document.rs_line_239_col_9.log
mutants.out.old\log\src__core__document.rs_line_239_col_9_001.log
mutants.out.old\log\src__core__document.rs_line_250_col_9.log
mutants.out.old\log\src__core__document.rs_line_253_col_18.log
mutants.out.old\log\src__core__document.rs_line_263_col_9.log
mutants.out.old\log\src__core__document.rs_line_263_col_9_001.log
mutants.out.old\log\src__core__document.rs_line_272_col_9.log
mutants.out.old\log\src__core__document.rs_line_272_col_9_001.log
mutants.out.old\log\src__core__document.rs_line_272_col_9_002.log
mutants.out.old\log\src__core__document.rs_line_272_col_9_003.log
mutants.out.old\log\src__core__document.rs_line_275_col_20.log
mutants.out.old\log\src__core__document.rs_line_275_col_20_001.log
mutants.out.old\log\src__core__document.rs_line_37_col_9.log
mutants.out.old\log\src__core__document.rs_line_37_col_9_001.log
mutants.out.old\log\src__core__document.rs_line_37_col_9_002.log
mutants.out.old\log\src__core__document.rs_line_45_col_9.log
mutants.out.old\log\src__core__document.rs_line_45_col_9_001.log
mutants.out.old\log\src__core__document.rs_line_55_col_9.log
mutants.out.old\log\src__core__document.rs_line_57_col_18.log
mutants.out.old\log\src__core__document.rs_line_63_col_18.log
mutants.out.old\log\src__core__document.rs_line_85_col_9.log
mutants.out.old\log\src__core__document.rs_line_85_col_9_001.log
mutants.out.old\log\src__core__recovery.rs_line_114_col_5.log
mutants.out.old\log\src__core__recovery.rs_line_114_col_5_001.log
mutants.out.old\log\src__core__recovery.rs_line_120_col_46.log
mutants.out.old\log\src__core__recovery.rs_line_120_col_51.log
mutants.out.old\log\src__core__recovery.rs_line_120_col_58.log
mutants.out.old\log\src__core__recovery.rs_line_120_col_63.log
mutants.out.old\log\src__core__recovery.rs_line_120_col_70.log
mutants.out.old\log\src__core__recovery.rs_line_120_col_75.log
mutants.out.old\log\src__core__recovery.rs_line_136_col_5.log
mutants.out.old\log\src__core__recovery.rs_line_136_col_5_001.log
mutants.out.old\log\src__core__recovery.rs_line_146_col_5.log
mutants.out.old\log\src__core__recovery.rs_line_182_col_5.log
mutants.out.old\log\src__core__recovery.rs_line_189_col_21.log
mutants.out.old\log\src__core__recovery.rs_line_192_col_21.log
mutants.out.old\log\src__core__recovery.rs_line_203_col_22.log
mutants.out.old\log\src__core__recovery.rs_line_216_col_5.log
mutants.out.old\log\src__core__recovery.rs_line_235_col_13.log
mutants.out.old\log\src__core__recovery.rs_line_235_col_5.log
mutants.out.old\log\src__core__recovery.rs_line_249_col_13.log
mutants.out.old\log\src__core__recovery.rs_line_259_col_5.log
mutants.out.old\log\src__core__recovery.rs_line_276_col_5.log
mutants.out.old\log\src__core__recovery.rs_line_276_col_5_001.log
mutants.out.old\log\src__core__recovery.rs_line_284_col_52.log
mutants.out.old\log\src__core__recovery.rs_line_287_col_61.log
mutants.out.old\log\src__core__recovery.rs_line_287_col_61_001.log
mutants.out.old\log\src__core__recovery.rs_line_287_col_61_002.log
mutants.out.old\log\src__core__recovery.rs_line_310_col_5.log
mutants.out.old\log\src__core__recovery.rs_line_310_col_5_001.log
mutants.out.old\log\src__core__recovery.rs_line_310_col_5_002.log
mutants.out.old\log\src__core__recovery.rs_line_325_col_5.log
mutants.out.old\log\src__core__recovery.rs_line_335_col_5.log
mutants.out.old\log\src__core__recovery.rs_line_335_col_5_001.log
mutants.out.old\log\src__core__recovery.rs_line_335_col_5_002.log
mutants.out.old\log\src__core__recovery.rs_line_336_col_8.log
mutants.out.old\log\src__core__recovery.rs_line_348_col_21.log
mutants.out.old\log\src__core__recovery.rs_line_386_col_5.log
mutants.out.old\log\src__core__recovery.rs_line_396_col_17.log
mutants.out.old\log\src__core__recovery.rs_line_60_col_5.log
mutants.out.old\log\src__core__recovery.rs_line_64_col_8.log
mutants.out.old\log\src__core__recovery.rs_line_77_col_25.log
mutants.out.old\log\src__core__recovery.rs_line_78_col_25.log
mutants.out.old\log\src__core__recovery.rs_line_91_col_5.log
mutants.out.old\missed.txt
mutants.out.old\mutants.json
mutants.out.old\outcomes.json
mutants.out.old\timeout.txt
mutants.out.old\unviable.txt
proptest-regressions/
proptest-regressions\app.txt
proptest-regressions\core/
proptest-regressions\core\atomic_io.txt
proptest-regressions\core\document.txt
qa-final-err.log
qa-final.log
qa-launch.log
src/
src\app/
src\app.rs
src\core/
src\core\atomic_io.rs
src\core\document.rs
src\core\mod.rs
src\core\recovery.rs
src\core\search.rs
src\error.rs
src\lib.rs
src\main.rs
src\platform/
src\ui/
src\ui\md_syntax.rs
src\ui\mod.rs
```
