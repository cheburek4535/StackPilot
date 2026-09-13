// ============================================================
// Canonical installation & environment-management pipeline
// (engine/)
// ============================================================
//
// The backend-authoritative replacement layer for ad-hoc install
// sessions. Layers:
//
//   request.rs  — the ONLY input shape accepted from the frontend
//                 (deny_unknown_fields: no URLs, paths, args, versions,
//                 sizes, states or dependencies can be expressed);
//   planner.rs  — resolves definitions/platform/deps/conflicts/sources,
//                 validates admin + integrity, computes disk/capabilities,
//                 applies idempotency, assigns ids + fingerprint;
//   jobs.rs     — job registry: persistence before execution, typed
//                 terminal states, restart recovery, bounded history;
//   exec.rs     — executor: typed phases, integrity/source revalidation,
//                 audited PATH changes, cancellation, cleanup;
//   events.rs   — typed events with full operation identity.
//
// Safety rules (contract §5) hold here: scans never mutate, plans are
// backend-built, secrets stay out of job state, PATH changes are
// explicit and reported, Project Creator keeps working through the
// legacy adapter in commands.rs.

// Публичная поверхность движка растёт вместе с UI-этапом; часть
// методов/типов пока используется только тестами и будущими
// потребителями. Точечные allow расставлять дороже, чем один честный
// модульный.
#![allow(dead_code)]

pub mod events;
pub mod exec;
pub mod jobs;
pub mod plan;
pub mod planner;
pub mod request;

// Re-exports form the public engine API consumed across the crate and
// by future frontend bindings; not every name is referenced internally.
#[allow(unused_imports)]
pub use events::{JobEvent, JobEventPayload, TcxEventSink};
#[allow(unused_imports)]
pub use exec::{run_job, RealRunner, TaskContext, TaskOutcome, TaskRunner};
#[allow(unused_imports)]
pub use jobs::{JobEngine, JobHandle, PersistedJob};
#[allow(unused_imports)]
pub use plan::{
    CanonicalPlan, EngineTaskStatus, ExecutionMode, JobStatus, NoopReason, PathChangeRecord, Phase,
    PlanTask, PlanWarning, SelectedSource, TaskAction,
};
#[allow(unused_imports)]
pub use planner::{build_plan, Detector, DiscoveryDetector, PlanError, PlanInputs};
#[allow(unused_imports)]
pub use request::{EngineRequest, ExecutionChoice, OperationKind, ToolRequest, VersionChannel};

// ============================================================
// Тесты согласования слоёв (roundtrip: request -> plan -> job)
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::toolchain::models::{DetectionRules, InstallSources, VersionRules};
    use planner::fixtures::FakeDetectorHarness;
    use std::sync::Arc;

    #[tokio::test]
    async fn pipeline_end_to_end_on_fake_catalog() {
        let defs = vec![fake_def("git")];
        let detector = FakeDetectorHarness::missing_all();
        let inputs = PlanInputs::new(&defs, &detector).with_free_space(1000);

        let req = EngineRequest::new(OperationKind::Install, vec![ToolRequest::id("git")]);

        let plan = build_plan(&req, &inputs).await.expect("plan builds");
        assert_eq!(plan.operation, OperationKind::Install);
        assert_eq!(plan.tasks.len(), 1);
        assert!(matches!(
            plan.tasks[0].action,
            TaskAction::InstallNew { .. }
        ));

        let dir = std::env::temp_dir().join(format!(
            "tcx-engine-e2e-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let engine = Arc::new(JobEngine::load(&dir));
        let sink = Arc::new(events::MemorySink::default());
        engine.add_sink(sink.clone());

        let handle = engine.register(plan).expect("job registers");
        let runner: Arc<dyn TaskRunner> = Arc::new(exec::test_support::SuccessRunner);
        let secrets = run_job(
            engine.clone(),
            handle.clone(),
            Arc::new(defs.clone()),
            runner,
        )
        .await;

        assert!(secrets.is_empty());
        let snap = handle.snapshot();
        assert_eq!(snap.status, JobStatus::Succeeded);
        assert!(snap.finished_at.is_some());

        let seqs = sink.seqs_of(&snap.job_id);
        assert!(!seqs.is_empty());
        assert!(seqs.windows(2).all(|w| w[1] > w[0]), "monotonic seq");

        let listed = engine.list();
        assert!(listed.iter().any(|j| j.job_id == snap.job_id));

        let _ = std::fs::remove_dir_all(&dir);
    }

    fn fake_def(id: &str) -> crate::modules::toolchain::models::ToolDefinition {
        use crate::modules::toolchain::models::{InstallSource, InstallSourceKind};
        // Источник кладётся во все ОС-слоты: планировщик берёт слот текущей
        // ОС, тест обязан работать одинаково на Windows/Linux/macOS.
        let src = InstallSource {
            kind: InstallSourceKind::Official,
            id: "src-1".into(),
            url: Some("https://example.com/x.exe".into()),
            file_name: None,
            args: vec![],
            extra_args: vec![],
            dynamic_args: false,
            install_dir: None,
            needs_admin: None,
            execution: None,
            bootstrap: None,
            sha256: Some("aa".into()),
            url_template: None,
            version_resolver: None,
        };
        crate::modules::toolchain::models::ToolDefinition {
            id: id.to_string(),
            category: "utility".into(),
            display: id.into(),
            description: String::new(),
            icon: None,
            detection: DetectionRules::default(),
            versions: VersionRules::default(),
            sources: InstallSources {
                windows: vec![src.clone()],
                linux: vec![src.clone()],
                macos: vec![src],
            },
            size_mb: 5,
            needs_admin: false,
            path_entries: vec![],
            bundled_with: None,
            health_checks: vec![],
            notes: None,
            manual_install: None,
            extended: Default::default(),
        }
    }
}
