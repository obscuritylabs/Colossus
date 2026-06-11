"""Policy decisions for tools and bundles."""

from typing import Literal

from pydantic import BaseModel, ConfigDict


class PolicyDecision(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    decision: Literal["allow", "deny", "requires_approval"]
    reason: str

    @property
    def allowed_without_approval(self) -> bool:
        return self.decision == "allow"
