// ============================================================
// Typed job events (engine/events.rs)
// ============================================================
//
// Every event carries full operation identity: job id, task id, tool
// id, per-job sequence number, timestamp and a typed payload. A
// constructor without identity does not exist; consumers filter by
// `belongs_to` so stale events from unrelated jobs can be dropped.

use serde::{Deserialize, Serialize};

use super::plan::{EngineTaskStatus, JobStatus, PathChangeRecord, Phase};
use super::request::OperationKind;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobEventPayload {
    JobStarted {
        operation: OperationKind,
        total_tasks: usize,
    },
    TaskStarted {
        index: usize,
        total: usize,
    },
    TaskPhase {
        phase: Phase,
    },
    /// Raw progress line (installer output). Secrets are redacted upstream.
    Progress {
        line: String,
    },
    PathUpdated {
        record: PathChangeRecord,
    },
    TaskCompleted {
        status: EngineTaskStatus,
    },
    JobFinished {
        status: JobStatus,
        errors: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobEvent {
    pub job_id: String,
    pub task_id: String,
    pub tool_id: String,
    pub seq: u64,
    pub timestamp: String,
    pub payload: JobEventPayload,
}

impl JobEvent {
    /// Builds an event. Returns None when identity is missing — events
    /// without a job id are contract violations and must not exist.
    pub fn try_new(
        job_id: &str,
        task_id: &str,
        tool_id: &str,
        seq: u64,
        payload: JobEventPayload,
    ) -> Option<Self> {
        if job_id.is_empty() || seq == 0 {
            return None;
        }
        Some(Self {
            job_id: job_id.to_string(),
            task_id: task_id.to_string(),
            tool_id: tool_id.to_string(),
            seq,
            timestamp: crate::modules::toolchain::core::console::timestamp(),
            payload,
        })
    }

    /// Consumer-side guard: ignore everything from unrelated jobs.
    pub fn belongs_to(&self, job_id: &str) -> bool {
        self.job_id == job_id
    }
}

/// Emission port implemented by the commands layer (tauri emit) and by
/// test buffers.
pub trait TcxEventSink: Send + Sync {
    fn emit(&self, event: &JobEvent);
}

/// Collects events in memory — used by tests to assert ordering/seqs.
#[derive(Default)]
pub struct MemorySink {
    pub events: std::sync::Mutex<Vec<JobEvent>>,
}

impl TcxEventSink for MemorySink {
    fn emit(&self, event: &JobEvent) {
        if let Ok(mut buf) = self.events.lock() {
            buf.push(event.clone());
        }
    }
}

impl MemorySink {
    pub fn snapshot(&self) -> Vec<JobEvent> {
        self.events.lock().map(|b| b.clone()).unwrap_or_default()
    }

    pub fn seqs_of(&self, job_id: &str) -> Vec<u64> {
        self.snapshot()
            .into_iter()
            .filter(|e| e.belongs_to(job_id))
            .map(|e| e.seq)
            .collect()
    }
}

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(job_id: &str, seq: u64) -> JobEvent {
        JobEvent::try_new(
            job_id,
            "task-1",
            "git",
            seq,
            JobEventPayload::Progress { line: "x".into() },
        )
        .unwrap()
    }

    #[test]
    fn event_without_job_id_is_rejected() {
        let no_job = JobEvent::try_new(
            "",
            "t",
            "git",
            1,
            JobEventPayload::Progress { line: "x".into() },
        );
        assert!(no_job.is_none(), "identity-less events must not exist");
    }

    #[test]
    fn zero_seq_is_rejected() {
        assert!(JobEvent::try_new(
            "j",
            "t",
            "git",
            0,
            JobEventPayload::Progress { line: "x".into() }
        )
        .is_none());
    }

    #[test]
    fn belongs_to_filters_foreign_jobs() {
        let mine = ev("job-a", 1);
        let stale = ev("job-old", 7);
        assert!(mine.belongs_to("job-a"));
        assert!(
            !stale.belongs_to("job-a"),
            "stale event from unrelated job is droppable"
        );
    }

    #[test]
    fn memory_sink_records_everything_with_seq_and_timestamp() {
        let sink = MemorySink::default();
        sink.emit(&ev("j1", 1));
        sink.emit(&ev("j1", 2));
        sink.emit(&ev("j2", 1));

        let all = sink.snapshot();
        assert_eq!(all.len(), 3);
        assert_eq!(sink.seqs_of("j1"), vec![1, 2]);
        assert!(all.iter().all(|e| !e.timestamp.is_empty()));
        assert!(!all[0].task_id.is_empty());
        assert_eq!(all[0].tool_id, "git");
    }

    #[test]
    fn payloads_roundtrip_through_serde() {
        let payload = JobEventPayload::JobFinished {
            status: JobStatus::Partial,
            errors: vec!["node failed".into()],
        };
        let raw = serde_json::to_string(&payload).unwrap();
        let back: JobEventPayload = serde_json::from_str(&raw).unwrap();
        match back {
            JobEventPayload::JobFinished { status, errors } => {
                assert_eq!(status, JobStatus::Partial);
                assert_eq!(errors, vec!["node failed"]);
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }
}
