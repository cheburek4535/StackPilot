// Web emulator for Tauri IPC commands and events when running in browser mode

import toolsJson from "../../../src-tauri/src/modules/toolchain/tools.json";
import wizardTreeJson from "../../../src-tauri/src/modules/project_creator/knowledge/wizard_tree.json";
import type { ToolDefinition } from "$lib/modules/toolchain/types";
import type { WizardTreeData, ProjectTypeDef } from "$lib/modules/project_creator/types";

export function initTauriMock() {
  if (typeof window === "undefined") return;

  if ((window as any).__TAURI_INTERNALS__) {
    return;
  }

  // In-memory / local storage state
  const STORAGE_PREFIX = "stackpilot:mock:";
  const getStorage = <T>(key: string, fallback: T): T => {
    try {
      const val = localStorage.getItem(STORAGE_PREFIX + key);
      return val ? JSON.parse(val) : fallback;
    } catch {
      return fallback;
    }
  };

  const setStorage = <T>(key: string, val: T): void => {
    try {
      localStorage.setItem(STORAGE_PREFIX + key, JSON.stringify(val));
    } catch {
      // ignore
    }
  };

  const defaultSettings = () => ({
    vscode_path: "code",
    browser_path: "",
    terminal: "",
    theme: "dark",
    language: "ru",
    auto_save_profiles: true,
    auto_save: true,
    font_size: "md",
    reduced_motion: false,
    show_interface_hints: true,
    accent_color: "violet",
    restore_last_route: true,
    confirm_before_reset: true,
    personal: { name: "", username: "", email: "" },
    ai: {
      enabled: false,
      provider: "openai",
      base_url: "",
      api_key: "",
      model: "",
      temperature: 0.7,
      max_tokens: 2048,
      timeout_secs: 30,
      system_prompt: "",
      page_context: true,
    },
  });

  // Event callbacks registry
  let callbackIdCounter = 1;
  const callbacks = new Map<number, (event: any) => void>();
  const eventListeners = new Map<string, Set<number>>();

  const transformCallback = (cb: (event: any) => void, once = false) => {
    const id = callbackIdCounter++;
    callbacks.set(id, (payload: any) => {
      cb(payload);
      if (once) {
        callbacks.delete(id);
      }
    });
    return id;
  };

  const emitEvent = (event: string, payload: any) => {
    const ids = eventListeners.get(event);
    if (!ids || ids.size === 0) return;
    for (const id of Array.from(ids)) {
      const cb = callbacks.get(id);
      if (cb) {
        try {
          cb({ event, payload, id });
        } catch (e) {
          console.error(`Error in event listener for ${event}:`, e);
        }
      }
    }
  };

  // Preload initial catalog and tools
  const catalog: ToolDefinition[] = toolsJson as any;
  const wizardTree: WizardTreeData = wizardTreeJson as any;

  // Tracked processes
  let processes: Array<{
    id: string;
    pid: number;
    command: string;
    args: string[];
    label: string;
    status: string;
    started_at: string;
    working_dir: string | null;
  }> = [
    {
      id: "proc-dev-server",
      pid: 3000,
      command: "npm",
      args: ["run", "dev"],
      label: "StackPilot Web Dev Server",
      status: "Running",
      started_at: new Date(Date.now() - 1000 * 60 * 15).toISOString(),
      working_dir: "~/Projects/StackPilot",
    },
  ];

  const processLogs = new Map<string, { stdout: string[]; stderr: string[] }>();
  processLogs.set("proc-dev-server", {
    stdout: [
      "[Vite] VITE v6.0.3  ready in 145 ms",
      "[Vite] ➜  Local:   http://localhost:1420/",
      "[Vite] ➜  Network: use --host to expose",
      "[StackPilot] Services initialized successfully.",
    ],
    stderr: [],
  });

  // Current project state
  let currentProject: any = {
    profile_name: "StackPilot",
    project_path: "/workspace/StackPilot",
    description: "Developer toolchain management and project workspace dashboard",
    stack: ["typescript", "svelte", "tauri", "vite"],
  };

  // Mock workspace file system
  const mockFiles: Record<string, string> = {
    "/workspace/StackPilot/README.md": `# StackPilot\n\nDeveloper toolchain management and project workspace dashboard.`,
    "/workspace/StackPilot/package.json": JSON.stringify(
      {
        name: "stackpilot",
        version: "0.1.0",
        type: "module",
        scripts: { dev: "vite dev", build: "vite build" },
      },
      null,
      2
    ),
    "/workspace/StackPilot/src/App.svelte": `<script lang="ts">\n  // Main application\n</script>\n\n<main>\n  <h1>StackPilot</h1>\n</main>`,
  };

  // DevLauncher Profiles
  const defaultProfiles = [
    {
      name: "StackPilot Web",
      description: "Development environment for StackPilot web interface",
      project_path: "/workspace/StackPilot",
      working_dir: "/workspace/StackPilot",
      actions: [
        {
          id: "act-dev",
          name: "Start Dev Server",
          command: "npm",
          args: ["run", "dev"],
          action_type: "run",
          icon: "play",
          is_background: true,
          env_vars: {},
        },
        {
          id: "act-build",
          name: "Build App",
          command: "npm",
          args: ["run", "build"],
          action_type: "build",
          icon: "hammer",
          is_background: false,
          env_vars: {},
        },
        {
          id: "act-test",
          name: "Run Test Suite",
          command: "npm",
          args: ["test"],
          action_type: "test",
          icon: "check",
          is_background: false,
          env_vars: {},
        },
      ],
      env_vars: { NODE_ENV: "development", PORT: "3000" },
      created_at: new Date().toISOString(),
      updated_at: new Date().toISOString(),
    },
    {
      name: "FastAPI Backend",
      description: "Python async API service with Uvicorn",
      project_path: "/workspace/fastapi-service",
      working_dir: "/workspace/fastapi-service",
      actions: [
        {
          id: "act-fastapi-run",
          name: "Start Uvicorn",
          command: "uvicorn",
          args: ["main:app", "--reload", "--port", "8000"],
          action_type: "run",
          icon: "play",
          is_background: true,
          env_vars: {},
        },
        {
          id: "act-fastapi-test",
          name: "Run Pytest",
          command: "pytest",
          args: ["tests/"],
          action_type: "test",
          icon: "check",
          is_background: false,
          env_vars: {},
        },
      ],
      env_vars: { ENV: "local", DEBUG: "True" },
      created_at: new Date().toISOString(),
      updated_at: new Date().toISOString(),
    },
  ];

  // Active scan state
  let activeScanJob: any = null;
  let persistedJobs: any[] = getStorage("jobs", [
    {
      id: "job-init-101",
      operation: "install",
      status: "Success",
      created_at: new Date(Date.now() - 1000 * 60 * 60 * 2).toISOString(),
      finished_at: new Date(Date.now() - 1000 * 60 * 59).toISOString(),
      tools: [
        { tool_id: "nodejs", requested_version: "22", installed_version: "22.12.0", status: "Success" },
        { tool_id: "git", requested_version: "2.43", installed_version: "2.43.0", status: "Success" },
      ],
      tasks: [
        { id: "task-1", label: "Verify Node.js", status: "Success" },
        { id: "task-2", label: "Verify Git", status: "Success" },
      ],
    },
  ]);

  // Environment detected tool states
  const detectedToolMap: Record<string, { version: string; status: string; path: string }> = {
    nodejs: { version: "22.12.0", status: "healthy", path: "/usr/local/bin/node" },
    npm: { version: "10.9.0", status: "healthy", path: "/usr/local/bin/npm" },
    git: { version: "2.43.0", status: "healthy", path: "/usr/bin/git" },
    python: { version: "3.11.8", status: "healthy", path: "/usr/bin/python3" },
    pip: { version: "24.0", status: "healthy", path: "/usr/bin/pip3" },
    rust: { version: "1.82.0", status: "healthy", path: "~/.cargo/bin/rustc" },
    cargo: { version: "1.82.0", status: "healthy", path: "~/.cargo/bin/cargo" },
    docker: { version: "27.1.1", status: "healthy", path: "/usr/bin/docker" },
    vscode: { version: "1.95.0", status: "healthy", path: "/usr/bin/code" },
  };

  // Active project execution state
  let activeExecution: {
    running: boolean;
    recipe_name: string;
    project_path: string;
    events: any[];
  } | null = null;

  const invoke = async (cmd: string, args: Record<string, any> = {}): Promise<any> => {
    // console.log("[Tauri Mock Invoke]", cmd, args);

    // ==========================================
    // Core & Settings
    // ==========================================
    if (cmd === "get_settings") {
      const settings = getStorage("settings", defaultSettings());
      return settings;
    }

    if (cmd === "update_settings") {
      setStorage("settings", args.settings);
      return args.settings;
    }

    if (cmd === "reset_settings") {
      const def = defaultSettings();
      setStorage("settings", def);
      return def;
    }

    if (cmd === "settings_check_path") {
      return typeof args.path === "string" && args.path.trim().length > 0;
    }

    if (cmd === "get_app_data_dir") {
      return "~/AppData/Roaming/StackPilot (mock)";
    }

    // ==========================================
    // DevLauncher Commands
    // ==========================================
    if (cmd === "ping_rust" || cmd === "ping_toolchain" || cmd === "ping_project_creator") {
      return "pong";
    }

    if (cmd === "get_demo_profile") {
      return defaultProfiles[0];
    }

    if (cmd === "list_profiles") {
      return getStorage("profiles", defaultProfiles);
    }

    if (cmd === "get_profile") {
      const profiles = getStorage("profiles", defaultProfiles);
      const found = profiles.find((p: any) => p.name === args.name);
      if (!found) throw new Error(`Profile '${args.name}' not found`);
      return found;
    }

    if (cmd === "save_profile") {
      const profiles = getStorage("profiles", defaultProfiles);
      const idx = profiles.findIndex((p: any) => p.name === args.profile.name);
      if (idx >= 0) {
        profiles[idx] = { ...args.profile, updated_at: new Date().toISOString() };
      } else {
        profiles.push({
          ...args.profile,
          created_at: new Date().toISOString(),
          updated_at: new Date().toISOString(),
        });
      }
      setStorage("profiles", profiles);
      return;
    }

    if (cmd === "delete_profile") {
      let profiles = getStorage("profiles", defaultProfiles);
      profiles = profiles.filter((p: any) => p.name !== args.name);
      setStorage("profiles", profiles);
      return;
    }

    if (cmd === "execute_action") {
      const act = args.action;
      const pid = Math.floor(1000 + Math.random() * 9000);
      const newProc = {
        id: `proc-${Date.now()}`,
        pid,
        command: act.command || "node",
        args: act.args || [],
        label: act.name || "Action",
        status: act.is_background ? "Running" : "Success",
        started_at: new Date().toISOString(),
        working_dir: act.working_dir || "~/Projects",
      };
      processes.unshift(newProc);
      processLogs.set(newProc.id, {
        stdout: [
          `Executing: ${newProc.command} ${newProc.args.join(" ")}`,
          `Process started with PID ${pid}`,
          `Ready and listening.`,
        ],
        stderr: [],
      });
      return {
        success: true,
        pid,
        message: `Action ${act.name} executed successfully.`,
      };
    }

    if (cmd === "analyze_project") {
      return {
        name: args.path.split("/").pop() || "New Project",
        description: `Auto-analyzed project from ${args.path}`,
        project_path: args.path,
        working_dir: args.path,
        actions: [
          {
            id: "act-dev",
            name: "Dev Server",
            command: "npm",
            args: ["run", "dev"],
            action_type: "run",
            icon: "play",
            is_background: true,
            env_vars: {},
          },
          {
            id: "act-build",
            name: "Build",
            command: "npm",
            args: ["run", "build"],
            action_type: "build",
            icon: "hammer",
            is_background: false,
            env_vars: {},
          },
        ],
        env_vars: {},
        created_at: new Date().toISOString(),
        updated_at: new Date().toISOString(),
      };
    }

    // ==========================================
    // Project Creator Commands
    // ==========================================
    if (cmd === "get_wizard_tree") {
      return wizardTree;
    }

    if (cmd === "get_project_types") {
      return wizardTree.project_types || [];
    }

    if (cmd === "start_wizard") {
      return {
        id: `wiz-${Date.now()}`,
        project_path: args.projectPath || "~/Projects/my-app",
        current_question_id: "project_type",
        history: [],
        completed: false,
        stack: {
          project_type: null,
          languages: [],
          frameworks: [],
          tools: [],
          package_manager: null,
          docker: false,
          git: true,
        },
      };
    }

    if (cmd === "submit_wizard_answer") {
      const session = { ...args.session };
      session.history.push({
        question_id: args.questionId,
        answers: args.answers,
      });

      // Simple question graph progression
      const qOrder = ["project_type", "backend_lang", "backend_framework", "frontend_lang", "frontend_framework", "tools"];
      const currentIdx = qOrder.indexOf(args.questionId);
      if (currentIdx >= 0 && currentIdx < qOrder.length - 1) {
        session.current_question_id = qOrder[currentIdx + 1];
      } else {
        session.completed = true;
      }
      return session;
    }

    if (cmd === "analyze_project_technologies") {
      return {
        path: args.path,
        detected_languages: ["TypeScript", "Svelte", "Rust"],
        detected_frameworks: ["SvelteKit", "Tauri"],
        detected_tools: ["Git", "npm", "Vite"],
        confidence_score: 95,
        recommendations: ["Ensure Node.js 22+ and Rust are in PATH"],
      };
    }

    if (cmd === "preview_project_recipe") {
      const name = args.context?.project_name || "new-project";
      return {
        recipe_name: `${args.context?.project_type || "Web"} Starter Recipe`,
        description: `Generates a full scaffolding for ${name}`,
        files: [
          {
            path: "package.json",
            content: JSON.stringify({ name, version: "0.1.0", private: true }, null, 2),
            is_executable: false,
          },
          {
            path: "src/main.ts",
            content: `console.log("Hello from ${name}!");\n`,
            is_executable: false,
          },
          {
            path: "README.md",
            content: `# ${name}\n\nCreated with StackPilot Project Creator.\n`,
            is_executable: false,
          },
        ],
        steps: [
          { label: "Create directory structure", command: "mkdir -p" },
          { label: "Write template files", command: "scaffold files" },
          { label: "Initialize Git repository", command: "git init" },
          { label: "Install base dependencies", command: "npm install" },
        ],
        estimated_time_sec: 4,
      };
    }

    if (cmd === "start_project_execution") {
      const plan = {
        execution_id: `exec-${Date.now()}`,
        recipe_name: "Full Stack Starter",
        project_path: args.projectPath || "~/Projects/new-app",
        total_steps: 4,
      };

      activeExecution = {
        running: true,
        recipe_name: plan.recipe_name,
        project_path: plan.project_path,
        events: [],
      };

      // Simulate step-by-step progress events
      const steps = [
        "Creating project directory layout...",
        "Writing source code and config files...",
        "Configuring package manager and dependencies...",
        "Project scaffolding complete! Ready for development.",
      ];

      steps.forEach((step, idx) => {
        setTimeout(() => {
          const event = {
            step_index: idx + 1,
            total_steps: steps.length,
            step_name: step,
            status: idx === steps.length - 1 ? "completed" : "running",
            logs: [`[INFO] ${step}`],
          };
          if (activeExecution) {
            activeExecution.events.push(event);
            if (idx === steps.length - 1) {
              activeExecution.running = false;
            }
          }
          emitEvent("project_creator:step_event", event);
        }, (idx + 1) * 600);
      });

      return plan;
    }

    if (cmd === "project_execution_snapshot") {
      return activeExecution || { running: false, recipe_name: "", project_path: "", events: [] };
    }

    if (cmd === "check_project_folder_exists") {
      return false;
    }

    if (cmd === "get_host_platform") {
      return "linux";
    }

    if (cmd === "validate_project_stack" || cmd === "validate_project_stack_error") {
      const issues: any[] = [];
      const { backendLanguages = [], frontendLanguages = [], frameworks = [] } = args;
      // Check known warning pairs
      if (frameworks.includes("phoenix") && frameworks.includes("nextjs")) {
        issues.push({
          level: "warning",
          frameworks: ["phoenix", "nextjs"],
          message: "Phoenix LiveView duplicates Next.js UI layer.",
          recommendation: "Use standard LiveView templates or React SPA mode.",
        });
      }
      return issues;
    }

    if (cmd === "get_stack_recommendations") {
      const frameworks = args.frameworks || [];
      const recommended: any[] = [];
      if (frameworks.includes("nestjs")) {
        recommended.push({ framework: "react", note: "Excellent pairing for full-stack TypeScript enterprise applications." });
      }
      return {
        paired_frameworks: recommended,
        suggested_tools: ["docker", "git", "vscode"],
        missing_languages: [],
      };
    }

    // ==========================================
    // Toolchain Commands
    // ==========================================
    if (cmd === "tcx_get_catalog" || cmd === "tc_get_tool_definitions") {
      return catalog;
    }

    if (cmd === "tcx_get_environment_snapshot" || cmd === "tc_get_environment_info") {
      const tools: Record<string, any> = {};
      catalog.forEach((t) => {
        const detected = detectedToolMap[t.id];
        if (detected) {
          tools[t.id] = {
            tool_id: t.id,
            state: "installed",
            installed_version: detected.version,
            health_status: detected.status,
            active_path: detected.path,
            all_detected_paths: [detected.path],
            issues: [],
          };
        } else {
          tools[t.id] = {
            tool_id: t.id,
            state: "missing",
            installed_version: null,
            health_status: "unknown",
            active_path: null,
            all_detected_paths: [],
            issues: [],
          };
        }
      });
      return {
        timestamp: new Date().toISOString(),
        host_os: "linux",
        host_arch: "x86_64",
        score: { overall: 85, satisfied_count: 8, total_tools: catalog.length, health_ratio: 0.95 },
        tools,
      };
    }

    if (cmd === "tcx_start_scan") {
      const jobId = `scan-${Date.now()}`;
      activeScanJob = {
        job_id: jobId,
        status: "running",
        scanned_count: 0,
        total_count: catalog.length,
        results: {},
      };

      // Emit scan progress in background
      setTimeout(() => {
        catalog.forEach((tool, i) => {
          setTimeout(() => {
            const detected = detectedToolMap[tool.id];
            const result = {
              tool_id: tool.id,
              state: detected ? "installed" : "missing",
              installed_version: detected?.version ?? null,
              health_status: detected?.status ?? "unknown",
              active_path: detected?.path ?? null,
            };
            if (activeScanJob) {
              activeScanJob.scanned_count = i + 1;
              activeScanJob.results[tool.id] = result;
            }
            emitEvent("toolchainx:scan_progress", {
              job_id: jobId,
              current_tool: tool.id,
              progress: Math.round(((i + 1) / catalog.length) * 100),
              scanned: i + 1,
              total: catalog.length,
              result,
            });

            if (i === catalog.length - 1) {
              if (activeScanJob) activeScanJob.status = "done";
              emitEvent("toolchainx:scan_done", {
                job_id: jobId,
                status: "Success",
                duration_ms: 1200,
                scanned_count: catalog.length,
              });
            }
          }, (i + 1) * 35);
        });
      }, 50);

      return {
        job_id: jobId,
        is_new: true,
        target_count: catalog.length,
      };
    }

    if (cmd === "tcx_get_scan_job" || cmd === "tcx_get_latest_scan_job") {
      return activeScanJob;
    }

    if (cmd === "tcx_cancel_scan") {
      if (activeScanJob) activeScanJob.status = "cancelled";
      return true;
    }

    if (cmd === "tcx_get_tool_details") {
      const toolId = args.toolId;
      const detected = detectedToolMap[toolId];
      return {
        tool_id: toolId,
        state: detected ? "installed" : "missing",
        installed_version: detected?.version ?? null,
        health_status: detected?.status ?? "unknown",
        active_path: detected?.path ?? null,
        all_detected_paths: detected ? [detected.path] : [],
        issues: [],
        version_check: { ok: true, message: detected ? `Installed version ${detected.version}` : "Not detected in PATH" },
      };
    }

    if (cmd === "tcx_run_health_checks") {
      const toolIds: string[] = args.toolIds || [];
      return toolIds.map((id) => {
        const detected = detectedToolMap[id];
        return {
          tool_id: id,
          state: detected ? "installed" : "missing",
          installed_version: detected?.version ?? null,
          health_status: detected?.status ?? "unknown",
          active_path: detected?.path ?? null,
          all_detected_paths: detected ? [detected.path] : [],
          issues: [],
        };
      });
    }

    if (cmd === "tcx_profile_resolve") {
      const req = args.requirements || { languages: ["typescript", "rust"], tools: ["git", "docker"] };
      return {
        profile_name: "Project Toolchain",
        requirements: req,
        satisfied_count: 3,
        total_count: 4,
        items: [
          { tool_id: "nodejs", required_version: "20+", installed_version: "22.12.0", satisfied: true },
          { tool_id: "rust", required_version: "1.75+", installed_version: "1.82.0", satisfied: true },
          { tool_id: "git", required_version: "2.0+", installed_version: "2.43.0", satisfied: true },
          { tool_id: "docker", required_version: "24.0+", installed_version: "27.1.1", satisfied: true },
        ],
      };
    }

    if (cmd === "tcx_build_plan") {
      const req = args.request;
      const tools = req?.tools || [{ tool_id: "nodejs" }];
      return {
        fingerprint: `fp-${Date.now()}`,
        operation: req?.operation || "install",
        target_tools: tools.map((t: any) => t.tool_id),
        estimated_download_mb: 45,
        estimated_disk_mb: 180,
        requires_admin: false,
        tasks: tools.map((t: any, idx: number) => ({
          task_id: `task-${idx + 1}`,
          tool_id: t.tool_id,
          action: "download_and_extract",
          label: `Install ${t.tool_id}`,
          source_url: `https://mock-mirror.internal/${t.tool_id}.tar.gz`,
        })),
        warnings: [],
      };
    }

    if (cmd === "tcx_start_job") {
      const req = args.request;
      const jobId = `job-${Date.now()}`;
      const newJob = {
        id: jobId,
        operation: req?.operation || "install",
        status: "Running",
        created_at: new Date().toISOString(),
        tools: (req?.tools || []).map((t: any) => ({
          tool_id: t.tool_id,
          requested_version: "latest",
          installed_version: "latest",
          status: "Running",
        })),
        tasks: (req?.tools || []).map((t: any, idx: number) => ({
          id: `task-${idx + 1}`,
          label: `Install ${t.tool_id}`,
          status: "Running",
        })),
      };

      persistedJobs.unshift(newJob);
      setStorage("jobs", persistedJobs);

      // Emit job execution events
      setTimeout(() => {
        emitEvent("toolchainx:job_event", {
          job_id: jobId,
          seq: 1,
          event_type: "JobStarted",
          data: { operation: newJob.operation, tools: newJob.tools },
        });

        setTimeout(() => {
          emitEvent("toolchainx:job_event", {
            job_id: jobId,
            seq: 2,
            event_type: "TaskProgress",
            data: { task_id: "task-1", progress: 50, message: "Downloading package archive..." },
          });

          setTimeout(() => {
            newJob.status = "Success";
            newJob.tools.forEach((t: any) => (t.status = "Success"));
            newJob.tasks.forEach((t: any) => (t.status = "Success"));
            setStorage("jobs", persistedJobs);

            emitEvent("toolchainx:job_event", {
              job_id: jobId,
              seq: 3,
              event_type: "JobCompleted",
              data: { status: "Success" },
            });
          }, 800);
        }, 600);
      }, 200);

      return jobId;
    }

    if (cmd === "tcx_get_job") {
      return persistedJobs.find((j) => j.id === args.jobId) || null;
    }

    if (cmd === "tcx_list_jobs") {
      return persistedJobs;
    }

    if (cmd === "tcx_cancel_job") {
      const j = persistedJobs.find((x) => x.id === args.jobId);
      if (j) j.status = "Cancelled";
      setStorage("jobs", persistedJobs);
      return;
    }

    if (cmd === "tcx_retry_job") {
      return invoke("tcx_start_job", { request: { operation: "install", tools: [{ tool_id: "nodejs" }] } });
    }

    if (cmd === "tcx_adopt_tool") {
      return `Adopted tool ${args.toolId}`;
    }

    if (cmd === "tcx_uninstall_tool") {
      delete detectedToolMap[args.toolId];
      return;
    }

    // Legacy tc_*
    if (cmd === "tc_check_environment") {
      return {
        missing_tools: [],
        outdated_tools: [],
        installed_tools: ["nodejs", "git", "python", "rust"],
        all_satisfied: true,
      };
    }

    if (cmd === "tc_build_install_plan") {
      return {
        steps: [{ id: "step-1", tool: "nodejs", action: "install" }],
        estimated_time: "1m",
      };
    }

    if (cmd === "tc_run_install") {
      return;
    }

    if (cmd === "tc_get_install_status") {
      return null;
    }

    if (cmd === "tc_abort_install") {
      return true;
    }

    if (cmd === "tc_take_new_secrets") {
      return {};
    }

    if (cmd === "tc_get_metadata") {
      return { version: "0.1.0", initialized: true };
    }

    if (cmd === "tc_get_health_report") {
      return { healthy: true, issues: [] };
    }

    // ==========================================
    // Workspace Commands
    // ==========================================
    if (cmd === "spawn_process") {
      const pid = Math.floor(2000 + Math.random() * 8000);
      const proc = {
        id: `proc-${Date.now()}`,
        pid,
        command: args.command,
        args: args.args || [],
        label: args.label || args.command,
        status: "Running",
        started_at: new Date().toISOString(),
        working_dir: args.workingDir || "/workspace",
      };
      processes.unshift(proc);
      processLogs.set(proc.id, {
        stdout: [`Spawning ${proc.command} with args: ${proc.args.join(" ")}`, `Process started (PID ${pid})`],
        stderr: [],
      });
      return proc;
    }

    if (cmd === "list_processes") {
      return processes;
    }

    if (cmd === "kill_process") {
      const p = processes.find((x) => x.id === args.id);
      if (p) p.status = "Stopped";
      return;
    }

    if (cmd === "refresh_process") {
      const p = processes.find((x) => x.id === args.id);
      return p ? p.status : "Exited";
    }

    if (cmd === "get_process_logs") {
      return processLogs.get(args.id) || { stdout: ["No logs recorded yet."], stderr: [] };
    }

    if (cmd === "set_current_project") {
      currentProject = {
        profile_name: args.profileName,
        project_path: args.projectPath,
        description: args.description,
        stack: args.stack,
      };
      return currentProject;
    }

    if (cmd === "get_current_project") {
      return currentProject;
    }

    if (cmd === "clear_current_project") {
      currentProject = null;
      return;
    }

    if (cmd === "open_project_from_path") {
      currentProject = {
        profile_name: args.path.split("/").pop() || "Project",
        project_path: args.path,
        description: `Workspace loaded from ${args.path}`,
        stack: ["typescript", "svelte"],
      };
      return currentProject;
    }

    if (cmd === "get_session_info") {
      return {
        session_id: "sess-main-1",
        started_at: new Date(Date.now() - 1000 * 60 * 45).toISOString(),
        commands_executed: 14,
        active_processes: processes.filter((p) => p.status === "Running").length,
        errors_count: 0,
      };
    }

    if (cmd === "list_directory") {
      const basePath = args.path || "/workspace/StackPilot";
      return [
        { name: "src", path: `${basePath}/src`, is_dir: true, size: 4096, modified: new Date().toISOString() },
        { name: "static", path: `${basePath}/static`, is_dir: true, size: 4096, modified: new Date().toISOString() },
        { name: "package.json", path: `${basePath}/package.json`, is_dir: false, size: 1024, modified: new Date().toISOString() },
        { name: "README.md", path: `${basePath}/README.md`, is_dir: false, size: 512, modified: new Date().toISOString() },
        { name: "vite.config.js", path: `${basePath}/vite.config.js`, is_dir: false, size: 768, modified: new Date().toISOString() },
      ];
    }

    if (cmd === "read_file") {
      const content = mockFiles[args.path] || `// Contents of ${args.path}\nexport const ready = true;\n`;
      return { path: args.path, content };
    }

    if (cmd === "write_file") {
      mockFiles[args.path] = args.content;
      return;
    }

    if (cmd === "open_in_vscode") {
      console.log("[Tauri Mock] Opened in VSCode:", args.path);
      return;
    }

    if (cmd === "get_workspace_overview") {
      return {
        total_projects: 3,
        active_processes: processes.filter((p) => p.status === "Running").length,
        toolchain_health_score: 92,
        diagnostics_count: 0,
      };
    }

    if (cmd === "get_problems") {
      return [];
    }

    if (cmd === "clear_problems") {
      return;
    }

    // ==========================================
    // Plugin event calls
    // ==========================================
    if (cmd === "plugin:event|listen") {
      const eventName = args.event;
      const handlerId = args.handler;
      if (!eventListeners.has(eventName)) {
        eventListeners.set(eventName, new Set());
      }
      eventListeners.get(eventName)!.add(handlerId);
      return handlerId;
    }

    if (cmd === "plugin:event|unlisten") {
      const eventName = args.event;
      const handlerId = args.handler;
      eventListeners.get(eventName)?.delete(handlerId);
      callbacks.delete(handlerId);
      return;
    }

    if (cmd === "plugin:event|emit") {
      emitEvent(args.event, args.payload);
      return;
    }

    // ==========================================
    // Plugin dialog & opener calls
    // ==========================================
    if (cmd === "plugin:dialog|open") {
      return "/workspace/StackPilot";
    }

    if (cmd === "plugin:opener|open_url") {
      if (typeof window !== "undefined" && args.url) {
        window.open(args.url, "_blank");
      }
      return;
    }

    console.warn(`[Tauri Mock] Unhandled invoke command: ${cmd}`, args);
    return null;
  };

  // Mount Tauri internals on window
  (window as any).__TAURI_INTERNALS__ = {
    invoke,
    transformCallback,
    plugins: {},
  };

  // Also mount on __TAURI__ global if expected
  (window as any).__TAURI__ = {
    core: { invoke, transformCallback },
    event: {
      listen: (event: string, handler: any) => {
        const id = transformCallback(handler);
        return invoke("plugin:event|listen", { event, handler: id }).then(() => () => {
          invoke("plugin:event|unlisten", { event, handler: id });
        });
      },
      emit: (event: string, payload: any) => invoke("plugin:event|emit", { event, payload }),
    },
  };
}
