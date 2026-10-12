//! Stable bounded recent-Session ranking for one Runner detail page.
//! The Runner page historically ranked only by updated_at (not working status).

use super::{RuntimeConsoleRecentSession, RuntimeConsoleRecentSessions, DEFAULT_MAX_SESSIONS};
use crate::tool_runtime::sessions::WorkflowSessionConsoleListItem;
use std::collections::BinaryHeap;

struct RankedRunnerSession {
    position: usize,
    row: RuntimeConsoleRecentSession,
}

impl PartialEq for RankedRunnerSession {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other).is_eq()
    }
}
impl Eq for RankedRunnerSession {}
impl PartialOrd for RankedRunnerSession {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for RankedRunnerSession {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // The largest (worst/oldest, or later input on a tie) is evicted.
        other
            .row
            .session
            .updated_at
            .cmp(&self.row.session.updated_at)
            .then_with(|| self.position.cmp(&other.position))
    }
}

#[derive(Default)]
pub(super) struct RunnerRecentCollector {
    heap: BinaryHeap<RankedRunnerSession>,
    candidate_count: usize,
}

impl RunnerRecentCollector {
    pub(super) fn offer(
        &mut self,
        client_id: &str,
        project_id: &str,
        project_name: Option<&str>,
        session: WorkflowSessionConsoleListItem,
    ) {
        let position = self.candidate_count;
        self.candidate_count += 1;
        // Equal timestamps retain earlier input, matching stable sort.
        if self.heap.len() >= DEFAULT_MAX_SESSIONS
            && self
                .heap
                .peek()
                .is_some_and(|worst| session.updated_at <= worst.row.session.updated_at)
        {
            return;
        }
        let entry = RankedRunnerSession {
            position,
            row: RuntimeConsoleRecentSession {
                client_id: client_id.to_string(),
                project_id: project_id.to_string(),
                project_name: project_name.map(str::to_string),
                session,
            },
        };
        if self.heap.len() >= DEFAULT_MAX_SESSIONS {
            self.heap.pop();
        }
        self.heap.push(entry);
    }

    pub(super) fn finish(self, scan_truncated: bool) -> RuntimeConsoleRecentSessions {
        let sessions = self
            .heap
            .into_sorted_vec()
            .into_iter()
            .map(|item| item.row)
            .collect::<Vec<_>>();
        RuntimeConsoleRecentSessions {
            returned: sessions.len(),
            candidate_count: self.candidate_count,
            truncated: self.candidate_count > DEFAULT_MAX_SESSIONS,
            scan_truncated,
            sessions,
        }
    }
}
