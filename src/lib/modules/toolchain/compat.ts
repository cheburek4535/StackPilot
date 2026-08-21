// ============================================================
// Toolchain — слой совместимости Project Creator (контракт §7)
// ============================================================
// ЕДИНАЯ точка входа мастера создания проектов в toolchain. Поверх
// этого слоя мастер НЕ должен импортировать api.ts напрямую: здесь
// видно всё легаси-поверхность, которую когда-нибудь мигрируем на tcx_*.
//
// Гарантии совместимости:
//  - имена функций/типов и payload'ы байт-совместимы с легаси-командами;
//  - слушатели возвращают unlisten-функции (как раньше);
//  - секреты по-прежнему только через одноразовый getNewSecrets();
//  - поведение мастера не меняется (миграция на канонический движок
//    уже выполнена НА БЭКЕНДЕ внутри адаптера tc_run_install).

// ---- Легаси-команды ----
export {
  checkEnvironment,
  buildInstallPlan,
  runInstall,
  abortInstall,
  getNewSecrets,
  getInstallStatus,
  getToolchainMetadata,
  getEnvironmentInfo,
  getHealthReport,
  getToolDefinitions,
} from "./api";

// ---- Легаси-слушатели (unlisten-совместимые) ----
export {
  listenToolchainEvents,
  listenInstallDone,
  listenCheckProgress,
} from "./api";

// ---- Легаси-типы payload'ов ----
export type {
  ProjectRequirements,
  EnvironmentCheck,
  EnvironmentInfo,
  ToolRequirement,
  ToolStatus,
  InstallPlan,
  InstallTask,
  InstallSession,
  TaskPhase,
  TaskState,
  ToolchainEvent,
  CheckProgressEvent,
  ToolchainMetadata,
  HealthReport,
} from "./types";

// ---- Легаси-хелперы дискриминаторов ----
export {
  statusIsOk,
  statusLabel,
  statusKind,
  taskStateKind,
  taskStateLabel,
} from "./types";

// ---- Guard против stale-событий (чужие session_id/scan_id) ----
export { identityMatches } from "./stateLogic";
