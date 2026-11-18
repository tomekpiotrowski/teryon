//! Which thread a copy made without `teryon worktree create` belongs to.
//!
//! Agents are told to start changes through Teryon, but one may still run
//! `git worktree add` itself, and copies made before Teryon named them are
//! named after their thread. Such a copy is given to a thread only when that
//! cannot be wrong. Giving work to the wrong thread is worse than not showing
//! it at all.
//!
//! A copy is judged once, when Teryon first notices it. Later the situation
//! may look clearer, say once one of two running threads has stopped, but
//! the copy is no newer than it was, and the stopped thread may have made it.

use uuid::Uuid;

/// A thread of the project that has no change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub thread_id: Uuid,
    pub running: bool,
}

/// The thread that claims the copy in `.teryon/worktrees/<folder>`, if any.
///
/// - A folder named after a thread without a change belongs to that thread,
///   whenever it is found. That is where Teryon told agents to work before
///   changes had names.
/// - A `new` folder, one Teryon has just noticed, belongs to the one running
///   thread without a change, when there is exactly one. With two, either
///   agent could have made it.
/// - Any other folder belongs to nobody.
pub fn claimant(folder: &str, new: bool, candidates: &[Candidate]) -> Option<Uuid> {
    if let Ok(named) = Uuid::parse_str(folder) {
        if candidates
            .iter()
            .any(|candidate| candidate.thread_id == named)
        {
            return Some(named);
        }
    }
    if !new {
        return None;
    }
    let mut running = candidates.iter().filter(|candidate| candidate.running);
    match (running.next(), running.next()) {
        (Some(only), None) => Some(only.thread_id),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use lets_expect::lets_expect;

    use super::*;

    fn id(number: u128) -> Uuid {
        Uuid::from_u128(number)
    }

    fn running(number: u128) -> Candidate {
        Candidate {
            thread_id: id(number),
            running: true,
        }
    }

    fn idle(number: u128) -> Candidate {
        Candidate {
            thread_id: id(number),
            running: false,
        }
    }

    lets_expect! {
        expect(claimant(folder, new, &candidates)) {
            let folder = "fix-login";
            let new = true;
            let candidates: Vec<Candidate> = Vec::new();

            to go_to_nobody_without_threads { equal(None) }
            when(candidates = vec![running(1)]) as with_one_running_thread to go_to_it { equal(Some(id(1))) }
            when(candidates = vec![running(1), idle(2)]) as with_one_running_and_one_idle to go_to_the_running_one { equal(Some(id(1))) }
            when(candidates = vec![running(1), running(2)]) as with_two_running_threads to go_to_nobody { equal(None) }
            when(candidates = vec![idle(1)]) as with_only_idle_threads to go_to_nobody { equal(None) }

            when(new = false) as a_folder_noticed_before {
                when(candidates = vec![running(1)]) as with_one_running_thread to go_to_nobody { equal(None) }
            }

            when(folder = "00000000-0000-0000-0000-000000000002") as a_folder_named_after_a_thread {
                when(candidates = vec![idle(2)]) as that_is_idle to go_to_it { equal(Some(id(2))) }
                when(candidates = vec![running(1), running(2)]) as among_two_running to go_to_it { equal(Some(id(2))) }
                when(candidates = vec![running(1), idle(3)]) as that_has_a_change_already to go_to_the_running_one { equal(Some(id(1))) }
                when(candidates = vec![idle(3)]) as that_is_not_a_candidate to go_to_nobody { equal(None) }
                when(new = false, candidates = vec![idle(2)]) as noticed_before to still_go_to_it { equal(Some(id(2))) }
            }
        }
    }
}
