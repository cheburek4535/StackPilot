# Cross-Platform Refactor Handoff

## Baseline Commit

- **SHA**: `742d594ddbb66e19c8753110a5472d30279e8f1b`
- **Message**: `docs: initialize cross-platform refactor baseline`
- **Files**: `docs/CROSS_PLATFORM_PROGRESS.md`, `docs/CROSS_PLATFORM_HANDOFF.md`

## Architecture Decision Record (ADR)

**Decision**: Implement cross-platform support via `PlatformAdapter` trait with OS-specific adapters.

**Rationale**:
- Existing codebase already has `PlatformAdapter` trait and OS-specific modules
- Windows is the current developed/tested platform
- Linux/macOS adapters exist but are not fully tested
- This approach preserves Windows behavior while enabling Linux/macOS

**Tradeoffs**:
- Requires native CI on all three OSes (compilation only is not sufficient)
- Windows regression tests are mandatory for any production code change
- PATH persistence differs between Windows (registry) and Unix (rc-file markers)

## Source File Modifications Per Phase

### Session 0 (Current)
- **No production code changes** — audit and documentation only
- `docs/CROSS_PLATFORM_PROGRESS.md` — created
- `docs/CROSS_PLATFORM_HANDOFF.md` — created

### Session 1 (Future)
- `src-tauri/src/modules/toolchain/platforms/mod.rs` — enforce `PlatformAdapter` trait
- `src-tauri/src/modules/toolchain/platforms/linux.rs` — Linux adapter
- `src-tauri/src/modules/toolchain/platforms/macos.rs` — macOS adapter
- `src-tauri/.github/workflows/ci.yml` — Linux/macOS CI pipeline

### Session 2 (Future)
- `src-tauri/src/modules/toolchain/platforms/unix_rc.rs` — PATH write backport
- `src-tauri/src/modules/toolchain/platforms/unix_rc.rs` — rc-file write tests

### Session 3 (Future)
- `src-tauri/src/modules/workspace/process_manager.rs` — Windows process group kill
- `src-tauri/src/modules/workspace/process_manager.rs` — toolchain workflow tests

### Session 4 (Future)
- `src-tauri/.github/workflows/ci.yml` — Linux/macOS native CI
- `docs/CROSS_PLATFORM_PROGRESS.md` — documentation updates

### Session 5 (Future)
- `src-tauri/src/modules/toolchain/platforms/tests/` — cross-platform acceptance tests
- `src-tauri/tests/` — regression suite

## Build/Test Commands and Results

### Commands Run
```bash
cargo fmt --check  # Failed — pre-existing formatting diffs
cargo check        # Passed — warnings only
cargo test         # Failed — 682 passed, 1 failed, 1 ignored
```

### Results Summary
- **cargo fmt --check**: FAILED
  - Pre-existing diffs in `core/models.rs`, `core/settings.rs`, `toolchain/commands.rs`
  - Not caused by this session

- **cargo check**: PASSED
  - Warnings: unused imports, dead code
  - No errors

- **cargo test**: FAILED
  - 682 passed
  - 1 failed: `python_health_checks_do_not_include_pip_assertion` (pre-existing)
  - 1 ignored: `live_repo_resolution_and_parsing` (network-dependent)

## Risks

1. **Windows regression risk**: Any production code change could break Windows behavior
2. **Native CI required**: Compilation on all three OSes does NOT guarantee full support
3. **PATH persistence differences**: Windows uses registry, Unix uses rc-file markers
4. **Process kill differences**: May need process group handling on Unix
5. **Pre-existing test failure**: `python_health_checks_do_not_include_pip_assertion` must be fixed or skipped

## Missing Coverage

1. **Linux/macOS CI**: No CI pipeline for Linux or macOS yet
2. **Native runtime tests**: No tests for actual process execution on Linux/macOS
3. **PATH write tests**: No tests for rc-file marker behavior on Unix
4. **Process group kill**: No tests for process group handling on Unix
5. **Acceptance tests**: No cross-platform acceptance tests

## Instructions for Later Agents

1. **Update this document**: When completing a session, update:
   - Phase checklist in `CROSS_PLATFORM_PROGRESS.md`
   - Cross-platform matrix in `CROSS_PLATFORM_PROGRESS.md`
   - Source file modifications in this document
   - Build/test commands and results in this document
   - Risks and missing coverage

2. **Commit messages**: Use format `docs: update cross-platform refactor progress (Session N)`

3. **Validation before commit**: Always run:
   ```bash
   cargo fmt --check
   cargo check
   cargo test
   ```

4. **Windows regression**: Any production code change MUST include a Windows regression test

5. **Native CI**: Linux/macOS support is NOT complete until CI runs on the actual OS

6. **Pre-existing failures**: The `python_health_checks_do_not_include_pip_assertion` test failure is pre-existing. If fixing it, document the fix. If skipping, document why.

## Key Findings from Session 0

### OS-Specific Behaviors Identified
1. **DevLauncher RunCommand**: `launch_engine.rs:51-55` — uses `StdCommand` directly without shell wrapping
2. **DevLauncher ExecuteScript**: `launch_engine.rs` — `shell: Option<String>` — no Unix default shell logic
3. **Process kill**: `process_manager.rs` — uses `child.kill()` from std
4. **Toolchain installer**: `installer.rs:22` — Windows-first, Linux/macOS tasks marked Skipped
5. **Console process running**: `console.rs:17` — Windows-first
6. **PATH persistence**: `unix_rc.rs` — uses marker-delimited block `# StackPilot:begin`/`# StackPilot:end`

### Platform Adapters
- `platforms/mod.rs` — `PlatformAdapter` trait
- `platforms/windows.rs` — Windows (winget, registry, PowerShell)
- `platforms/linux.rs` — Linux (apt/dnf/pacman)
- `platforms/macos.rs` — macOS (brew)

### Project Creator
- `engine/process.rs` — cross-platform `TokioCommand` with Windows (exe/.cmd/.bat) vs Unix (`sh -c`)
- `validate.rs:10` — OS platform check
