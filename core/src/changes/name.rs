//! What a change may be called.
//!
//! The agent picks the name, and it becomes both a folder in
//! `.teryon/worktrees/` and a branch. So it has to be safe as either, and
//! readable enough for a person who needs the folder's location: lowercase
//! words joined by single dashes.

use thiserror::Error;

/// The longest name allowed.
pub const LONGEST_NAME: usize = 48;

/// A name that cannot be used. The text is meant for the agent.
#[derive(Debug, Error, PartialEq, Eq)]
#[error(
    "A worktree name is 1 to 48 lowercase letters, digits and single dashes, starting with a letter, such as `fix-login-redirect`."
)]
pub struct BadName;

/// Whether `name` may name a change.
///
/// The rules also rule out anything git or a file system would read
/// specially: no slashes, dots, spaces, or a leading dash that would look like an option.
pub fn check_name(name: &str) -> Result<(), BadName> {
    let starts_with_letter = name
        .chars()
        .next()
        .is_some_and(|first| first.is_ascii_lowercase());
    let allowed = name.chars().all(|character| {
        character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
    });
    let fits = name.len() <= LONGEST_NAME;
    let dashes_between_words = !name.contains("--") && !name.ends_with('-');
    if starts_with_letter && allowed && fits && dashes_between_words {
        Ok(())
    } else {
        Err(BadName)
    }
}

/// A name like `name`, followed by the first free number, that `taken` does not hold.
///
/// The name is shortened when needed, so that the suggestion still follows
/// the rules and can be used as it is.
pub fn free_name(name: &str, taken: impl Fn(&str) -> bool) -> String {
    (2..)
        .map(|number| {
            let ending = format!("-{number}");
            let room = LONGEST_NAME - ending.len();
            let start: String = name.chars().take(room).collect();
            format!("{}{ending}", start.trim_end_matches('-'))
        })
        .find(|candidate| !taken(candidate))
        .expect("some number is free")
}

#[cfg(test)]
mod tests {
    use lets_expect::lets_expect;

    use super::*;

    fn taken_up_to(last: u32) -> impl Fn(&str) -> bool {
        move |candidate: &str| (2..=last).any(|number| candidate.ends_with(&format!("-{number}")))
    }

    lets_expect! {
        expect(check_name(name)) {
            when(name = "fix-login-redirect") as words_joined_by_dashes to be_accepted { equal(Ok(())) }
            when(name = "a") as one_letter to be_accepted { equal(Ok(())) }
            when(name = "v2-upgrade") as digits_after_the_first_letter to be_accepted { equal(Ok(())) }
            when(name = "a".repeat(48).leak()) as exactly_the_longest to be_accepted { equal(Ok(())) }
            when(name = "a".repeat(49).leak()) as one_longer to be_refused { equal(Err(BadName)) }
            when(name = "") as an_empty_name to be_refused { equal(Err(BadName)) }
            when(name = "2fast") as a_leading_digit to be_refused { equal(Err(BadName)) }
            when(name = "-fix") as a_leading_dash to be_refused { equal(Err(BadName)) }
            when(name = "fix-") as a_trailing_dash to be_refused { equal(Err(BadName)) }
            when(name = "fix--login") as a_double_dash to be_refused { equal(Err(BadName)) }
            when(name = "Fix-login") as a_capital to be_refused { equal(Err(BadName)) }
            when(name = "fix login") as a_space to be_refused { equal(Err(BadName)) }
            when(name = "fix_login") as an_underscore to be_refused { equal(Err(BadName)) }
            when(name = "fix/login") as a_slash to be_refused { equal(Err(BadName)) }
            when(name = "fix.login") as a_dot to be_refused { equal(Err(BadName)) }
            when(name = "..") as dots_alone to be_refused { equal(Err(BadName)) }
            when(name = "zażółć") as letters_beyond_ascii to be_refused { equal(Err(BadName)) }
        }

        expect(free_name(name, taken_up_to(last))) {
            let name = "fix-login";
            let last = 1;

            to add_two_when_nothing_is_taken { equal("fix-login-2".to_string()) }
            when(last = 3) as when_two_and_three_are_taken to add_four { equal("fix-login-4".to_string()) }
            when(name = "a".repeat(48).leak()) as for_the_longest_name {
                to shorten_it_to_fit { equal(format!("{}-2", "a".repeat(46))) }
            }
            when(name = format!("{}-b", "a".repeat(45)).leak()) as when_shortening_leaves_a_dash {
                to drop_the_dash { equal(format!("{}-2", "a".repeat(45))) }
            }
        }

        expect(check_name(&free_name(name, taken_up_to(9)))) as a_suggestion {
            when(name = "a".repeat(48).leak()) as for_the_longest_name to follow_the_rules { equal(Ok(())) }
            when(name = "fix") as for_a_short_name to follow_the_rules { equal(Ok(())) }
        }
    }
}
