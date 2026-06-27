"""Credential resolution ports for integrations."""

from typing import Protocol

from pydantic import BaseModel, ConfigDict


class CredentialMaterial(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    ref: str
    value: str


class CredentialBroker(Protocol):
    def resolve(self, credential_ref: str) -> CredentialMaterial:
        """Resolve a local credential handle into secret material."""
        ...
