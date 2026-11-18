//! The list of projects, kept in `projects.json` in Teryon's data folder.

use std::fs::{create_dir_all, read_to_string};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use uuid::Uuid;

use crate::store::{replace_file, Project, StoreError};

/// The projects Teryon knows, in the order they were added.
pub struct ProjectList {
    path: PathBuf,
    projects: Vec<Project>,
}

impl ProjectList {
    /// Reads the list kept in `folder`. A folder without one has no projects yet.
    pub fn open(folder: &Path) -> Result<Self, StoreError> {
        let path = folder.join("projects.json");
        let projects = match read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).map_err(|source| StoreError::Unreadable {
                path: path.clone(),
                source,
            })?,
            Err(problem) if problem.kind() == ErrorKind::NotFound => Vec::new(),
            Err(source) => return Err(StoreError::Io { path, source }),
        };
        Ok(Self { path, projects })
    }

    pub fn projects(&self) -> &[Project] {
        &self.projects
    }

    pub fn add(&mut self, project: Project) -> Result<(), StoreError> {
        self.projects.push(project);
        self.save()
    }

    /// Forgets a project. Returns whether it was there.
    pub fn remove(&mut self, id: Uuid) -> Result<bool, StoreError> {
        let before = self.projects.len();
        self.projects.retain(|project| project.id != id);
        if self.projects.len() == before {
            return Ok(false);
        }
        self.save().map(|()| true)
    }

    fn save(&self) -> Result<(), StoreError> {
        if let Some(folder) = self.path.parent() {
            create_dir_all(folder).map_err(StoreError::io(folder))?;
        }
        let text = serde_json::to_string_pretty(&self.projects).map_err(|source| {
            StoreError::Unreadable {
                path: self.path.clone(),
                source,
            }
        })?;
        replace_file(&self.path, &text)
    }
}

#[cfg(test)]
mod tests {
    use std::fs::write;

    use chrono::Utc;
    use lets_expect::lets_expect;
    use tempfile::tempdir;
    use uuid::Uuid;

    use super::*;

    fn project(name: &str) -> Project {
        Project {
            id: Uuid::new_v4(),
            name: name.to_string(),
            directory: PathBuf::from(format!("/work/{name}")),
            created_at: Utc::now(),
        }
    }

    fn names(list: &ProjectList) -> Vec<String> {
        list.projects()
            .iter()
            .map(|project| project.name.clone())
            .collect()
    }

    lets_expect! {
        expect(names(&ProjectList::open(&folder).unwrap())) as the_list_read_back {
            let root = tempdir().unwrap();
            let folder = root.path().to_path_buf();

            when nothing_was_ever_added to be_empty { equal(Vec::<String>::new()) }

            when two_projects_were_added {
                before {
                    let mut list = ProjectList::open(&folder).unwrap();
                    list.add(project("teryon")).unwrap();
                    list.add(project("erno")).unwrap();
                }
                to keep_them_in_order { equal(vec!["teryon".to_string(), "erno".to_string()]) }
            }

            when one_of_two_was_removed {
                before {
                    let mut list = ProjectList::open(&folder).unwrap();
                    let first = project("teryon");
                    list.add(first.clone()).unwrap();
                    list.add(project("erno")).unwrap();
                    list.remove(first.id).unwrap();
                }
                to keep_the_other { equal(vec!["erno".to_string()]) }
            }

            when the_data_folder_does_not_exist_yet {
                let folder = root.path().join("deeper/still");
                before { ProjectList::open(&folder).unwrap().add(project("teryon")).unwrap(); }
                to create_it { equal(vec!["teryon".to_string()]) }
            }
        }

        expect(ProjectList::open(root.path()).unwrap().remove(Uuid::new_v4())) as removing_an_unknown_project {
            let root = tempdir().unwrap();
            to say_it_was_not_there { be_ok_and equal(false) }
        }

        expect(ProjectList::open(root.path()).is_err()) as reading_a_damaged_list {
            let root = tempdir().unwrap();
            before { write(root.path().join("projects.json"), "[{ not json").unwrap(); }
            to fail_rather_than_forget_every_project { be_true }
        }
    }
}
