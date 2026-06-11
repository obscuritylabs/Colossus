"""Domain-level exceptions."""


class ColossusError(Exception):
    """Base error for expected Colossus failures."""


class ProviderError(ColossusError):
    """Raised when a model provider fails."""


class PolicyDeniedError(ColossusError):
    """Raised when policy denies a requested action."""


class ToolExecutionError(ColossusError):
    """Raised when a tool cannot execute successfully."""


class SkillError(ColossusError):
    """Raised when a skill cannot be loaded or validated."""


class BundleVerificationError(ColossusError):
    """Raised when an offline bundle cannot be verified."""
