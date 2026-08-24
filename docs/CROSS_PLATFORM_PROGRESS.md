# Cross-Platform Refactor Progress

## Phase Checklist (Sessions 0–5)

- [x] **Session 0**: Baseline audit (OS-specific behaviors, validation, documentation)
- [ ] **Session 1**: Platform adapter trait enforcement + Linux/macOS CI pipeline
- [ ] **Session 2**: PATH write backport + rc-file write tests
- [ ] **Session 3**: Windows process group kill + toolchain workflow tests
- [ ] **Session 4**: Linux/macOS native CI + documentation
- [ ] **Session 5**: Cross-platform acceptance tests + regression suite

## Cross-Platform Matrix

| Feature | Windows | Linux | macOS | Status |
|---------|---------|-------|-------|--------|
| DevLauncher RunCommand | `cmd /C` | `sh -c` | `sh -c` | Windows verified |
| DevLauncher ExecuteScript | PowerShell/`cmd /C` | `/bin/sh`/bash | `/bin/sh`/zsh | Windows verified |
| Process kill | `child.kill()` | `child.kill()` | `child.kill()` | Needs audit |
| PATH persistence | Registry + PS script | rc-file markers | rc-file markers | Windows verified |
| Toolchain installer | winget/registry | apt/dnf/pacman | brew | Windows verified |
| Console process run | PowerShell/UAC | bash/sh | zsh/bash | Windows verified |

## OS-Specific Behaviors

### DevLauncher
- **RunCommand dispatch**: `src-tauri/src/modules/devlauncher/launch_engine.rs:51-55` — `ActionType::RunCommand` uses `StdCommand` directly without shell wrapping
- **ExecuteScript**: `launch_engine.rs` — `ActionType::ExecuteScript` with `shell: Option<String>` — no Unix default shell logic
- **Process management**: `src-tauri/src/modules/workspace/process_manager.rs` — uses `child.kill()` from std

### Toolchain
- **Windows-first comment**: `src-tauri/src/modules/toolchain/core/installer.rs:22` — "Windows-first: Linux/macOS tasks marked Skipped"
- **Console process running**: `src-tauri/src/modules/toolchain/core/console.rs:17` — all process running is Windows-first
- **PATH write**: `src-tauri/src/modules/toolchain/platforms/unix_rc.rs` — uses marker-delimited block `# StackPilot:begin`/`# StackPilot:end`

### Platform Adapters
- `src-tauri/src/modules/toolchain/platforms/mod.rs` — `PlatformAdapter` trait
- `src-tauri/src/modules/toolchain/platforms/windows.rs` — Windows (winget, registry, PowerShell)
- `src-tauri/src/modules/toolchain/platforms/linux.rs` — Linux (apt/dnf/pacman)
- `src-tauri/src/modules/toolchain/platforms/macos.rs` — macOS (brew)

### Project Creator
- **Process dispatch**: `src-tauri/src/modules/project_creator/engine/process.rs` — cross-platform `TokioCommand` with Windows (exe/.cmd/.bat) vs Unix (`sh -c`)
- **Validate**: `src-tauri/src/modules/project_creator/validate.rs:10` — OS platform check

## Acceptance Criteria

1. **All sessions complete**: 6/6 sessions done
2. **All phases complete**: All sub-tasks in each session done
3. **Platform matrix**: All features verified on all 3 OSes
4. **CI**: All three OSes pass `cargo fmt --check`, `cargo check`, `cargo test`
5. **Documentation**: Both docs complete and up-to-date

## Windows Regression Rule

**CRITICAL**: Any production code change MUST include a Windows regression test. If changing `RunCommand`, `ExecuteScript`, process kill, or PATH persistence, add a test that validates Windows behavior is preserved.

## Native CI Validation

**IMPORTANT**: Compilation on all three OSes does NOT guarantee full support. Each OS must have:
- Native runtime validation (actual process execution, PATH writes, file operations)
- CI pipeline running on the actual OS
- Acceptance tests passing on the actual OS

## Validation Results (Session 0)

| Check | Result | Notes |
|-------|--------|-------|
| `cargo fmt --check` | FAILED | Pre-existing diffs in core/models.rs, core/settings.rs, toolchain/commands.rs |
| `cargo check` | PASSED | Warnings only (unused imports, dead code) — no errors |
| `cargo test` | FAILED | 682 passed, 1 failed, 1 ignored |

### Test Failure Details

- **Failed**: `modules::toolchain::domain::detect::tests::python_health_checks_do_not_include_pip_assertion`
  - Location: `src-tauri/src/modules/toolchain/domain/detect.rs:1336:32`
  - Error: `pip нет в tools.json` (pip not in tools.json)
  - Status: Pre-existing failure, not caused by this session

- **Ignored**: `modules::toolchain::core::qt_installer::tests::live_repo_resolution_and_parsing`
  - Status: Pre-existing skip (network-dependent test)

## Baseline Commit

- **SHA**: `742d594ddbb66e19c8753110a5472d30279e8f1b`
- **Date**: Session 0 baseline
