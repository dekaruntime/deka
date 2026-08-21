use super::*;

impl IsolatePool {
    /// Hash handler key to worker index.
    pub(super) fn hash_to_worker(&self, key: &HandlerKey) -> usize {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        (hasher.finish() as usize) % self.workers.len()
    }

    pub(super) fn select_worker_with_exclude(
        &self,
        key: &HandlerKey,
        exclude: Option<usize>,
    ) -> usize {
        if self.workers.len() <= 1 {
            return 0;
        }
        match self.config.scheduler_strategy {
            SchedulerStrategy::ConsistentHash => {
                let mut index = self.hash_to_worker(key);
                if Some(index) == exclude {
                    index = (index + 1) % self.workers.len();
                }
                index
            }
            SchedulerStrategy::LeastLoaded => {
                let mut best: Option<(usize, usize)> = None;
                for (index, worker) in self.workers.iter().enumerate() {
                    if Some(index) == exclude {
                        continue;
                    }
                    let queued = worker.load.queued_requests.load(Ordering::Relaxed);
                    let active = worker.load.active_requests.load(Ordering::Relaxed);
                    let load = queued + active;
                    match best {
                        None => best = Some((index, load)),
                        Some((_, best_load)) if load < best_load => best = Some((index, load)),
                        _ => {}
                    }
                }
                best.map(|(index, _)| index).unwrap_or(0)
            }
        }
    }
}
