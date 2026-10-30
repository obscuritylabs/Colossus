use crate::{CreateWorktree, WorktreeError, WorktreeErrorCode, WorktreeSettings, error::Result};

pub(crate) fn name(value: &str) -> Result<()> {
    let reserved = [
        "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
        "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
    ];
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
        || value.starts_with('-')
        || value.ends_with('-')
        || reserved.contains(&value)
    {
        return Err(WorktreeError::new(
            WorktreeErrorCode::InvalidInput,
            "Use a short lowercase worktree name containing letters, numbers and internal hyphens.",
        ));
    }
    Ok(())
}
pub(crate) fn id(value: &str) -> Result<()> {
    if value.len() != 32
        || !value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return Err(WorktreeError::ownership());
    }
    Ok(())
}
pub(crate) fn branch(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 200
        || !value.is_ascii()
        || value.starts_with('-')
        || value.chars().any(char::is_control)
        || !git2::Reference::is_valid_name(&format!("refs/heads/{value}"))
    {
        return Err(WorktreeError::new(
            WorktreeErrorCode::InvalidInput,
            "The branch name is invalid. Use a bounded local Git branch name.",
        ));
    }
    Ok(())
}
pub(crate) fn settings(value: &WorktreeSettings) -> Result<()> {
    if value.branch_prefix.len() > 120
        || (!value.branch_prefix.is_empty() && !value.branch_prefix.ends_with('/'))
    {
        return Err(WorktreeError::new(
            WorktreeErrorCode::InvalidInput,
            "The branch prefix must be empty or end with a slash.",
        ));
    }
    branch(&format!("{}example", value.branch_prefix))
}
pub(crate) fn request(value: &CreateWorktree) -> Result<()> {
    name(&value.name)?;
    if let Some(value) = &value.branch {
        branch(value)?;
    }
    if value.existing_branch && (value.branch.is_none() || value.base.is_some()) {
        return Err(WorktreeError::new(
            WorktreeErrorCode::InvalidInput,
            "An existing branch requires --branch and cannot also select a base.",
        ));
    }
    if let Some(base) = &value.base
        && base != "HEAD"
        && !(base.len() == 40 && base.bytes().all(|c| c.is_ascii_hexdigit()))
    {
        branch(base)?;
    }
    Ok(())
}
