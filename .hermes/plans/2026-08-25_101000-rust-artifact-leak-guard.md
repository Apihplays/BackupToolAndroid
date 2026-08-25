# Rust-Artifact Leak Into Phone: Root-Cause Guard Plan

> **For Hermes:** Use subagent-driven-development skill to implement this plan task-by-task.

**Goal:** Determine how cargo build artifacts ended up in `/sdcard/Android/media/com.whatsapp/` on the veux, prevent recurrence, and make andpull refuse to back up such junk if it ever appears again.

**Architecture:** Three layers. (1) Forensic reproduction: attempt to reproduce the leak under controlled conditions to confirm the mechanism. (2) Environment guardrail: detect-and-warn when any tool session has `CARGO_TARGET_DIR` or cwd pointing at a device-mounted path. (3) In-app defense: profile scanner flags build-artifact patterns in source dirs before pulling them as "backup data".

**Tech Stack:** Existing Rust stack; bash for environment probe. No new dependencies.

---

## Current Context (forensic findings, 2026-08-25)

**What was found on the phone:** 2,205 files matching cargo's `target/` directory layout exactly:
- `.rustc_info.json`, `.cargo-lock`, `.cargo-build-lock`, `.cargo-artifact-lock`
- ~1,800 `*.o` object files with cargo's random names
- ~388 40-char hex files (cargo fingerprints)
- Stray copy of `.hermes/plans/2026-08-25_010500-cleanflash-backup-profiles.md`, `.gitignore`, `test-bin-andpull`, `TAG`, `shallow`, `root-output`, `run-build-script-build-script-build`, `rustix_test_can_compile`

**Timeline correlation:** artifact mtimes 01:16–02:10 (+08) = precisely the Codebuff/freebuff session doing ce9c3bc work (183 cargo commands: test×77, check×60, clippy×40).

**What the logs EXCLUDE:**
- Freebuff chat/log JSONL contains NO `adb push`, NO `CARGO_TARGET_DIR=` string, NO rsync/scp/cp targeting `/sdcard`.
- No cargo config overrides (`~/.cargo/config.toml`, repo `.cargo/config.toml` absent).
- No MTP/FUSE phone mounts on the Linux box.
- Termux IS installed on the phone (uid u0_a274) but artifacts showed u0_a216 (WhatsApp) — unreliable evidence due to FUSE ownership attribution inside `Android/media/<pkg>`.

**Honest status: mechanism NOT definitively proven.** Leading hypothesis: a tool-session environment leak where one shell invocation had `CARGO_TARGET_DIR` (or cwd) resolving to the phone through some path we haven't found in the logs (freebuff may not log every subprocess env). The presence of `rustix_test_can_compile` and `run-build-script-build-script-build` (build-script outputs) proves a REAL `cargo build/test` wrote there directly — these files are never copied by backup tools; they're only produced by cargo running with that output dir.

**Key deduction:** since cargo definitely executed with its target dir at that phone path (build-script outputs can't get there otherwise), and freebuff's *logged* commands don't show it, the setter was either (a) an unlogged env var in freebuff's execution sandbox, (b) a shell config sourced during that session, or (c) a command whose full text isn't captured in the JSONL we searched. Reproduction attempt must cover all three.

## Proposed approach

### Task 1: Controlled reproduction attempts
**Files:** none (investigation only)
1. Check bash/zsh history + all `~/.config/manicode/projects/*/chats/*/log.jsonl` for `CARGO_TARGET_DIR`, `cd /sdcard`, `adb shell cd`, `TERM=` anomalies around 01:00–02:15.
2. Grep freebuff binary strings for `Android/media` and `CARGO_TARGET_DIR` defaults: `strings ~/.config/manicode/freebuff | grep -iE 'android/media|CARGO_TARGET'`.
3. Check whether freebuff sandboxes runs with a shared `TMPDIR` or workspace that could have been bind-mounted/symlinked: inspect `~/.config/manicode/settings.json`, `freebuff-instance-owner.json`, any `projects/*/workspace*` dirs.
4. Test hypothesis (b): run `bash -l -i -c 'env | grep -iE "cargo|target"'` to see if any login-shell rc exports something odd.
5. Document findings — even negative results — in this plan file's appendix.

### Task 2: Session environment guardrail script
**Files:** Create `scripts/env-guard.sh`; Modify `.gitignore` (nothing needed).
1. Script prints FAIL when `CARGO_TARGET_DIR` or `PWD` contains `Android/media|/sdcard|/storage/emulated` OR when `$HOME` looks like an app sandbox.
2. Wire into preflight: source it from any future manual test runs; optionally suggest as git pre-commit hook.
3. Test: positive case (set var, expect fail exit 1), negative case (clean env, exit 0).

### Task 3: In-app junk guard for profile sources
**Files:** Modify `src/profile/mod.rs` (add pattern list), `src/profile/runner.rs` (apply filter + warn).
1. Failing tests first: `is_build_artifact(name)` returns true for `foo.o`, `.rustc_info.json`, `.cargo-lock`, `test-bin-andpull`, `<40-hex>` (exactly 40 lowercase hex chars), false for `IMG_20260727_111036.jpg`, `msgstore.db.crypt15`, `.nomedia`.
2. Add `pub const JUNK_PATTERNS: &[&str] = &["*.o", "*.rlib", "*.rmeta", ".rustc_info.json", ".cargo-lock", ".cargo-build-lock", ".cargo-artifact-lock", "target"]` plus hex-fingerprint detection fn.
3. In runner's scan step: count junk matches per source root; if > 20 junk files detected at top level, emit warning via ProfileOutcome warning field AND skip them from transfer (never silently pull garbage).
4. Surface in CLI summary: `[warn] com.whatsapp: skipped 2205 build artifacts`.
5. Tests green, clippy clean. Commit: `feat(profile): skip build-artifact junk in profile sources`.

### Task 4: Freebuff instruction note (the "fix using freebuff later" ask)
**Files:** Create `.github/AGENT_RULES.md` (or append to existing AGENTS.md if present).
1. One-page rule sheet for AI coding agents working in this repo:
   - NEVER set CARGO_TARGET_DIR outside the repo.
   - Never run cargo with cwd on removable/device paths.
   - Device testing only via documented andpull commands with dest under `./test_output/`.
   - Before any `git commit --signoff`, verify `git status` shows no unexpected new top-level files.
2. Commit: `docs: agent rules preventing device-path build leaks`.

## Files likely to change
- Create: `scripts/env-guard.sh`, `.github/AGENT_RULES.md`
- Modify: `src/profile/mod.rs`, `src/profile/runner.rs`, possibly `src/cli.rs` (warning print)

## Tests / validation
- Unit: `is_build_artifact` matrix (Task 3 step 1).
- Offline integration: seed a temp dir with fake artifacts + real photos, run scan, assert photos selected & artifacts skipped+counted.
- Live: rerun whatsapp profile on veux after cleanup — expect zero junk warnings (dir already cleaned manually today).
- Env guard: positive/negative shell tests.

## Risks / tradeoffs / open questions
- **Root cause may stay unproven** if freebuff's sandbox hides env from its own logs. Acceptable: guards in Tasks 2-4 prevent recurrence regardless of mechanism. Do NOT let perfect forensics block defense.
- Hex-fingerprint heuristic could false-positive on legit 40-char-hex-named media files — rare; acceptable, and warning (not silent skip) covers it.
- Open question for user: does freebuff expose an env-injection setting you may have configured once (settings.json is tiny — checked, nothing)? If you recall running anything manually around 01:16, say so.
