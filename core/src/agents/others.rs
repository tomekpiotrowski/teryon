//! What a thread's agent is told about inspecting and managing other threads.
//!
//! A person may ask an agent in one thread to check on, answer, or apply work
//! in other threads across projects. One short sentence points the agent to
//! the command's help without cluttering every turn, and reminds it not to
//! touch other threads unless asked.

/// What the agent of every turn of a thread's own agent is told about other threads.
pub const OTHERS: &str = "When the person asks about other threads or projects, `teryon threads`, `teryon thread --help` and `teryon change --help` show and manage them; don't act on other threads unless asked.";

/// The brief for a turn of the thread's own agent: what it is told already, then [`OTHERS`].
pub fn with_others(brief: Option<String>) -> String {
    match brief {
        Some(brief) => format!("{brief}\n\n{OTHERS}"),
        None => OTHERS.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use lets_expect::lets_expect;

    use super::*;

    lets_expect! {
        expect(OTHERS) as the_brief {
            to name_threads { have(contains("`teryon threads`")) be_true }
            to name_thread_help { have(contains("`teryon thread --help`")) be_true }
            to name_change_help { have(contains("`teryon change --help`")) be_true }
            to say_only_when_asked { have(contains("don't act on other threads unless asked")) be_true }
        }

        expect(with_others(brief)) as the_author_brief {
            let brief = Some("Ask questions.".to_string());

            to come_after_the_rest { equal(format!("Ask questions.\n\n{OTHERS}")) }
            when(brief = None) as be_alone_without_the_rest to hold { equal(OTHERS.to_string()) }
        }
    }
}
