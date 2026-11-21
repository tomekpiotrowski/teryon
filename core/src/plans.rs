//! A thread's plan: one markdown file per thread, in a place Teryon knows.
//!
//! Approving a plan is where the person decides what gets built, and with
//! automatic starts, where another agent may take over. It has to be about
//! something the person can read, and something the next agent can be
//! pointed at. So each thread has one plan file, `plan.md`, next to its
//! conversation in `.teryon/threads/<thread>/`, never in the project's
//! history. The agent is told the path and writes the file with its own
//! tools; Teryon looks at it after each turn and says in the thread when it
//! was written or changed.
//!
//! Only the current plan is kept. A fingerprint of the file tells Teryon
//! when it changed.

use std::fs::read;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::warn;
use ts_rs::TS;
use uuid::Uuid;

/// The largest plan Teryon shows. A larger file is noted in the thread, but not shown.
pub const LARGEST_PLAN: usize = 1024 * 1024;

/// What Teryon last saw of a thread's plan.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct PlanInfo {
    /// The plan's first heading, or "Plan" without one.
    pub title: String,
    /// Tells a changed file from the same one; not meant for the person.
    pub fingerprint: String,
    /// Whether the file is larger than Teryon shows.
    #[serde(default)]
    pub too_large: bool,
}

/// Where a thread's plan is, in a project.
pub fn plan_path(project: &Path, thread_id: Uuid) -> PathBuf {
    project
        .join(".teryon")
        .join("threads")
        .join(thread_id.to_string())
        .join("plan.md")
}

/// What a plan file holds now. Nothing when there is none, or it can't be read.
///
/// A file that can't be read for another reason than being missing is
/// logged here; the thread simply shows no change.
pub fn read_plan(path: &Path) -> Option<PlanInfo> {
    let bytes = match read(path) {
        Ok(bytes) => bytes,
        Err(problem) if problem.kind() == ErrorKind::NotFound => return None,
        Err(problem) => {
            warn!("could not read the plan at {}: {problem}", path.display());
            return None;
        }
    };
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return None;
    }
    let fingerprint = Sha256::digest(&bytes)[..12]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let too_large = bytes.len() > LARGEST_PLAN;
    let title = match too_large {
        true => "Plan".to_string(),
        false => title_of(&String::from_utf8_lossy(&bytes)),
    };
    Some(PlanInfo {
        title,
        fingerprint,
        too_large,
    })
}

/// The plan's first markdown heading, without its marks, or "Plan" when it has none.
pub fn title_of(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| line.starts_with('#'))
        .map(|line| line.trim_start_matches('#').trim().to_string())
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| "Plan".to_string())
}

/// The line in the thread for a plan written for the first time, or changed.
pub fn plan_line(plan: &PlanInfo, first: bool) -> String {
    let what = if first {
        "Plan written"
    } else {
        "Plan changed"
    };
    match plan.too_large {
        true => format!("{what}, but it is larger than 1 MB, too large to show"),
        false => format!("{what} · {}", plan.title),
    }
}

#[cfg(test)]
mod tests {
    use std::fs::write;

    use lets_expect::lets_expect;
    use tempfile::tempdir;

    use super::*;

    fn reading(content: Option<String>) -> Option<(String, bool)> {
        let folder = tempdir().unwrap();
        let path = folder.path().join("plan.md");
        if let Some(content) = content {
            write(&path, content).unwrap();
        }
        read_plan(&path).map(|plan| (plan.title, plan.too_large))
    }

    fn too_large() -> String {
        "a".repeat(LARGEST_PLAN + 1)
    }

    fn fingerprints_differ(first: &str, second: &str) -> bool {
        let folder = tempdir().unwrap();
        let path = folder.path().join("plan.md");
        write(&path, first).unwrap();
        let before = read_plan(&path).unwrap().fingerprint;
        write(&path, second).unwrap();
        before != read_plan(&path).unwrap().fingerprint
    }

    fn info(title: &str, too_large: bool) -> PlanInfo {
        PlanInfo {
            title: title.to_string(),
            fingerprint: String::new(),
            too_large,
        }
    }

    lets_expect! {
        expect(plan_path(Path::new("/work/app"), Uuid::nil())) as the_path_of_a_plan {
            to be_next_to_the_thread { equal(PathBuf::from("/work/app/.teryon/threads/00000000-0000-0000-0000-000000000000/plan.md")) }
        }

        expect(title_of(text)) as the_title_of_a_plan {
            when(text = "# Thread plans\n\nText") as a_heading_first to be_it { equal("Thread plans".to_string()) }
            when(text = "Intro\n\n## Later heading\n# Another") as a_heading_after_text to be_the_first_heading { equal("Later heading".to_string()) }
            when(text = "Just text") as no_heading to be_plan { equal("Plan".to_string()) }
            when(text = "#\n# Real") as an_empty_heading to be_skipped { equal("Plan".to_string()) }
        }

        expect(reading(content)) as reading_a_plan {
            when(content = None) as a_missing_file to be_nothing { equal(None) }
            when(content = Some("  \n\n".to_string())) as an_emptied_file to be_nothing { equal(None) }
            when(content = Some("# Plan A\n".to_string())) as a_written_file to give_its_title { equal(Some(("Plan A".to_string(), false))) }
            when(content = Some(too_large())) as a_file_too_large to say_so { equal(Some(("Plan".to_string(), true))) }
        }

        expect(fingerprints_differ(first, second)) as telling_changes_apart {
            when(first = "# A\n", second = "# A\n") as the_same_text to be_the_same { be_false }
            when(first = "# A\n", second = "# A\nmore\n") as a_changed_text to differ { be_true }
        }

        expect(plan_line(&plan, first)) as the_line_in_the_thread {
            when(plan = info("Thread plans", false), first = true) as a_first_plan to say_written { equal("Plan written · Thread plans".to_string()) }
            when(plan = info("Thread plans", false), first = false) as a_changed_plan to say_changed { equal("Plan changed · Thread plans".to_string()) }
            when(plan = info("Plan", true), first = true) as a_plan_too_large to say_so { equal("Plan written, but it is larger than 1 MB, too large to show".to_string()) }
        }
    }
}
