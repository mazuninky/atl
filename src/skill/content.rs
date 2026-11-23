//! The skill as the binary carries it.
//!
//! `skill/SKILL.md` and `skill/references/*.md`, embedded at compile time;
//! `skill/evals/` stays in the repository. A missing entry would leave the
//! file out of every installation without a word, so a test compares
//! [`FILES`] with the directory. Keep the list sorted by path.

/// One file of the skill.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkillFile {
    /// Path inside the skill directory, components separated by `/`.
    pub path: &'static str,
    /// What the file holds; written as it is, byte for byte.
    pub text: &'static str,
}

/// Every file of the embedded skill.
const FILES: &[SkillFile] = &[
    SkillFile {
        path: "SKILL.md",
        text: include_str!("../../skill/SKILL.md"),
    },
    SkillFile {
        path: "references/body-formats.md",
        text: include_str!("../../skill/references/body-formats.md"),
    },
    SkillFile {
        path: "references/confluence-commands.md",
        text: include_str!("../../skill/references/confluence-commands.md"),
    },
    SkillFile {
        path: "references/jira-commands.md",
        text: include_str!("../../skill/references/jira-commands.md"),
    },
];

/// The skill of one atl version: what `install` and `update` write.
///
/// A value, not a global, so that a test can stand for an older or a newer
/// atl.
#[derive(Debug, Clone, Copy)]
pub struct Edition {
    /// The atl version the files belong to.
    pub version: &'static str,
    /// The files, sorted by path.
    pub files: &'static [SkillFile],
}

impl Edition {
    /// The skill this binary carries.
    #[must_use]
    pub fn embedded() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION"),
            files: FILES,
        }
    }

    /// What this edition has at `path`, `None` when it has no such file.
    #[must_use]
    pub fn text(&self, path: &str) -> Option<&'static str> {
        self.files
            .iter()
            .find(|file| file.path == path)
            .map(|file| file.text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use std::path::{Path, PathBuf};

    fn skill_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("skill")
    }

    /// Every file under `dir`, but in the directories `skip` names, as
    /// `/`-separated paths, sorted.
    fn files_in(dir: &Path, skip: &[&str]) -> Vec<String> {
        let mut found = Vec::new();
        let mut todo = vec![(dir.to_path_buf(), String::new())];
        while let Some((at, prefix)) = todo.pop() {
            for entry in std::fs::read_dir(&at).expect("read dir") {
                let entry = entry.expect("entry");
                let name = entry.file_name().into_string().expect("UTF-8 name");
                if skip.contains(&name.as_str()) {
                    continue;
                }
                let path = format!("{prefix}{name}");
                if entry.file_type().expect("type").is_dir() {
                    todo.push((entry.path(), format!("{path}/")));
                } else {
                    found.push(path);
                }
            }
        }
        found.sort();
        found
    }

    /// Every file of the skill but `evals/`, whatever its name and however
    /// deep, byte for byte.
    #[test]
    fn the_embedded_files_are_those_of_the_directory() {
        let skill = skill_dir();
        let on_disk = files_in(&skill, &["evals", ".DS_Store"]);
        let edition = Edition::embedded();
        let embedded: Vec<&str> = edition.files.iter().map(|file| file.path).collect();
        assert_eq!(embedded, on_disk);
        assert!(embedded.is_sorted(), "{embedded:?}");
        for file in edition.files {
            let bytes = std::fs::read(skill.join(file.path)).expect("skill file");
            assert_eq!(file.text.as_bytes(), bytes, "{}", file.path);
        }
    }

    /// What is installed names no other way to install it.
    #[test]
    fn the_skill_names_no_other_installer() {
        let skill = skill_dir();
        for path in files_in(&skill, &["evals", ".DS_Store"]) {
            assert_ne!(path, "skill.json");
            let text = std::fs::read_to_string(skill.join(&path))
                .expect("text")
                .to_lowercase();
            for other in ["npx", "skills.sh", "skills add"] {
                assert!(!text.contains(other), "{path}: {other}");
            }
        }
    }

    /// The front matter by which an agent finds the skill, under the name of
    /// its directory.
    #[test]
    fn the_skill_has_the_name_of_its_directory() {
        let text = Edition::embedded().text("SKILL.md").expect("SKILL.md");
        let text = text.replace("\r\n", "\n");
        let front = text.strip_prefix("---\n").expect("front matter");
        let front = front.split("\n---\n").next().expect("its end");
        assert!(front.lines().any(|line| line == "name: atl"), "{front}");
        assert!(
            front.lines().any(|line| line.starts_with("description:")),
            "{front}"
        );
    }

    /// The compiler needs `skill/` next to `src/`, also in the image, and
    /// before it runs.
    #[test]
    fn the_image_is_built_with_the_skill() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let read = |name: &str| std::fs::read_to_string(root.join(name)).expect(name);
        let text = read("Dockerfile");
        let line = |wanted: &str| {
            text.lines()
                .position(|line| line.starts_with(wanted))
                .unwrap_or_else(|| panic!("the Dockerfile has no line {wanted:?}"))
        };
        let build = text
            .lines()
            .position(|line| line.contains("cargo build"))
            .expect("the Dockerfile builds atl");
        assert!(line("COPY skill ./skill") < build);
        for line in read(".dockerignore").lines() {
            let pattern = line.trim().trim_matches('/');
            assert!(
                !matches!(
                    pattern,
                    "skill" | "skill/*" | "skill/**" | "**/*.md" | "**/references" | "*"
                ),
                ".dockerignore leaves the skill out: {line}"
            );
        }
    }

    #[test]
    fn text_finds_a_file_by_its_whole_path() {
        let edition = Edition::embedded();
        assert!(edition.text("SKILL.md").is_some());
        assert!(edition.text("jira-commands.md").is_none());
    }
}
