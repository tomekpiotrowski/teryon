//! Opening a thread's copy in another program: the file manager, an editor, a terminal.
//!
//! Sometimes a person wants to look at or work on a change with their own
//! tools. The file manager is always there; editors and terminals differ
//! from person to person, so they are commands in `config.toml`.
//!
//! A configured command is split on spaces and started directly, with no
//! shell in between, so a path with spaces or quotes in it stays one
//! argument and cannot be read as anything else.

use std::path::Path;

/// A program to start, and what to give it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandLine {
    pub program: String,
    pub arguments: Vec<String>,
}

/// The configured command with `{path}` replaced by `path`. Nothing for a command with no program in it.
///
/// A command without `{path}` gets the path as its last argument.
pub fn command_line(template: &str, path: &Path) -> Option<CommandLine> {
    let path = path.to_string_lossy();
    let mut words = template.split_whitespace();
    let program = words.next()?.to_string();
    let mut arguments: Vec<String> = words.map(|word| word.replace("{path}", &path)).collect();
    if !template.contains("{path}") {
        arguments.push(path.into_owned());
    }
    Some(CommandLine { program, arguments })
}

/// The system's own way of opening a folder.
pub fn file_manager(path: &Path) -> CommandLine {
    let program = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(windows) {
        "explorer"
    } else {
        "xdg-open"
    };
    CommandLine {
        program: program.to_string(),
        arguments: vec![path.to_string_lossy().into_owned()],
    }
}

#[cfg(test)]
mod tests {
    use lets_expect::lets_expect;

    use super::*;

    fn line(program: &str, arguments: &[&str]) -> Option<CommandLine> {
        Some(CommandLine {
            program: program.to_string(),
            arguments: arguments
                .iter()
                .map(|argument| argument.to_string())
                .collect(),
        })
    }

    lets_expect! {
        expect(command_line(template, Path::new(path))) {
            let template = "code {path}";
            let path = "/work/app/.teryon/worktrees/fix-login";

            to put_the_path_in_its_place { equal(line("code", &["/work/app/.teryon/worktrees/fix-login"])) }
            when(path = "/work/my app/it's here") as a_path_with_spaces_and_quotes to keep_it_one_argument {
                equal(line("code", &["/work/my app/it's here"]))
            }
            when(template = "alacritty --working-directory {path}") as an_option_before_the_path to keep_it {
                equal(line("alacritty", &["--working-directory", "/work/app/.teryon/worktrees/fix-login"]))
            }
            when(template = "kitty --directory={path}") as the_path_inside_a_word to replace_it_there {
                equal(line("kitty", &["--directory=/work/app/.teryon/worktrees/fix-login"]))
            }
            when(template = "zed") as no_path_named to add_it_at_the_end { equal(line("zed", &["/work/app/.teryon/worktrees/fix-login"])) }
            when(template = "  nvim   -p  {path} ") as extra_spaces to be_ignored { equal(line("nvim", &["-p", "/work/app/.teryon/worktrees/fix-login"])) }
            when(template = "") as an_empty_command to give_nothing { equal(None) }
            when(template = "   ") as only_spaces to give_nothing { equal(None) }
        }
    }
}
