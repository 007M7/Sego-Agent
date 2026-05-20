//! Swarm branch-lock protocol — prevents parallel agents from colliding
//! on the same module/scope/branch.
//!
//! Uses `.claw/locks/` as the lock directory. Each lock is a JSON file
//! recording the agent ID, scope, and timestamp.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const LOCK_DIR: &str = ".claw/locks";
const LOCK_TTL_SECONDS: u64 = 300; // 5 minutes

#[derive(Debug, Clone)]
pub struct BranchLock {
    pub scope: String,
    pub agent_id: String,
    pub locked_at_secs: u64,
}

#[derive(Debug, Clone, Default)]
pub struct BranchLockRegistry {
    locks: HashMap<String, BranchLock>,
    lock_dir: PathBuf,
}

impl BranchLockRegistry {
    #[must_use]
    pub fn new(workspace_root: impl AsRef<Path>) -> Self {
        Self {
            locks: HashMap::new(),
            lock_dir: workspace_root.as_ref().join(LOCK_DIR),
        }
    }

    /// Try to acquire a lock for the given scope. Returns true if acquired,
    /// false if the scope is already locked by another agent.
    pub fn try_acquire(&mut self, scope: &str, agent_id: &str) -> bool {
        self.refresh();

        if let Some(existing) = self.locks.get(scope) {
            let now = now_secs();
            if existing.locked_at_secs + LOCK_TTL_SECONDS > now
                && existing.agent_id != agent_id
            {
                return false; // Still locked by another agent
            }
            // Lock expired or same agent — allow re-acquire
        }

        let lock = BranchLock {
            scope: scope.to_string(),
            agent_id: agent_id.to_string(),
            locked_at_secs: now_secs(),
        };

        if let Err(_) = self.persist_lock(&lock) {
            return false;
        }

        self.locks.insert(scope.to_string(), lock);
        true
    }

    /// Release a lock held by the given agent.
    pub fn release(&mut self, scope: &str, agent_id: &str) -> bool {
        self.refresh();
        if let Some(lock) = self.locks.get(scope) {
            if lock.agent_id == agent_id {
                self.locks.remove(scope);
                let _ = fs::remove_file(self.lock_path(scope));
                return true;
            }
        }
        false
    }

    /// Check if a scope is currently locked.
    #[must_use]
    pub fn is_locked(&self, scope: &str) -> bool {
        self.locks.get(scope).is_some_and(|lock| {
            lock.locked_at_secs + LOCK_TTL_SECONDS > now_secs()
        })
    }

    /// List all active locks.
    #[must_use]
    pub fn active_locks(&self) -> Vec<&BranchLock> {
        let now = now_secs();
        self.locks
            .values()
            .filter(|lock| lock.locked_at_secs + LOCK_TTL_SECONDS > now)
            .collect()
    }

    fn refresh(&mut self) {
        let now = now_secs();
        self.locks.retain(|_, lock| {
            lock.locked_at_secs + LOCK_TTL_SECONDS > now
        });
    }

    fn persist_lock(&self, lock: &BranchLock) -> Result<(), std::io::Error> {
        let dir = &self.lock_dir;
        fs::create_dir_all(dir)?;
        let path = self.lock_path(&lock.scope);
        let content = serde_json::json!({
            "scope": lock.scope,
            "agent_id": lock.agent_id,
            "locked_at_secs": lock.locked_at_secs,
        });
        fs::write(&path, serde_json::to_string(&content)?)?;
        Ok(())
    }

    fn lock_path(&self, scope: &str) -> PathBuf {
        let safe_name = scope.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
        self.lock_dir.join(format!("{safe_name}.lock"))
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn acquire_and_release_lock() {
        let tmp = std::env::temp_dir().join(format!("sego-lock-test-{}", rand_id()));
        let mut registry = BranchLockRegistry::new(&tmp);

        assert!(registry.try_acquire("src/auth", "agent-1"));
        assert!(registry.is_locked("src/auth"));

        assert!(registry.release("src/auth", "agent-1"));
        assert!(!registry.is_locked("src/auth"));

        fs::remove_dir_all(registry.lock_dir).expect("cleanup");
    }

    #[test]
    fn prevents_concurrent_lock() {
        let tmp = std::env::temp_dir().join(format!("sego-lock-test-{}", rand_id()));
        let mut registry = BranchLockRegistry::new(&tmp);

        assert!(registry.try_acquire("src/auth", "agent-1"));
        assert!(!registry.try_acquire("src/auth", "agent-2")); // blocked

        // Same agent can re-acquire
        assert!(registry.try_acquire("src/auth", "agent-1"));

        fs::remove_dir_all(registry.lock_dir).expect("cleanup");
    }

    #[test]
    fn different_scopes_dont_conflict() {
        let tmp = std::env::temp_dir().join(format!("sego-lock-test-{}", rand_id()));
        let mut registry = BranchLockRegistry::new(&tmp);

        assert!(registry.try_acquire("src/auth", "agent-1"));
        assert!(registry.try_acquire("src/api", "agent-2"));

        assert_eq!(registry.active_locks().len(), 2);

        fs::remove_dir_all(registry.lock_dir).expect("cleanup");
    }

    fn rand_id() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0)
    }
}
