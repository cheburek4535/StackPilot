# Cross-Platform Refactor Handoff

## Baseline Commit

- **SHA**: `742d594ddbb66e19c8753110a5472d30279e8f1b`
- **Message**: `docs: initialize cross-platform refactor baseline`
- **Files**: `docs/CROSS_PLATFORM_PROGRESS.md`, `docs/CROSS_PLATFORM_HANDOFF.md`

## Session 1 Commit

- **Message**: `refactor: add cross-platform command foundation`
- **Files**: See Source File Modifications below

## Architecture Decision Record (ADR)

**Decision**: Implement cross-platform support via `platform` module with typed abstractions, alongside the existing `PlatformAdapter` trait.

**Rationale**:
- `platform` module provides low-level building blocks (host OS, shell, command, env, paths) used by ALL modules
- Existing `PlatformAdapter` trait (toolchain/platforms) handles toolchain-specific installer logic
- Both layers compose: business modules use `platform` for command execution, toolchain uses `PlatformAdapter` for installer dispatch

**Tradeoffs**:
- Two platform abstraction layers coexist temporarily until `PlatformAdapter` is refactored to use `platform` types
- Windows regression tests are mandatory for any production code change
- PATH persistence differs between Windows (registry) and Unix (rc-file markers)

## Source File Modifications Per Phase

### Session 0
- **No production code changes** — audit and documentation only
- `docs/CROSS_PLATFORM_PROGRESS.md` — created
- `docs/CROSS_PLATFORM_HANDOFF.md` — created

### Session 1 (Current)
- `src-tauri/src/platform/mod.rs` — **NEW** module root with doc comments
- `src-tauri/src/platform/host.rs` — **NEW** `HostOs`/`HostArch` enums, `current_os()`, `current_arch()`
- `src-tauri/src/platform/shell.rs` — **NEW** `ShellKind` enum, `parse_shell()`, `default_shell_for_platform()`, `resolve_shell()`, `shell_executable()`
- `src-tauri/src/platform/command.rs` — **NEW** `build_tokio_command()`, `build_std_command()`, `infer_command_mode()`, quoting functions, Windows program resolution
- `src-tauri/src/platform/environment.rs` — **NEW** `EnvironmentOverlay` with `apply_std()`/`apply_tokio()`, PATH deduplication
- `src-tauri/src/platform/paths.rs` — **NEW** `paths_eq()`, `is_descendant()`, `resolve_executable()` (via `which` crate), batch file detection
- `src-tauri/src/lib.rs` — added `pub mod platform;`
- `src-tauri/src/modules/project_creator/engine/process.rs` — refactored `build_command()` to delegate to `platform::command`, helper functions now delegate to platform

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

### Session 1 Commands and Results
```bash
cargo fmt          # Passed (auto-formatted)
cargo fmt --check  # Passed
cargo check        # Passed (warnings only — pre-existing)
cargo test         # 744 passed, 9 failed (all pre-existing), 1 ignored
```

### Results Summary
- **cargo fmt**: PASSED
- **cargo fmt --check**: PASSED
- **cargo check**: PASSED (warnings only — pre-existing unused imports, dead code, non-snake-case FFI)
- **cargo test**: 744 passed, 9 failed (all pre-existing), 1 ignored

#### New Tests (70 platform tests)
All `platform::*` tests pass:
- `platform::host` (5 tests): OS/arch detection, display, consistency
- `platform::shell` (13 tests): parse, defaults, resolution, executable pairs, cross-platform validation
- `platform::command` (35 tests): shell validation, Windows batch detection, PowerShell heuristics, direct mode, quoting functions, script line building
- `platform::environment` (8 tests): overlay builder, PATH dedup (case-sensitive/insensitive), path_entry_eq, apply_std
- `platform::paths` (9 tests): paths_eq, is_descendant, executable resolution, suffix handling, batch detection

### Pre-existing Failures
- `python_health_checks_do_not_include_pip_assertion` — pip not in tools.json
- `runner_executes_absolute_executable_path` — env-dependent batch file test
- 6× `scaffold_*` tests — scaffold CLI environment issues
- `live_repo_resolution_and_parsing` — ignored (network-dependent)

## Key Capabilities Delivered (Session 1)

1. **Typed host platform**: `HostOs::{Windows,Linux,Macos}` — no raw OS string comparisons
2. **Typed shell model**: `ShellKind::{Default,Sh,Bash,Zsh,Cmd,PowerShell,Pwsh}` — safe parsing, platform-aware defaults
3. **Command construction**: Direct mode (args via `Command::arg(s)`, no string concat), Shell mode (shell + flag + script), Windows batch through `cmd /D /C`
4. **Executable resolution**: Centralized via `which` crate, handles Windows extensions
5. **Environment overlay**: `EnvironmentOverlay` with PATH prepend, env set/remove, platform-aware dedup
6. **Safe path comparison**: OS-aware slash normalization, case sensitivity per platform

## Risks

1. **Windows regression risk**: Any production code change could break Windows behavior
2. **Native CI required**: Compilation on all three OSes does NOT guarantee full support
3. **PATH persistence differences**: Windows uses registry, Unix uses rc-file markers
4. **Process kill differences**: May need process group handling on Unix
5. **Pre-existing test failures**: 9 tests fail, all pre-existing

## Missing Coverage

1. **Linux/macOS CI**: No CI pipeline for Linux or macOS yet
2. **Native runtime tests**: No tests for actual process execution on Linux/macOS
3. **PATH write tests**: No tests for rc-file marker behavior on Unix
4. **Process group kill**: No tests for process group handling on Unix
5. **Acceptance tests**: No cross-platform acceptance tests

## Work Intentionally Deferred

- Process-tree termination (TODO left in `platform` module)
- Package-manager installation (Session 2+)
- DevLauncher/Toolchain command routing through platform module (future sessions)
- `PlatformAdapter` trait migration to `platform` types
- Interactive stdin behavior audit for cross-platform correctness

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
