//! What a thread's agent is told about naming the thread.
//!
//! A person finds a thread in a list by its title. Until the agent names it,
//! the title is the start of the first message, which is often a greeting or
//! background rather than the point. The agent has read the whole message,
//! so it is asked to name the thread through `teryon thread title`, the same
//! way it names its worktree. Teryon itself talks to no model.
//!
//! The request goes with every turn of the thread's agent until the thread
//! has a title someone chose, so an agent that skipped it once is asked again.

/// What the agent of a thread without a chosen title is told.
pub const NAMING: &str = concat!(
    "This thread has no title yet. Before anything else, run `teryon thread title \"<title>\"`, ",
    "where `<title>` says in three to six words what the person wants, so they can find this thread among others, ",
    "such as `Fix the login redirect`. Write it in the language of the person's message. "
);

/// The brief for a turn of the thread's own agent: the naming request when the thread needs one, then the rest.
pub fn with_naming(named: bool, rest: Option<String>) -> Option<String> {
    match (named, rest) {
        (true, rest) => rest,
        (false, Some(rest)) => Some(format!("{NAMING}{rest}")),
        (false, None) => Some(NAMING.trim_end().to_string()),
    }
}

#[cfg(test)]
mod tests {
    use lets_expect::lets_expect;

    use super::*;

    lets_expect! {
        expect(NAMING) {
            to name_the_command { have(contains("`teryon thread title \"<title>\"`")) be_true }
            to give_an_example_title { have(contains("`Fix the login redirect`")) be_true }
            to ask_for_the_language_of_the_message { have(contains("in the language of the person's message")) be_true }
        }

        expect(with_naming(named, rest)) {
            let rest = Some("Work in the worktree.".to_string());

            when(named = true) as a_named_thread {
                to keep_the_rest { equal(Some("Work in the worktree.".to_string())) }
            }
            when(named = true, rest = None) as a_named_thread_with_nothing_else {
                to have_no_brief { equal(None) }
            }
            when(named = false) as an_unnamed_thread {
                to ask_for_a_title_first { equal(Some(format!("{NAMING}Work in the worktree."))) }
            }
            when(named = false, rest = None) as an_unnamed_thread_outside_a_repository {
                to ask_for_a_title_alone { equal(Some(NAMING.trim_end().to_string())) }
            }
        }
    }
}
