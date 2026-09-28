//! Read-only, selected-workspace Git inspection. No agent or repository-write API.

pub(crate) mod commands;
mod discovery;
mod dto;
mod reader;

#[cfg(test)]
mod tests;

use crate::dto::CommandErrorDto;

fn error(message: &str) -> CommandErrorDto {
    CommandErrorDto::local_sanitized("workspace_git", message, true)
}
