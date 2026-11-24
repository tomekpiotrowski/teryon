//! Resolving thread identifier beginnings and project names from a snapshot.
//!
//! People and agents should not have to copy a full 36-character identifier to
//! inspect or message a thread. Any unique beginning of at least four
//! characters works, as git does with commits, and projects can be named by
//! name or identifier.

use thiserror::Error;
use uuid::Uuid;

use teryon_core::protocol::ProjectView;
use teryon_core::store::Thread;

/// Shortest beginning of a thread identifier accepted in place of the whole thing.
pub const MINIMUM_PREFIX: usize = 4;

/// How many characters of an identifier plain-text listings show.
pub const SHORT_ID_LENGTH: usize = 8;

/// Why a thread or project name could not be resolved.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum NamingError {
    #[error("Give at least {MINIMUM_PREFIX} characters of the thread's identifier.")]
    PrefixTooShort,
    #[error("There is no thread matching `{0}`.")]
    UnknownThread(String),
    #[error("More than one thread starts with `{query}`:\n{matches}")]
    AmbiguousThread { query: String, matches: String },
    #[error("There is no project `{0}`.")]
    UnknownProject(String),
    #[error("More than one project matches `{query}`:\n{matches}")]
    AmbiguousProject { query: String, matches: String },
}

/// First eight characters of an identifier, short enough for tables and lists.
pub fn short_id(id: Uuid) -> String {
    id.to_string().chars().take(SHORT_ID_LENGTH).collect()
}

/// Finds a thread by its full identifier or by a unique beginning of at least four characters.
pub fn resolve_thread<'a>(threads: &'a [Thread], query: &str) -> Result<&'a Thread, NamingError> {
    let query = query.trim();
    if let Ok(id) = Uuid::parse_str(query) {
        return threads
            .iter()
            .find(|thread| thread.id == id)
            .ok_or_else(|| NamingError::UnknownThread(query.to_string()));
    }
    if query.chars().count() < MINIMUM_PREFIX {
        return Err(NamingError::PrefixTooShort);
    }
    let lowered = query.to_ascii_lowercase();
    let mut matched: Vec<&Thread> = threads
        .iter()
        .filter(|thread| thread.id.to_string().starts_with(&lowered))
        .collect();
    match matched.len() {
        0 => Err(NamingError::UnknownThread(query.to_string())),
        1 => Ok(matched[0]),
        _ => {
            matched.sort_by_key(|thread| thread.id);
            let lines = matched
                .iter()
                .map(|thread| {
                    let title = if thread.title.is_empty() {
                        "(untitled)"
                    } else {
                        &thread.title
                    };
                    format!("  {}  {title}", short_id(thread.id))
                })
                .collect::<Vec<_>>()
                .join("\n");
            Err(NamingError::AmbiguousThread {
                query: query.to_string(),
                matches: lines,
            })
        }
    }
}

/// Finds a project by identifier, exact name, case-insensitive name, or unique identifier beginning.
pub fn resolve_project<'a>(
    projects: &'a [ProjectView],
    query: &str,
) -> Result<&'a ProjectView, NamingError> {
    let query = query.trim();
    if query.is_empty() {
        return Err(NamingError::UnknownProject(String::new()));
    }
    if let Ok(id) = Uuid::parse_str(query) {
        return projects
            .iter()
            .find(|project| project.id == id)
            .ok_or_else(|| NamingError::UnknownProject(query.to_string()));
    }
    let exact: Vec<&ProjectView> = projects
        .iter()
        .filter(|project| project.name == query)
        .collect();
    if !exact.is_empty() {
        return single_project(query, exact);
    }
    let insensitive: Vec<&ProjectView> = projects
        .iter()
        .filter(|project| project.name.eq_ignore_ascii_case(query))
        .collect();
    if !insensitive.is_empty() {
        return single_project(query, insensitive);
    }
    if query.chars().count() >= MINIMUM_PREFIX {
        let lowered = query.to_ascii_lowercase();
        let prefixed: Vec<&ProjectView> = projects
            .iter()
            .filter(|project| project.id.to_string().starts_with(&lowered))
            .collect();
        if !prefixed.is_empty() {
            return single_project(query, prefixed);
        }
    }
    Err(NamingError::UnknownProject(query.to_string()))
}

fn single_project<'a>(
    query: &str,
    mut matched: Vec<&'a ProjectView>,
) -> Result<&'a ProjectView, NamingError> {
    if matched.len() == 1 {
        return Ok(matched[0]);
    }
    matched.sort_by_key(|project| project.id);
    let lines = matched
        .iter()
        .map(|project| format!("  {}  {}  {}", short_id(project.id), project.name, project.directory.display()))
        .collect::<Vec<_>>()
        .join("\n");
    Err(NamingError::AmbiguousProject {
        query: query.to_string(),
        matches: lines,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use chrono::Utc;
    use lets_expect::lets_expect;

    use teryon_core::protocol::ProjectState;
    use teryon_core::store::Status;
    use teryon_core::workflow::Stage;

    use super::*;

    fn id(text: &str) -> Uuid {
        Uuid::parse_str(text).unwrap()
    }

    fn thread_with(id: Uuid, project_id: Uuid, title: &str) -> Thread {
        Thread {
            id,
            project_id,
            title: title.to_string(),
            named: !title.is_empty(),
            agent: "claude".to_string(),
            model: None,
            effort: None,
            conversation: None,
            handover: false,
            status: Status::Idle,
            unread: false,
            asking: false,
            commands: Vec::new(),
            change: None,
            finished: None,
            reviewer: None,
            workflow: "default".to_string(),
            stage: Stage::Talking,
            stage_since: None,
            plan: None,
            rounds: 0,
            held: false,
            put_away: false,
            done_seen: false,
            picks: BTreeMap::new(),
            ended: None,
            preview: None,
            created_at: Utc::now(),
        }
    }

    fn project_with(id: Uuid, name: &str) -> ProjectView {
        ProjectView {
            id,
            name: name.to_string(),
            directory: PathBuf::from(format!("/work/{name}")),
            created_at: Utc::now(),
            state: ProjectState::Ready,
            on_github: false,
        }
    }

    const ID_A1: &str = "abcd1111-0000-4000-8000-000000000001";
    const ID_A2: &str = "abcd2222-0000-4000-8000-000000000002";
    const ID_B1: &str = "ef019999-0000-4000-8000-000000000003";
    const PROJ_1: &str = "1111aaaa-0000-4000-8000-000000000001";
    const PROJ_2: &str = "2222bbbb-0000-4000-8000-000000000002";
    const PROJ_3: &str = "2222cccc-0000-4000-8000-000000000003";

    fn sample_threads() -> Vec<Thread> {
        vec![
            thread_with(id(ID_A1), id(PROJ_1), "First"),
            thread_with(id(ID_A2), id(PROJ_1), "Second"),
            thread_with(id(ID_B1), id(PROJ_2), "Other project"),
        ]
    }

    fn sample_projects() -> Vec<ProjectView> {
        vec![
            project_with(id(PROJ_1), "teryon"),
            project_with(id(PROJ_2), "www"),
            project_with(id(PROJ_3), "Teryon"),
        ]
    }

    fn resolved_thread(threads: &[Thread], input: &str) -> Result<Uuid, NamingError> {
        resolve_thread(threads, input).map(|thread| thread.id)
    }

    fn resolved_project(projects: &[ProjectView], input: &str) -> Result<Uuid, NamingError> {
        resolve_project(projects, input).map(|project| project.id)
    }

    fn ambiguous_abcd() -> NamingError {
        NamingError::AmbiguousThread {
            query: "abcd".to_string(),
            matches: "  abcd1111  First\n  abcd2222  Second".to_string(),
        }
    }

    lets_expect! {
        expect(resolved_thread(&threads, input)) as resolving_a_thread {
            let threads = sample_threads();
            let input = ID_A1;

            to find_by_full_identifier { equal(Ok(id(ID_A1))) }
            when(input = "ef01") as unique_four_char_prefix_in_another_project to find_it { equal(Ok(id(ID_B1))) }
            when(input = "ABCD1") as uppercase_prefix to find_it { equal(Ok(id(ID_A1))) }
            when(input = "abcd") as ambiguous_prefix to refuse_with_both { equal(Err(ambiguous_abcd())) }
            when(input = "abc") as too_short to refuse { equal(Err(NamingError::PrefixTooShort)) }
            when(input = "") as empty to refuse { equal(Err(NamingError::PrefixTooShort)) }
            when(input = "9999") as unknown_prefix to refuse { equal(Err(NamingError::UnknownThread("9999".to_string()))) }
            when(input = "00000000-0000-4000-8000-000000000000") as unknown_uuid to refuse {
                equal(Err(NamingError::UnknownThread("00000000-0000-4000-8000-000000000000".to_string())))
            }
        }

        expect(resolved_project(&projects, input)) as resolving_a_project {
            let projects = sample_projects();
            let input = "teryon";

            to prefer_exact_case { equal(Ok(id(PROJ_1))) }
            when(input = "WWW") as case_insensitive_name to find_it { equal(Ok(id(PROJ_2))) }
            when(input = "TERYON") as ambiguous_case_insensitive_name to refuse { be_err }
            when(input = PROJ_2) as full_identifier to find_it { equal(Ok(id(PROJ_2))) }
            when(input = "1111") as unique_prefix to find_it { equal(Ok(id(PROJ_1))) }
            when(input = "2222") as ambiguous_prefix to refuse { be_err }
            when(input = "missing") as unknown_name to refuse { equal(Err(NamingError::UnknownProject("missing".to_string()))) }
            when(input = "") as empty to refuse { equal(Err(NamingError::UnknownProject(String::new()))) }
        }
    }
}
