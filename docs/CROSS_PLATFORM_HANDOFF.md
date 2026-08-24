# Cross-Platform Refactor Handoff

## Baseline Commit

- **SHA**: `742d594ddbb66e19c8753110a5472d30279e8f1b`
- **Message**: `docs: initialize cross-platform refactor baseline`
- **Files**: `docs/CROSS_PLATFORM_PROGRESS.md`, `docs/CROSS_PLATFORM_HANDOFF.md`

## Session 1 Commit

- **Message**: `refactor: add cross-platform command foundation`
- **Files**: See Source File Modifications below

## Session 2 Commit

- **Message**: `refactor: make launcher and process control cross-platform`
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
- Unix process group management uses `libc::setsid()` via `pre_exec` (no additional dependency)

## Source File Modifications Per Phase

### Session 0
- **No production code changes** — audit and documentation only
- `docs/CROSS_PLATFORM_PROGRESS.md` — created
- `docs/CROSS_PLATFORM_HANDOFF.md` — created

### Session 1
- `src-tauri/src/platform/mod.rs` — **NEW** module root with doc comments
- `src-tauri/src/platform/host.rs` — **NEW** `HostOs`/`HostArch` enums, `current_os()`, `current_arch()`
- `src-tauri/src/platform/shell.rs` — **NEW** `ShellKind` enum, `parse_shell()`, `default_shell_for_platform()`, `resolve_shell()`, `shell_executable()`
- `src-tauri/src/platform/command.rs` — **NEW** `build_tokio_command()`, `build_std_command()`, `infer_command_mode()`, quoting functions, Windows program resolution
- `src-tauri/src/platform/environment.rs` — **NEW** `EnvironmentOverlay` with `apply_std()`/`apply_tokio()`, PATH deduplication
- `src-tauri/src/platform/paths.rs` — **NEW** `paths_eq()`, `is_descendant()`, `resolve_executable()` (via `which` crate), batch file detection
- `src-tauri/src/lib.rs` — added `pub mod platform;`
- `src-tauri/src/modules/project_creator/engine/process.rs` — refactored `build_command()` to delegate to `platform::command`

### Session 2 (Current)
- `src-tauri/src/modules/devlauncher/models.rs` — added `args_list: Option<Vec<String>>` to `ActionType::OpenApplication` with `#[serde(default, skip_serializing_if)]` for backward compatibility
- `src-tauri/src/modules/devlauncher/launch_engine.rs` — **MAJOR REFACTOR**: RunCommand uses `default_shell_for_platform()` instead of hardcoded `cmd /C`; ExecuteScript uses `parse_shell()` + `resolve_shell()` + `is_windows_only_shell()` validation; OpenApplication prefers `args_list` over `split_whitespace`; added 9 unit tests
- `src-tauri/src/modules/devlauncher/analyzer.rs` — added `args_list: None` to 3 `ActionType::OpenApplication` constructors
- `src-tauri/src/modules/workspace/process_manager.rs` — **MAJOR REFACTOR**: `build_command()` creates process groups (`CREATE_NEW_PROCESS_GROUP` on Windows, `setsid()` via `pre_exec` on Unix); `kill_process_tree()` dispatches to `kill_windows_tree()` (taskkill /F /T) or `kill_unix_group()` (SIGTERM → grace → SIGKILL); `kill()` does not overwrite natural Exited/Crashed with Killed; added 4+ unit tests

### Session 3 (Future)
- `src-tauri/src/modules/toolchain/platforms/unix_rc.rs` — PATH write backport
- `src-tauri/src/modules/toolchain/platforms/unix_rc.rs` — rc-file write tests

### Session 4 (Future)
- `src-tauri/.github/workflows/ci.yml` — Linux/macOS native CI
- `docs/CROSS_PLATFORM_PROGRESS.md` — documentation updates

### Session 5 (Future)
- `src-tauri/src/modules/toolchain/platforms/tests/` — cross-platform acceptance tests
- `src-tauri/tests/` — regression suite

## Build/Test Commands and Results

### Session 2 Commands and Results
```bash
cargo fmt          # Passed (auto-formatted)
cargo fmt --check  # Passed
cargo check        # Passed (warnings only — pre-existing)
cargo test         # BLOCKED: STATUS_ENTRYPOINT_NOT_FOUND (pre-existing Windows DLL issue)
```

### Results Summary
- **cargo fmt**: PASSED
- **cargo fmt --check**: PASSED
- **cargo check**: PASSED (warnings only — pre-existing unused imports, dead code, non-snake-case FFI)
- **cargo test**: BLOCKED by pre-existing `STATUS_ENTRYPOINT_NOT_FOUND` (0xc0000139) in test binary

#### New Tests (13 tests in session 2)
**launch_engine** (9 tests):
- `runcommand_uses_platform_default_shell` — verifies correct shell exe for current OS
- `runcommand_does_not_name_cmd_on_unix` — platform-gated: ensures cmd is never used on Unix
- `execute_script_shell_none_resolves_to_default` — Default resolves to Cmd on Windows, Sh on Unix
- `execute_script_legacy_shell_strings_parse_correctly` — all 7 legacy shell names parse
- `execute_script_legacy_shell_case_insensitive` — CMD, PowerShell, Bash parse correctly
- `execute_script_unsupported_shell_returns_error` — fish, zsh-plus return UnsupportedShell
- `open_application_prefers_args_list_over_string` — deserializes both fields correctly
- `open_application_backward_compat_no_args_list` — legacy profiles without args_list work
- `open_application_no_args_at_all` — no args at all deserializes correctly

**process_manager** (4+ tests):
- `generate_id_is_unique_enough` — IDs are unique and prefixed
- `timestamp_now_is_nonzero` — timestamp is valid
- `build_command_sets_working_dir` — spawn with working_dir succeeds
- `kill_returns_error_for_nonexistent_process` — error message includes "not found"
- `refresh_status_returns_error_for_nonexistent_process` — error on missing ID
- `get_logs_returns_error_for_nonexistent_process` — error on missing ID
- `list_empty_initially` — new manager has no processes
- `kill_windows_tree_handles_already_exited` (cfg windows) — taskkill on nonexistent PID returns 128+
- `kill_signal_to_process_group_does_not_crash` (cfg unix) — SIGTERM to group + reap works

#### Note on Test Binary
The `STATUS_ENTRYPOINT_NOT_FOUND` (0xc0000139) error is a pre-existing Windows environment issue where the test binary cannot load a required DLL. This affects ALL tests including session 1's `platform::shell`, `platform::command`, etc. `cargo check` and `cargo fmt --check` pass, confirming compilation correctness. Native CI on all three OSes (session 4) will resolve this.

### Pre-existing Failures
- `python_health_checks_do_not_include_pip_assertion` — pip not in tools.json
- `runner_executes_absolute_executable_path` — env-dependent batch file test
- 6× `scaffold_*` tests — scaffold CLI environment issues
- `live_repo_resolution_and_parsing` — ignored (network-dependent)

## Key Capabilities Delivered (Session 2)

### Launch Engine Cross-Platform Dispatch
1. **RunCommand**: Uses `default_shell_for_platform()` — `cmd /D /C` on Windows, `sh -lc` on Unix. No more hardcoded `cmd /C`.
2. **ExecuteScript**: Parses legacy shell strings via `parse_shell()` into typed `ShellKind`. Validates OS compatibility via `is_windows_only_shell()`. Clear error messages naming the shell, host OS, and remediation.
3. **OpenApplication**: Backward-compatible `args_list: Option<Vec<String>>` field. When present, uses structured args instead of `split_whitespace`. Front-end serialized profiles that lack this field deserialize as `None` via `#[serde(default)]`.

### Process Management Cross-Platform
4. **Process group spawn**: Windows uses `CREATE_NEW_PROCESS_GROUP`; Unix uses `setsid()` via `pre_exec`. Each tracked process runs in its own session/group.
5. **Windows tree kill**: `taskkill /F /T /PID` terminates the entire process tree (child + grandchildren from npm, cargo, Python, Java).
6. **Unix group kill**: `SIGTERM` to process group (`-pgid`), 2s grace period, then `SIGKILL` if still alive. No shell interpolation.
7. **Status transitions**: `kill()` does not overwrite natural `Exited`/`Crashed` with `Killed`. If process already exited, the existing status is preserved.
8. **Error propagation**: Failed kill returns `Err` with descriptive message. Kill failure is not silently swallowed.

## Risks

1. **Windows regression risk**: Any production code change could break Windows behavior
2. **Native CI required**: Compilation on all three OSes does NOT guarantee full support
3. **PATH persistence differences**: Windows uses registry, Unix uses rc-file markers
4. **Pre-exec safety**: `setsid()` in `pre_exec` is POSIX async-signal-safe but requires careful testing on macOS (where `fork` behavior differs)
5. **Test binary blocked**: `STATUS_ENTRYPOINT_NOT_FOUND` prevents running all unit tests on this Windows environment
6. **Grace period tuning**: 2s SIGTERM→SIGKILL interval is a reasonable default but may need per-action configuration
7. **Pre-existing test failures**: 9 tests fail, all pre-existing

## Missing Coverage

1. **Linux/macOS CI**: No CI pipeline for Linux or macOS yet
2. **Native runtime tests**: No tests for actual process execution on Linux/macOS (compilation only)
3. **PATH write tests**: No tests for rc-file marker behavior on Unix
4. **Process group kill on macOS**: `setsid()` + `killpg` needs native macOS CI validation
5. **Acceptance tests**: No cross-platform acceptance tests
6. **Timeout interaction**: No tests for process group kill combined with timeout behavior

## Work Intentionally Deferred

- PATH write backport (Session 3)
- Package-manager installation (Session 3+)
- `PlatformAdapter` trait migration to `platform` types
- Interactive stdin behavior audit for cross-platform correctness
- Per-action timeout with process group kill

## Instructions for Later Agents

1. **Update this document**: When completing a session, update:
   - Phase checklist in `CROSS_PLATFORM_PROGRESS.md`
   - Cross-platform matrix in `CROSS_PLATFORM_PROGRESS.md`
   - Source file modifications in this document
   - Build/test commands and results in this document
   - Risks and missing coverage

2. **Commit messages**: Use format `refactor: <description>`

3. **Validation before commit**: Always run:
   ```bash
   cargo fmt --check
   cargo check
   cargo test
   ```

4. **Windows regression**: Any production code change MUST include a Windows regression test

5. **Native CI**: Linux/macOS support is NOT complete until CI runs on the actual OS

6. **Pre-existing failures**: The `python_health_checks_do_not_include_pip_assertion` test failure is pre-existing. If fixing it, document the fix. If skipping, document why.
