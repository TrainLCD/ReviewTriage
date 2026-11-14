use std::{path::PathBuf, time::Instant};

pub struct Progress {
    path: PathBuf,
    started: Instant,
    pub total: usize,
    pub batch: usize,
    pub batches: usize,
    pub analyzed: usize,
    pub failed: usize,
    pub created: usize,
    pub pending: usize,
}

impl Progress {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            started: Instant::now(),
            total: 0,
            batch: 0,
            batches: 0,
            analyzed: 0,
            failed: 0,
            created: 0,
            pending: 0,
        }
    }

    pub fn save(&self, phase: &'static str) {
        // Progress is separate from untrusted model output and contains no review text or IDs.
        let value = serde_json::json!({"phase":phase,"total":self.total,"batch":self.batch,
            "batches":self.batches,"analyzed":self.analyzed,"failed":self.failed,
            "created":self.created,"pending":self.pending,"elapsed_seconds":self.started.elapsed().as_secs()});
        if crate::atomic_json(&self.path, &value).is_err() {
            eprintln!("Could not save numeric progress; review processing continues");
        }
    }
}
