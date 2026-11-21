//! What an agent taking over a thread from another agent is told.
//!
//! A person switches a thread to another agent when the first one cannot go
//! on, for example because its subscription ran out. A conversation lives
//! inside one agent program and cannot move to another, so the new agent
//! starts a conversation of its own. To carry on where the other left off it
//! needs what was said, so its first message carries the thread's messages
//! so far: the person's and the agent's, leaving out the tools it ran, since
//! what those did is in the files.
//!
//! Questions the agent asked the person are handed over too, written out as
//! text, so a person who answers after switching agents is understood.
//!
//! Only one side of the thread is handed over: the author's lines to a new
//! author, the reviewer's lines to a new reviewer, so the reviewer keeps
//! judging the change on its own.

use crate::agents::asking::{Choosing, Question};
use crate::agents::{EntryKind, Voice};
use crate::store::Entry;

/// How many bytes of earlier messages are handed over at most.
///
/// A long thread would otherwise fill much of the new agent's context. The
/// newest messages matter most, so the oldest are left out first. Every
/// agent program gets its message as one command-line argument, which Linux
/// caps at 128 KiB, so the handover stays well under that.
const LONGEST_HANDOVER: usize = 60_000;

/// How many bytes of one message are handed over at most.
///
/// A single huge reply, such as a pasted log, would otherwise crowd out
/// every other message, or alone break the argument limit. Its start is
/// kept, as the part that usually says what it is about.
const LONGEST_MESSAGE: usize = 10_000;

/// The message for an agent taking over a thread: the thread so far, then the person's new message.
///
/// `entries` are the thread's lines before the new message. Without any
/// earlier messages on this side of the thread the message goes as it is.
pub fn handed_over(entries: &[Entry], voice: Voice, message: &str) -> String {
    let lines: Vec<String> = entries
        .iter()
        .filter(|entry| (entry.body["reviewer"] == true) == (voice == Voice::Reviewer))
        .filter_map(said)
        .collect();
    if lines.is_empty() {
        return message.to_string();
    }

    let mut kept = Vec::new();
    let mut length = 0;
    for line in lines.iter().rev() {
        if length + line.len() > LONGEST_HANDOVER {
            break;
        }
        length += line.len();
        kept.push(line.as_str());
    }
    kept.reverse();
    let left_out = if kept.len() < lines.len() {
        "(Earlier messages are left out.)\n\n"
    } else {
        ""
    };
    let conversation = kept.join("\n\n");
    format!(
        "You are taking over this thread from another agent, which can no longer work on it. \
         Here is the conversation so far: the person's messages and the other agent's replies. \
         What it did is in the files.\n\n\
         <conversation>\n{left_out}{conversation}\n</conversation>\n\n\
         The person's new message:\n\n{message}"
    )
}

/// A message in the conversation, as a line of the handover. Nothing for the agent's tools and Teryon's remarks.
fn said(entry: &Entry) -> Option<String> {
    if entry.kind == EntryKind::Questions.name() {
        return asked(entry);
    }
    let speaker = match entry.kind.as_str() {
        kind if kind == EntryKind::UserMessage.name() => "Person",
        kind if kind == EntryKind::AgentMessage.name() => "Agent",
        _ => return None,
    };
    let text = entry.body["text"].as_str()?.trim();
    if text.is_empty() {
        return None;
    }
    if text.len() <= LONGEST_MESSAGE {
        return Some(format!("{speaker}: {text}"));
    }
    let mut end = LONGEST_MESSAGE;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    Some(format!(
        "{speaker}: {} (The rest is left out.)",
        text[..end].trim_end()
    ))
}

/// Questions the agent asked, as a line of the handover: each with its answers, the recommended ones marked.
///
/// Nothing when the entry doesn't hold questions in the shape Teryon stores.
fn asked(entry: &Entry) -> Option<String> {
    let questions: Vec<Question> = serde_json::from_value(entry.body["questions"].clone()).ok()?;
    let lines: Vec<String> = questions
        .iter()
        .enumerate()
        .map(|(index, question)| {
            let answers: Vec<String> = question
                .answers
                .iter()
                .enumerate()
                .map(|(answer, choice)| {
                    let recommended = match &question.choosing {
                        Choosing::One { recommended } => *recommended == answer,
                        Choosing::Several { recommended } => recommended.contains(&answer),
                    };
                    if recommended {
                        format!("{} (recommended)", choice.label)
                    } else {
                        choice.label.clone()
                    }
                })
                .collect();
            let several = match question.choosing {
                Choosing::One { .. } => "",
                Choosing::Several { .. } => ", any of",
            };
            format!(
                "{}. {}{several}: {}",
                index + 1,
                question.question,
                answers.join(" / ")
            )
        })
        .collect();
    (!lines.is_empty()).then(|| format!("Agent asked the person:\n{}", lines.join("\n")))
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use lets_expect::lets_expect;
    use serde_json::{json, Value};
    use uuid::Uuid;

    use super::*;

    fn entry(kind: &str, body: Value) -> Entry {
        Entry {
            id: Uuid::nil(),
            thread_id: Uuid::nil(),
            position: 1,
            kind: kind.to_string(),
            body,
            created_at: Utc::now(),
        }
    }

    fn asked_body() -> Value {
        json!({
            "turn": Uuid::nil(),
            "questions": [
                { "question": "Where?", "answers": [{ "label": "Here", "description": null }, { "label": "There", "description": null }], "choosing": { "kind": "one", "recommended": 1 } },
                { "question": "Which", "answers": [{ "label": "A", "description": null }, { "label": "B", "description": null }, { "label": "C", "description": null }], "choosing": { "kind": "several", "recommended": [0, 2] } },
            ],
        })
    }

    fn conversation() -> Vec<Entry> {
        vec![
            entry("user_message", json!({ "text": "Fix the login." })),
            entry(
                "activity",
                json!({ "tool": "Bash", "summary": "cargo test" }),
            ),
            entry("agent_message", json!({ "text": "Fixed it." })),
            entry("turn_summary", json!({ "tokens": null })),
            entry(
                "notice",
                json!({ "text": "Codex reviews the change.", "reviewer": true }),
            ),
            entry(
                "agent_message",
                json!({ "text": "One problem.", "reviewer": true, "agent": "codex" }),
            ),
            entry("error", json!({ "text": "Limit hit." })),
        ]
    }

    lets_expect! {
        expect(handed_over(&entries, voice, "Go on.")) as the_message {
            let entries = conversation();
            let voice = Voice::Author;

            when(voice = Voice::Author) as to_a_new_author {
                to carry_the_authors_messages { have(contains("<conversation>\nPerson: Fix the login.\n\nAgent: Fixed it.\n</conversation>")) be_true }
                to leave_out_the_reviewers { have(contains("One problem.")) be_false }
                to leave_out_tools { have(contains("cargo test")) be_false }
                to leave_out_errors { have(contains("Limit hit.")) be_false }
                to end_with_the_new_message { have(ends_with("The person's new message:\n\nGo on.")) be_true }
                to say_it_takes_over { have(starts_with("You are taking over this thread")) be_true }
            }
            when(voice = Voice::Reviewer) as to_a_new_reviewer {
                to carry_the_reviewers_messages { have(contains("<conversation>\nAgent: One problem.\n</conversation>")) be_true }
                to leave_out_the_authors { have(contains("Fix the login.")) be_false }
            }
            when(entries = vec![entry("questions", asked_body())]) as of_a_thread_with_questions {
                to carry_them_with_recommendations_marked {
                    have(contains("<conversation>\nAgent asked the person:\n1. Where?: Here / There (recommended)\n2. Which, any of: A (recommended) / B / C (recommended)\n</conversation>")) be_true
                }
            }
            when(entries = vec![entry("questions", json!({ "questions": "Where?" }))]) as of_a_thread_with_unreadable_questions {
                to go_as_it_is { equal("Go on.".to_string()) }
            }
            when(entries = Vec::new()) as of_a_thread_with_nothing_said {
                to go_as_it_is { equal("Go on.".to_string()) }
            }
            when(entries = vec![entry("activity", json!({ "tool": "Bash" })), entry("agent_message", json!({ "text": "  " }))]) as of_a_thread_with_no_messages {
                to go_as_it_is { equal("Go on.".to_string()) }
            }
            when(voice = Voice::Reviewer, entries = conversation()[..4].to_vec()) as to_a_reviewer_of_a_thread_never_reviewed {
                to go_as_it_is { equal("Go on.".to_string()) }
            }
            when(entries = vec![
                entry("user_message", json!({ "text": "old ".repeat(LONGEST_MESSAGE / 4) })),
                entry("agent_message", json!({ "text": "middle ".repeat(LONGEST_MESSAGE / 7) })),
                entry("agent_message", json!({ "text": "middle ".repeat(LONGEST_MESSAGE / 7) })),
                entry("agent_message", json!({ "text": "middle ".repeat(LONGEST_MESSAGE / 7) })),
                entry("agent_message", json!({ "text": "middle ".repeat(LONGEST_MESSAGE / 7) })),
                entry("agent_message", json!({ "text": "middle ".repeat(LONGEST_MESSAGE / 7) })),
                entry("agent_message", json!({ "text": "middle ".repeat(LONGEST_MESSAGE / 7) })),
                entry("agent_message", json!({ "text": "Newest." })),
            ]) as of_a_long_thread {
                to keep_the_newest { have(contains("Agent: Newest.")) be_true }
                to leave_out_the_oldest { have(contains("old old")) be_false }
                to say_some_are_left_out { have(contains("(Earlier messages are left out.)")) be_true }
            }
            when(entries = vec![entry("user_message", json!({ "text": "long ".repeat(LONGEST_HANDOVER) }))]) as of_one_message_longer_than_the_limit {
                to keep_its_start { have(contains("Person: long long")) be_true }
                to say_the_rest_is_left_out { have(contains("long (The rest is left out.)")) be_true }
                to stay_bounded { have(len() < LONGEST_MESSAGE + 1_000) be_true }
            }
            when(entries = vec![entry("agent_message", json!({ "text": "ż".repeat(LONGEST_MESSAGE) }))]) as of_a_long_message_in_letters_of_two_bytes {
                to cut_between_letters { have(contains("żż (The rest is left out.)")) be_true }
            }
            when(entries = vec![entry("agent_message", json!({ "text": "many ".repeat(LONGEST_MESSAGE) })); 100]) as of_many_long_messages {
                to stay_under_the_limit { have(len() < LONGEST_HANDOVER + 1_000) be_true }
                to keep_some { have(contains("Agent: many")) be_true }
            }
        }
    }
}
