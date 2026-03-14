//! Prompt composition and loading system.
//!
//! - [`composer`] — Priority-ordered section composition with conditional loading
//! - [`embedded`] — All `.md` templates embedded at compile time via `include_str!`
//! - [`loader`] — Template file loading with frontmatter stripping

pub mod composer;
pub mod embedded;
pub mod loader;
pub mod reminders;

pub use composer::{
    ConditionFn, PromptComposer, PromptContext, PromptSection, create_composer,
    create_default_composer, create_thinking_composer, strip_frontmatter, substitute_variables,
};
pub use embedded::{TEMPLATE_COUNT, TEMPLATES, get_embedded};
pub use loader::{PromptLoadError, PromptLoader};
pub use reminders::{
    MessageClass, append_directive, append_nudge, get_reminder, inject_system_message,
};
