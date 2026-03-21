//! Lazy per-subdirectory instruction injection.
//!
//! When the agent reads a file, this module checks parent directories for
//! instruction files (AGENTS.md, CLAUDE.md) that haven't been injected yet
//! and returns their content for injection into the conversation.
//!
//! Mirrors OpenCode's `InstructionPrompt.resolve()` behavior.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use tracing::debug;

/// Recognized instruction file names (same order as environment.rs).
const INSTRUCTION_FILENAMES: &[&str] = &["AGENTS.md", "CLAUDE.md", "CONTEXT.md"];

/// Additional instruction files from other AI tools.
const COMPAT_INSTRUCTION_FILES: &[&str] = &[".cursorrules", ".github/copilot-instructions.md"];

/// Maximum instruction file size to inject (50 KB).
const MAX_INSTRUCTION_SIZE: usize = 50 * 1024;

/// Tracks which subdirectory instruction files have been injected into the
/// conversation, and discovers new ones when files are read.
#[derive(Debug, Clone)]
pub struct SubdirInstructionTracker {
    /// Canonical paths of instruction files already injected (at startup or
    /// during the session).
    injected: HashSet<PathBuf>,
    /// The project root (git root or working dir). We don't walk above this.
    project_root: PathBuf,
}

/// An instruction file discovered from a subdirectory.
#[derive(Debug, Clone)]
pub struct SubdirInstruction {
    /// Path to the instruction file.
    pub path: PathBuf,
    /// Relative path from project root for display.
    pub relative_path: String,
    /// File contents.
    pub content: String,
}

impl SubdirInstructionTracker {
    /// Create a new tracker, pre-populating with instruction files already
    /// injected at startup (from the system prompt).
    pub fn new(project_root: PathBuf, startup_files: &[PathBuf]) -> Self {
        let mut injected = HashSet::new();
        for path in startup_files {
            if let Ok(canonical) = path.canonicalize() {
                injected.insert(canonical);
            }
        }
        Self {
            injected,
            project_root,
        }
    }

    /// Check if a file path triggers any new subdirectory instruction injection.
    ///
    /// Walks from the directory containing `file_path` up toward the project root,
    /// looking for AGENTS.md / CLAUDE.md files that haven't been injected yet.
    /// Returns any new instruction files found (and marks them as injected).
    pub fn check_file_read(&mut self, file_path: &Path) -> Vec<SubdirInstruction> {
        let dir = if file_path.is_dir() {
            file_path.to_path_buf()
        } else {
            match file_path.parent() {
                Some(p) => p.to_path_buf(),
                None => return Vec::new(),
            }
        };

        let canonical_root = self
            .project_root
            .canonicalize()
            .unwrap_or_else(|_| self.project_root.clone());
        let mut results = Vec::new();
        let mut current = dir;

        loop {
            // Check each instruction filename in this directory
            for filename in INSTRUCTION_FILENAMES {
                let candidate = current.join(filename);
                if let Ok(canonical) = candidate.canonicalize() {
                    if self.injected.contains(&canonical) {
                        continue; // Already injected
                    }

                    // Read the file
                    if let Ok(content) = std::fs::read_to_string(&canonical) {
                        let content = if content.len() > MAX_INSTRUCTION_SIZE {
                            content[..MAX_INSTRUCTION_SIZE].to_string()
                        } else {
                            content
                        };

                        let relative = canonical
                            .strip_prefix(&canonical_root)
                            .unwrap_or(&canonical)
                            .display()
                            .to_string();

                        debug!(path = %relative, "Injecting subdirectory instruction file");

                        self.injected.insert(canonical.clone());
                        results.push(SubdirInstruction {
                            path: canonical,
                            relative_path: relative,
                            content,
                        });
                    }
                }
            }

            // Also check .opendev/instructions.md
            for subdir in &[".opendev"] {
                let candidate = current.join(subdir).join("instructions.md");
                if let Ok(canonical) = candidate.canonicalize() {
                    if self.injected.contains(&canonical) {
                        continue;
                    }
                    if let Ok(content) = std::fs::read_to_string(&canonical) {
                        let content = if content.len() > MAX_INSTRUCTION_SIZE {
                            content[..MAX_INSTRUCTION_SIZE].to_string()
                        } else {
                            content
                        };
                        let relative = canonical
                            .strip_prefix(&canonical_root)
                            .unwrap_or(&canonical)
                            .display()
                            .to_string();
                        debug!(path = %relative, "Injecting subdirectory instruction file");
                        self.injected.insert(canonical.clone());
                        results.push(SubdirInstruction {
                            path: canonical,
                            relative_path: relative,
                            content,
                        });
                    }
                }
            }

            // Check compatibility instruction files (.cursorrules, copilot, etc.)
            for compat_path in COMPAT_INSTRUCTION_FILES {
                let candidate = current.join(compat_path);
                if let Ok(canonical) = candidate.canonicalize() {
                    if self.injected.contains(&canonical) {
                        continue;
                    }
                    if let Ok(content) = std::fs::read_to_string(&canonical) {
                        let content = if content.len() > MAX_INSTRUCTION_SIZE {
                            content[..MAX_INSTRUCTION_SIZE].to_string()
                        } else {
                            content
                        };
                        let relative = canonical
                            .strip_prefix(&canonical_root)
                            .unwrap_or(&canonical)
                            .display()
                            .to_string();
                        debug!(path = %relative, "Injecting compatibility instruction file");
                        self.injected.insert(canonical.clone());
                        results.push(SubdirInstruction {
                            path: canonical,
                            relative_path: relative,
                            content,
                        });
                    }
                }
            }

            // Stop at project root
            let canonical_current = current.canonicalize().unwrap_or_else(|_| current.clone());
            if canonical_current == canonical_root {
                break;
            }

            // Move up
            if !current.pop() {
                break;
            }
        }

        results
    }

    /// After compaction removes middle messages, allow subdirectory instructions
    /// to be re-discovered on the next file read.
    ///
    /// Preserves startup files (root-level instructions in system prompt) and
    /// any instructions whose content is still present in the remaining messages.
    pub fn reset_after_compaction(
        &mut self,
        startup_files: &[PathBuf],
        remaining_messages: &[serde_json::Value],
    ) {
        // Collect paths of instructions still present in remaining messages
        let mut still_present = HashSet::new();
        for msg in remaining_messages {
            if let Some(content) = msg.get("content").and_then(|v| v.as_str()) {
                for path in &self.injected {
                    let path_str = path.display().to_string();
                    if content.contains(&path_str)
                        || content
                            .contains(path.file_name().unwrap_or_default().to_str().unwrap_or(""))
                    {
                        still_present.insert(path.clone());
                    }
                }
            }
        }

        self.injected = still_present;

        // Always keep startup files marked as injected (they live in system prompt)
        for path in startup_files {
            if let Ok(canonical) = path.canonicalize() {
                self.injected.insert(canonical);
            }
        }
    }

    /// Return the number of instruction files currently tracked.
    pub fn injected_count(&self) -> usize {
        self.injected.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tracker_new_with_startup_files() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().canonicalize().unwrap();

        // Create a startup instruction file
        let agents_md = root.join("AGENTS.md");
        std::fs::write(&agents_md, "# Project rules").unwrap();

        let tracker = SubdirInstructionTracker::new(root.clone(), &[agents_md.clone()]);
        assert_eq!(tracker.injected_count(), 1);
    }

    #[test]
    fn test_check_file_read_finds_subdir_instruction() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().canonicalize().unwrap();

        // Create subdirectory with AGENTS.md
        let subdir = root.join("src").join("payments");
        std::fs::create_dir_all(&subdir).unwrap();
        let agents_md = subdir.join("AGENTS.md");
        std::fs::write(&agents_md, "# Payment rules\nBe careful with money").unwrap();

        // Create a file in that subdirectory
        let file = subdir.join("checkout.rs");
        std::fs::write(&file, "fn checkout() {}").unwrap();

        let mut tracker = SubdirInstructionTracker::new(root.clone(), &[]);

        let results = tracker.check_file_read(&file);
        assert_eq!(results.len(), 1);
        assert!(results[0].content.contains("Payment rules"));
        assert!(results[0].relative_path.contains("AGENTS.md"));
    }

    #[test]
    fn test_check_file_read_deduplicates() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().canonicalize().unwrap();

        let subdir = root.join("src");
        std::fs::create_dir_all(&subdir).unwrap();
        std::fs::write(subdir.join("AGENTS.md"), "rules").unwrap();
        std::fs::write(subdir.join("a.rs"), "").unwrap();
        std::fs::write(subdir.join("b.rs"), "").unwrap();

        let mut tracker = SubdirInstructionTracker::new(root.clone(), &[]);

        // First read finds the instruction
        let r1 = tracker.check_file_read(&subdir.join("a.rs"));
        assert_eq!(r1.len(), 1);

        // Second read in same dir should not re-inject
        let r2 = tracker.check_file_read(&subdir.join("b.rs"));
        assert_eq!(r2.len(), 0);
    }

    #[test]
    fn test_check_file_read_skips_startup_files() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().canonicalize().unwrap();

        // Create root-level AGENTS.md (already injected at startup)
        let agents_md = root.join("AGENTS.md");
        std::fs::write(&agents_md, "root rules").unwrap();

        let file = root.join("main.rs");
        std::fs::write(&file, "fn main() {}").unwrap();

        let mut tracker = SubdirInstructionTracker::new(root.clone(), &[agents_md]);

        // Should not find anything — root AGENTS.md was already injected
        let results = tracker.check_file_read(&file);
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn test_walks_up_to_root() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().canonicalize().unwrap();

        // Create instruction files at different levels
        let deep = root.join("a").join("b").join("c");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(root.join("a").join("CLAUDE.md"), "level a").unwrap();
        std::fs::write(deep.join("AGENTS.md"), "level c").unwrap();

        let file = deep.join("file.rs");
        std::fs::write(&file, "").unwrap();

        let mut tracker = SubdirInstructionTracker::new(root.clone(), &[]);

        let results = tracker.check_file_read(&file);
        // Should find both: a/CLAUDE.md and a/b/c/AGENTS.md
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_context_md_recognized() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().canonicalize().unwrap();

        let subdir = root.join("lib");
        std::fs::create_dir_all(&subdir).unwrap();
        std::fs::write(subdir.join("CONTEXT.md"), "deprecated but supported").unwrap();

        let file = subdir.join("util.rs");
        std::fs::write(&file, "").unwrap();

        let mut tracker = SubdirInstructionTracker::new(root.clone(), &[]);

        let results = tracker.check_file_read(&file);
        assert_eq!(results.len(), 1);
        assert!(results[0].content.contains("deprecated but supported"));
    }

    #[test]
    fn test_cursorrules_discovered() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().canonicalize().unwrap();

        // Create a .cursorrules file at project root
        std::fs::write(
            root.join(".cursorrules"),
            "Always use TypeScript strict mode",
        )
        .unwrap();

        let file = root.join("index.ts");
        std::fs::write(&file, "").unwrap();

        let mut tracker = SubdirInstructionTracker::new(root.clone(), &[]);

        let results = tracker.check_file_read(&file);
        assert_eq!(results.len(), 1);
        assert!(results[0].content.contains("TypeScript strict mode"));
    }

    #[test]
    fn test_reset_after_compaction_clears_subdirectory_instructions() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().canonicalize().unwrap();

        let subdir = root.join("src").join("payments");
        std::fs::create_dir_all(&subdir).unwrap();
        std::fs::write(subdir.join("AGENTS.md"), "Payment rules").unwrap();
        std::fs::write(subdir.join("checkout.rs"), "fn checkout() {}").unwrap();

        let mut tracker = SubdirInstructionTracker::new(root.clone(), &[]);

        // Inject instruction
        let results = tracker.check_file_read(&subdir.join("checkout.rs"));
        assert_eq!(results.len(), 1);

        // Simulate compaction removing all messages
        tracker.reset_after_compaction(&[], &[]);

        // Should be able to re-inject
        let results2 = tracker.check_file_read(&subdir.join("checkout.rs"));
        assert_eq!(results2.len(), 1);
    }

    #[test]
    fn test_reset_preserves_startup_files() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().canonicalize().unwrap();

        let agents_md = root.join("AGENTS.md");
        std::fs::write(&agents_md, "root rules").unwrap();
        std::fs::write(root.join("main.rs"), "fn main() {}").unwrap();

        let startup = vec![agents_md.clone()];
        let mut tracker = SubdirInstructionTracker::new(root.clone(), &startup);
        assert_eq!(tracker.injected_count(), 1);

        // Reset should preserve startup files
        tracker.reset_after_compaction(&startup, &[]);
        assert_eq!(tracker.injected_count(), 1);

        // Root AGENTS.md should still not be re-injected
        let results = tracker.check_file_read(&root.join("main.rs"));
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn test_reset_preserves_instructions_still_in_messages() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().canonicalize().unwrap();

        let subdir = root.join("src");
        std::fs::create_dir_all(&subdir).unwrap();
        std::fs::write(subdir.join("AGENTS.md"), "src rules").unwrap();
        std::fs::write(subdir.join("lib.rs"), "").unwrap();

        let mut tracker = SubdirInstructionTracker::new(root.clone(), &[]);

        // Inject
        let results = tracker.check_file_read(&subdir.join("lib.rs"));
        assert_eq!(results.len(), 1);

        // Simulate compaction that keeps the instruction in remaining messages
        let remaining = vec![serde_json::json!({
            "role": "user",
            "content": format!("Instructions from AGENTS.md in {}", subdir.display()),
        })];
        tracker.reset_after_compaction(&[], &remaining);

        // Should NOT re-inject since it's still in messages
        let results2 = tracker.check_file_read(&subdir.join("lib.rs"));
        assert_eq!(results2.len(), 0);
    }

    #[test]
    fn test_reinjection_after_reset() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().canonicalize().unwrap();

        let sub_a = root.join("a");
        let sub_b = root.join("b");
        std::fs::create_dir_all(&sub_a).unwrap();
        std::fs::create_dir_all(&sub_b).unwrap();
        std::fs::write(sub_a.join("AGENTS.md"), "rules a").unwrap();
        std::fs::write(sub_b.join("AGENTS.md"), "rules b").unwrap();
        std::fs::write(sub_a.join("f.rs"), "").unwrap();
        std::fs::write(sub_b.join("g.rs"), "").unwrap();

        let mut tracker = SubdirInstructionTracker::new(root.clone(), &[]);

        // Inject both
        assert_eq!(tracker.check_file_read(&sub_a.join("f.rs")).len(), 1);
        assert_eq!(tracker.check_file_read(&sub_b.join("g.rs")).len(), 1);

        // Reset with empty messages — both cleared
        tracker.reset_after_compaction(&[], &[]);

        // Both should re-inject
        assert_eq!(tracker.check_file_read(&sub_a.join("f.rs")).len(), 1);
        assert_eq!(tracker.check_file_read(&sub_b.join("g.rs")).len(), 1);
    }

    #[test]
    fn test_copilot_instructions_discovered() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().canonicalize().unwrap();

        // Create .github/copilot-instructions.md
        let github_dir = root.join(".github");
        std::fs::create_dir_all(&github_dir).unwrap();
        std::fs::write(
            github_dir.join("copilot-instructions.md"),
            "Use conventional commits",
        )
        .unwrap();

        let file = root.join("main.rs");
        std::fs::write(&file, "").unwrap();

        let mut tracker = SubdirInstructionTracker::new(root.clone(), &[]);

        let results = tracker.check_file_read(&file);
        assert_eq!(results.len(), 1);
        assert!(results[0].content.contains("conventional commits"));
    }
}
